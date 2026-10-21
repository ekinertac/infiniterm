//! The editor's read-only DIFF mode: what a diff card is (2026-10-20, Ekin:
//! "why diff cards using a different base. let's just make them read only
//! editors. that way they inherit all the editor features we already solved").
//!
//! A child of `editor_body` so it can reach the body's private state; the
//! pure part is `infiniterm-editor::diff` (`merged`: HEAD and the working
//! tree woven into one text with a kind and old/new numbers per line, folds
//! over the unchanged stretches, `merged_spans`: syntax colours per side).
//! In this mode the buffer holds that merged text and is read-only, the tree
//! is a FLAT list of the changed files (`Tree::flat`, with `+added -removed`
//! beside each row), and the painter adds three things to the ordinary
//! editor's: a wash over added and removed lines, two line-number columns
//! (HEAD's and the working tree's) with a `+`/`-` sign in place of the git
//! gutter, and optionally blame.
//!
//! What it must NOT do is run the file machinery of an editor: no reading
//! `path` from disk into the buffer, no draft, no git gutter against HEAD,
//! no save. Each of those is guarded in `editor_body.rs` by `is_diff`.
//!
//! Called by `editor_body.rs` (the guards, `gutter_w`, the painters) and by
//! `editors.rs` (`diff_start`, the Cmd+K and blame effects). The card is
//! hosted by `EditorTabs` with no strip; `Card.kind` stays `Diff`.
use super::*;
use infiniterm_core::blame::{blame_text, BlameLine};
use infiniterm_core::git::{git_blame, git_changes, ChangedFile};
use infiniterm_editor::diff::{merged, merged_spans, DiffLine, LineKind};

/// An added/removed line's background wash: a hint of colour, not a block.
const DIFF_WASH_ALPHA: f32 = 0.18;
/// The change bar sits this far left of the text, clear of the gutter.
const DIFF_BAR_OFFSET_PX: f64 = 6.;
/// The change bar's width.
const DIFF_BAR_WIDTH_PX: f64 = 3.;
/// Room for the `+`/`-` sign after the new line number, in px beyond a cell.
const DIFF_SIGN_EXTRA_PX: f64 = 4.;
/// The blame column's width in cells: a short hash, a first name, an age.
const BLAME_GUTTER_COLS: f64 = 22.;
/// The gap between the blame text and the line number that follows it.
const BLAME_GUTTER_GAP_COLS: f64 = 2.;
/// An uncommitted blame line's colour already marks it; a committed one is
/// dimmed so the code, not the attribution, reads first.
const BLAME_DIM_ALPHA: f32 = 0.85;
/// Gap between a list row's file name and its `+added -removed` counts.
const LIST_NAME_COUNT_GAP_PX: f32 = 8.;

/// The diff mode's state; `EditorBody::diff` is `Some` for a diff card only.
pub(super) struct DiffView {
    /// What `git_changes` is scoped to: a repository, a folder or a file.
    root: String,
    /// The repository's root, absolute; empty until git has answered.
    repo: String,
    files: Vec<ChangedFile>,
    /// The file on show, relative to the repo.
    shown: Option<String>,
    /// One per line of the buffer (the merged text).
    lines: Vec<DiffLine>,
    /// Syntax spans of the merged text, per side (`merged_spans`).
    spans: Vec<Span>,
    /// Digits the two number columns need, at least three like the editor's.
    old_digits: usize,
    new_digits: usize,
    blame_on: bool,
    blame: Vec<BlameLine>,
    /// The shown file's modification time at the last load, for the poll.
    stamp: Option<u64>,
    last_check: f64,
}

impl DiffView {
    fn new(root: &str) -> DiffView {
        DiffView {
            root: root.to_string(),
            repo: String::new(),
            files: vec![],
            shown: None,
            lines: vec![],
            spans: vec![],
            old_digits: MIN_GUTTER_DIGITS,
            new_digits: MIN_GUTTER_DIGITS,
            blame_on: false,
            blame: vec![],
            stamp: None,
            last_check: 0.,
        }
    }

    pub(super) fn spans(&self) -> &[Span] {
        &self.spans
    }

    fn blame_w(&self, cell_w: f64) -> f64 {
        if self.blame_on {
            (BLAME_GUTTER_COLS + BLAME_GUTTER_GAP_COLS) * cell_w
        } else {
            0.
        }
    }

    /// The width of the gutter in world px: blame, HEAD's numbers, the new
    /// numbers, the sign, and the editor's usual padding.
    pub(super) fn gutter_w(&self, cell_w: f64) -> f64 {
        self.blame_w(cell_w)
            + (self.old_digits + 1 + self.new_digits) as f64 * cell_w
            + cell_w
            + DIFF_SIGN_EXTRA_PX
            + GUTTER_EXTRA_PAD_PX
    }

    fn full(&self, rel: &str) -> String {
        format!("{}/{rel}", self.repo.trim_end_matches(['/', '\\']))
    }

    /// The list's rows: flat, the name is the path in the repo.
    fn entries(&self) -> Vec<Entry> {
        self.files
            .iter()
            .map(|f| Entry {
                name: f.path.clone(),
                path: self.full(&f.path),
                is_dir: false,
            })
            .collect()
    }
}

/// `path` below `repo`, as git spells it (`/` between names), or `None` when
/// it is not below it.
///
/// Both sides are compared with either separator: git answers the repo root
/// as `C:/Users/x/repo` on Windows while a card's paths are `C:\Users\x\repo\...`,
/// and a plain `starts_with` called them different places.
fn relative_to(repo: &str, path: &str) -> Option<String> {
    let repo = repo.replace('\\', "/");
    let path = path.replace('\\', "/");
    let rest = path.strip_prefix(repo.trim_end_matches('/'))?;
    let rest = rest.strip_prefix('/')?;
    (!rest.is_empty()).then(|| rest.to_string())
}

impl EditorBody {
    /// A diff card's body: an empty read-only editor that `diff_start` fills.
    pub fn new_diff(card_id: &str, root: &str, metrics: &Metrics, world: Size) -> EditorBody {
        let mut body = EditorBody::new(card_id, None, root.to_string(), metrics, world);
        body.read_only = true;
        body.diff = Some(DiffView::new(root));
        body
    }

    pub fn is_diff(&self) -> bool {
        self.diff.is_some()
    }

    /// The first state of a new diff card: the list read, and either the
    /// file asked for on show (with the list as the card says) or, with none,
    /// the list alone and focused.
    pub fn diff_start(&mut self, target: Option<&str>, list_shown: bool) {
        self.diff_refresh();
        let Some(d) = self.diff.as_ref() else { return };
        let repo = d.repo.clone();
        let file = target.filter(|t| {
            !repo.is_empty() && relative_to(&repo, t).is_some() && std::path::Path::new(t).is_file()
        });
        match file {
            Some(t) => {
                let rel = relative_to(&repo, t).unwrap_or_default();
                self.diff_show(&rel);
                if list_shown {
                    self.diff_show_list(false);
                }
            }
            None => self.diff_show_list(true),
        }
    }

    /// Shows the list (read again) with or without the keys.
    fn diff_show_list(&mut self, focused: bool) {
        self.diff_refresh();
        if self.tree.is_none() {
            let Some(d) = self.diff.as_ref() else { return };
            self.tree = Some(Tree::flat(&d.repo, d.entries()));
        }
        self.tree_focused = focused;
        self.focus = if focused { Focus::Tree } else { Focus::Buffer };
        self.dirty = true;
    }

    /// Reads the changed files again: Cmd+K, a change of the shown file, and
    /// once at the start. Updates the flat tree when it is on show.
    pub fn diff_refresh(&mut self) {
        let Some(d) = self.diff.as_mut() else { return };
        match git_changes(&d.root) {
            Ok(changes) => {
                d.repo = changes.repo;
                d.files = changes.files;
            }
            Err(e) => {
                eprintln!("[infiniterm/warn] git: {e}");
                self.events.push(EditorEvent::Notice(format!(
                    "not a git repository: {}",
                    infiniterm_core::paths::base_name(&d.root)
                )));
                d.files.clear();
            }
        }
        let entries = d.entries();
        if let Some(t) = self.tree.as_mut() {
            t.set_flat(entries);
        }
        self.dirty = true;
    }

    /// Shows one file's diff: HEAD's text as the original, the disk's as the
    /// current (a deleted file is empty now, a new one empty then). A
    /// binary file reads as empty on the disk side and as git printed it on
    /// HEAD's, as the old card did.
    pub fn diff_show(&mut self, rel: &str) {
        let Some(d) = self.diff.as_mut() else { return };
        if d.repo.is_empty() {
            return;
        }
        let full = d.full(rel);
        let original = git_show_head(&d.repo, rel).unwrap_or_default();
        let current = file_read(&full).unwrap_or_default();
        d.stamp = file_mtime(&full);
        let m = merged(&original, &current);
        let language = Language::for_path(&full);
        // Each side highlighted by itself: see `merged_spans`.
        d.spans = match language {
            Some(l) => {
                let old = self.highlighting.spans(l, &original);
                let new = self.highlighting.spans(l, &current);
                merged_spans(&m, &old, &new)
            }
            None => vec![],
        };
        let digits = |side: fn(&DiffLine) -> Option<usize>| {
            m.lines
                .iter()
                .filter_map(side)
                .max()
                .unwrap_or(1)
                .to_string()
                .len()
                .max(MIN_GUTTER_DIGITS)
        };
        d.old_digits = digits(|l| l.old);
        d.new_digits = digits(|l| l.new);
        d.shown = Some(rel.to_string());
        let blame_on = d.blame_on;
        self.buffer = Buffer::new(&m.text);
        self.buffer.set_fixed_folds(&m.folds);
        // The caret starts on the first line you can see and that is code,
        // not on a folded stretch's line or its "N unchanged lines".
        let first = (0..m.lines.len())
            .find(|&i| m.lines[i].kind != LineKind::Placeholder && !self.buffer.is_line_hidden(i))
            .unwrap_or(0);
        d.lines = m.lines;
        let at = self.buffer.line_start(first);
        self.buffer.set_cursor(at);
        self.saved = m.text;
        self.path = Some(full.clone());
        self.cwd = parent_of(&full);
        self.language = language;
        self.scroll_line = 0;
        self.scroll_x = 0.;
        self.shaped.clear();
        self.loads += 1;
        eprintln!(
            "[infiniterm] diff {rel}: {} lines, original {}b, current {}b",
            self.buffer.line_count(),
            original.len(),
            current.len()
        );
        if blame_on {
            self.diff_load_blame();
        }
        self.events.push(EditorEvent::PathChanged {
            path: full.clone(),
            cwd: parent_of(&full),
        });
        self.dirty = true;
    }

    fn diff_load_blame(&mut self) {
        let Some(d) = self.diff.as_mut() else { return };
        let Some(rel) = d.shown.clone() else { return };
        // git.rs parses what git said; blame.rs formats it and takes its
        // own record shape (the reference had the same two).
        d.blame = git_blame(&d.repo, &rel)
            .unwrap_or_else(|e| {
                eprintln!("[infiniterm/warn] blame: {e}");
                vec![]
            })
            .into_iter()
            .map(|b| BlameLine {
                line: b.line as usize,
                hash: b.hash,
                author: b.author,
                time: b.time as f64,
                summary: b.summary,
            })
            .collect();
        self.dirty = true;
    }

    /// Cmd+B: the blame gutter, for the lines the working tree has.
    pub fn diff_toggle_blame(&mut self) {
        let Some(d) = self.diff.as_mut() else { return };
        d.blame_on = !d.blame_on;
        if d.blame_on {
            self.diff_load_blame();
        } else if let Some(d) = self.diff.as_mut() {
            d.blame.clear();
        }
        self.dirty = true;
    }

    /// Cmd+K's three states for the list: hidden -> shown and focused ->
    /// (from the text) focused -> (from the list) hidden. True when it shows.
    pub fn diff_toggle_list(&mut self) -> bool {
        let shown = if self.tree.is_none() || !self.tree_focused {
            self.diff_show_list(true);
            true
        } else {
            self.tree = None;
            self.tree_focused = false;
            self.focus = Focus::Buffer;
            false
        };
        self.dirty = true;
        shown
    }

    /// Enter, Right or a click on a row of the list: that file's diff goes on
    /// show and the text has the keys. The file already on show is not
    /// loaded again, so the cursor and the scroll stay: Right from the list
    /// is "back to where I was reading".
    pub(super) fn diff_open(&mut self, path: &str) -> BodyAction {
        let Some(d) = self.diff.as_ref() else {
            return BodyAction::None;
        };
        let rel = relative_to(&d.repo, path).unwrap_or_else(|| path.to_string());
        if d.shown.as_deref() != Some(rel.as_str()) {
            self.diff_show(&rel);
        }
        self.tree_focused = false;
        self.focus = Focus::Buffer;
        self.dirty = true;
        BodyAction::None
    }

    /// The disk poll of a diff: the shown file changed under it, so the
    /// list and the diff are read again (the old card did the same).
    pub(super) fn diff_idle(&mut self, now: f64) {
        let Some(d) = self.diff.as_mut() else { return };
        if now - d.last_check < DISK_POLL_MS {
            return;
        }
        d.last_check = now;
        let Some(rel) = d.shown.clone() else { return };
        let stamp = file_mtime(&d.full(&rel));
        if stamp == d.stamp {
            return;
        }
        d.stamp = stamp;
        self.diff_refresh();
        self.diff_show(&rel);
    }

    /// The line number the status bar says for the caret's line: the file's
    /// (the working tree's), or HEAD's for a removed line.
    pub(super) fn diff_status_line(&self) -> Option<usize> {
        let d = self.diff.as_ref()?;
        let line = d.lines.get(self.buffer.line_of(self.buffer.cursor()))?;
        line.new.or(line.old)
    }

    /// Whether the caret is on an "N unchanged lines" line: Enter opens it.
    pub(super) fn diff_on_placeholder(&self) -> bool {
        self.diff.as_ref().is_some_and(|d| {
            d.lines
                .get(self.buffer.line_of(self.buffer.cursor()))
                .is_some_and(|l| l.kind == LineKind::Placeholder)
        })
    }

    /// Left at the first column of a line, with nothing selected, goes to the
    /// list when it shows: Left comes back to it as Right/Enter goes in. A
    /// Left anywhere else is the caret's.
    pub(super) fn diff_left_to_list(&mut self, shift: bool) -> bool {
        if self.diff.is_none()
            || self.tree.is_none()
            || shift
            || self.buffer.selection().is_some()
            || self.buffer.cursor_count() > 1
            || self.buffer.col_of(self.buffer.cursor()) != 0
        {
            return false;
        }
        self.tree_focused = true;
        self.focus = Focus::Tree;
        self.dirty = true;
        true
    }

    /// The selection as text, for copy: a diff leaves its "N unchanged
    /// lines" lines out, they are not in any file.
    pub(super) fn selected_for_copy(&self) -> Option<String> {
        let text = self.buffer.selected_text()?;
        let Some(d) = self.diff.as_ref() else {
            return Some(text);
        };
        let first = self.buffer.line_of(self.buffer.selection()?.start);
        let kept: Vec<&str> = text
            .split('\n')
            .enumerate()
            .filter(|(i, _)| {
                d.lines
                    .get(first + i)
                    .is_none_or(|l| l.kind != LineKind::Placeholder)
            })
            .map(|(_, t)| t)
            .collect();
        (!kept.is_empty()).then(|| kept.join("\n"))
    }

    /// The theme's green and red, for the washes and the counts.
    fn diff_green(&self) -> Hsla {
        hex(self
            .rules
            .iter()
            .find(|r| r.tag == "string")
            .map(|r| r.color.as_str())
            .unwrap_or("#a6e3a1"))
    }

    fn diff_red(&self) -> Hsla {
        hex(self
            .rules
            .iter()
            .find(|r| r.tag == "operator")
            .map(|r| r.color.as_str())
            .unwrap_or("#f38ba8"))
    }

    /// The number columns, the sign, the change bar and blame of one line, in
    /// the gutter and outside the text's clip. `gutter_w` is the gutter's
    /// width on screen; `origin` the text area's top-left plus its padding.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_diff_gutter(
        &self,
        line_no: usize,
        origin: gpui::Point<Pixels>,
        y: Pixels,
        gutter_w: Pixels,
        scale: f64,
        legible: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(d) = self.diff.as_ref() else { return };
        let Some(line) = d.lines.get(line_no) else {
            return;
        };
        let s = |v: f64| px((v * scale) as f32);
        let font_size = px((self.metrics.font_px * scale) as f32);
        let line_h = px((self.line_h() * scale) as f32);
        let base = self.metrics.font();
        let gutter_fg = hex(&self.colors.gutter);
        let color = match line.kind {
            LineKind::Added => Some(self.diff_green()),
            LineKind::Removed => Some(self.diff_red()),
            _ => None,
        };
        if let Some(color) = color {
            window.paint_quad(fill(
                Bounds::new(
                    point(origin.x + gutter_w - s(DIFF_BAR_OFFSET_PX), y),
                    size(s(DIFF_BAR_WIDTH_PX), line_h),
                ),
                color,
            ));
        }
        if !legible || line.kind == LineKind::Placeholder {
            return;
        }
        let cell_w = self.metrics.cell_w;
        let old_right = d.blame_w(cell_w) + d.old_digits as f64 * cell_w;
        let new_right = old_right + cell_w + d.new_digits as f64 * cell_w;
        for (n, right) in [(line.old, old_right), (line.new, new_right)] {
            if let Some(n) = n {
                let l = crate::text::shape(window, &n.to_string(), font_size, &base, gutter_fg);
                let _ = l.paint(point(origin.x + s(right) - l.width, y), line_h, window, cx);
            }
        }
        if let Some(color) = color {
            let sign = if line.kind == LineKind::Added {
                "+"
            } else {
                "-"
            };
            let l = crate::text::shape(window, sign, font_size, &base, color);
            let _ = l.paint(
                point(origin.x + s(new_right + DIFF_SIGN_EXTRA_PX), y),
                line_h,
                window,
                cx,
            );
        }
        if d.blame_on {
            if let Some(b) = line.new.and_then(|n| d.blame.iter().find(|b| b.line == n)) {
                let uncommitted = b.hash == "0000000";
                let l = crate::text::shape(
                    window,
                    &blame_text(b, crate::now_ms() / 1000.),
                    font_size,
                    &base,
                    if uncommitted {
                        self.diff_green()
                    } else {
                        crate::chrome::with_alpha(gutter_fg, BLAME_DIM_ALPHA)
                    },
                );
                let _ = l.paint(point(origin.x, y), line_h, window, cx);
            }
        }
    }

    /// Behind a line's text: the wash of an added or removed line, or the
    /// hairlines above and below an "N unchanged lines" line. `x` is where
    /// the text area starts, `width` how far the row reaches.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_diff_back(
        &self,
        line_no: usize,
        x: Pixels,
        y: Pixels,
        width: Pixels,
        line_h: Pixels,
        window: &mut Window,
    ) {
        let Some(line) = self.diff.as_ref().and_then(|d| d.lines.get(line_no)) else {
            return;
        };
        let row = Bounds::new(point(x, y), size(width, line_h));
        match line.kind {
            LineKind::Added => window.paint_quad(fill(
                row,
                crate::chrome::with_alpha(self.diff_green(), DIFF_WASH_ALPHA),
            )),
            LineKind::Removed => window.paint_quad(fill(
                row,
                crate::chrome::with_alpha(self.diff_red(), DIFF_WASH_ALPHA),
            )),
            LineKind::Placeholder => {
                let hairline = px(crate::chrome::HAIRLINE_PX as f32);
                for dy in [px(0.), line_h - hairline] {
                    window.paint_quad(fill(
                        Bounds::new(point(x, y + dy), size(width, hairline)),
                        crate::chrome::with_alpha(
                            hex(&self.colors.gutter),
                            crate::chrome::HAIRLINE_ALPHA,
                        ),
                    ));
                }
            }
            LineKind::Context => {}
        }
    }

    /// Whether a line is an "N unchanged lines" line, which is drawn in the
    /// gutter's colour instead of syntax colours.
    pub(super) fn diff_is_placeholder(&self, line_no: usize) -> bool {
        self.diff
            .as_ref()
            .and_then(|d| d.lines.get(line_no))
            .is_some_and(|l| l.kind == LineKind::Placeholder)
    }

    /// The list's heading, one row: how many files changed and the totals.
    pub(super) fn diff_list_heading(&self) -> Option<String> {
        let d = self.diff.as_ref()?;
        if d.files.is_empty() {
            return Some("no changes".into());
        }
        let (added, removed) = d
            .files
            .iter()
            .fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
        Some(format!("{} changed  +{added} −{removed}", d.files.len()))
    }

    /// One row of the list: the file, as many parents as fit before the
    /// counts and then cut from the FRONT so the extension stays (the end of
    /// a name tells files apart), with `+added -removed` at the right edge in
    /// the status colour. `color` is the row's text colour.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_diff_list_row(
        &self,
        index: usize,
        area: Bounds<Pixels>,
        y: Pixels,
        pad: Pixels,
        line_h: Pixels,
        font_size: Pixels,
        color: Hsla,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(file) = self.diff.as_ref().and_then(|d| d.files.get(index)) else {
            return;
        };
        let f = self.metrics.font();
        let counts = format!("+{} −{}", file.added, file.removed);
        let c = crate::text::shape(
            window,
            &counts,
            font_size,
            &f,
            if file.status == "deleted" {
                self.diff_red()
            } else {
                self.diff_green()
            },
        );
        let avail = area.size.width - pad * 2. - c.width - px(LIST_NAME_COUNT_GAP_PX);
        let mut shown = file.path.clone();
        let mut name = crate::text::shape(window, &shown, font_size, &f, color);
        while name.width > avail && shown.contains('/') {
            let rest = shown.trim_start_matches("…/");
            shown = match rest.split_once('/') {
                Some((_, tail)) => format!("…/{tail}"),
                None => break,
            };
            name = crate::text::shape(window, &shown, font_size, &f, color);
        }
        // A name with no folder left, or `…/name` still too wide, would run
        // over the diff pane (#372): cut it from the front.
        if name.width > avail {
            let room = f32::from(avail.max(px(0.)));
            shown = crate::text::elide_start(&shown, room, |t| {
                f32::from(crate::text::shape(window, t, font_size, &f, color).width)
            });
            name = crate::text::shape(window, &shown, font_size, &f, color);
        }
        let _ = name.paint(point(area.origin.x + pad, y), line_h, window, cx);
        let _ = c.paint(
            point(area.origin.x + area.size.width - pad - c.width, y),
            line_h,
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_body::Metrics;

    fn metrics() -> Metrics {
        Metrics {
            family: "Menlo".into(),
            font_px: 13.,
            line_height: 1.4,
            cell_w: 8.,
            weight: gpui::FontWeight::NORMAL,
            bold_weight: gpui::FontWeight::BOLD,
        }
    }

    /// The scratch repo's folder, removed when the test ends.
    struct Scratch(std::path::PathBuf);
    impl std::ops::Deref for Scratch {
        type Target = std::path::PathBuf;
        fn deref(&self) -> &std::path::PathBuf {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A scratch git repo with `a.rs` committed and then changed, and `b.txt`
    /// untracked: a modification and a new file.
    fn repo() -> (Scratch, EditorBody) {
        // A counter, not the clock: the tests run in parallel, and two of
        // them got the same microsecond (and the same folder).
        static SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ift-diffmode-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q"]);
        let before: String = (1..=20).map(|i| format!("let v{i} = {i};\n")).collect();
        std::fs::write(dir.join("a.rs"), &before).unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "first"]);
        let after = before.replace("let v10 = 10;\n", "let v10 = 100;\nlet extra = 1;\n");
        std::fs::write(dir.join("a.rs"), after).unwrap();
        std::fs::write(dir.join("b.txt"), "new\nfile\n").unwrap();
        // Not `canonicalize`: on Windows that returns a `\\?\` path, in which
        // a `/` is not a separator and the tests below join with one.
        let root = infiniterm_core::paths::canonical(&dir).unwrap();
        let body = EditorBody::new_diff(
            "c1",
            &root.to_string_lossy(),
            &metrics(),
            Size { w: 800., h: 400. },
        );
        (Scratch(root), body)
    }

    // The whole life of a diff body without a window: the list is the changed
    // files, a file's diff is the merged text (read-only, never dirty, with
    // the unchanged stretches folded), and Left/Right/Enter move between.
    #[test]
    fn a_diff_body_lists_the_changes_and_shows_a_file_as_one_read_only_text() {
        let (root, mut b) = repo();
        let a_rs = format!("{}/a.rs", root.display());
        b.diff_start(Some(&a_rs), true);
        // The list is flat and holds both files, a.rs first.
        let rows = b.tree.as_ref().unwrap().rows();
        let names: Vec<_> = rows.iter().map(|r| r.entry.name.as_str()).collect();
        assert_eq!(names, vec!["a.rs", "b.txt"]);
        // a.rs is on show: one removed line, two added, folds above and below.
        let d = b.diff.as_ref().unwrap();
        assert_eq!(d.shown.as_deref(), Some("a.rs"));
        let kinds: Vec<_> = d.lines.iter().map(|l| l.kind).collect();
        assert_eq!(kinds.iter().filter(|k| **k == LineKind::Removed).count(), 1);
        assert_eq!(kinds.iter().filter(|k| **k == LineKind::Added).count(), 2);
        assert_eq!(
            kinds
                .iter()
                .filter(|k| **k == LineKind::Placeholder)
                .count(),
            2
        );
        assert!(b.read_only && !b.is_dirty(), "a diff is never unsaved");
        assert_eq!(b.path.as_deref(), Some(a_rs.as_str()));
        assert!(b.buffer.has_folds(), "the unchanged stretches start folded");
        assert!(b.buffer.text().contains("let v10 = 10;"), "removed text");
        assert!(!b.buffer.text().contains("+let"), "no diff prefixes");
        // The text is Rust, so it has syntax spans, and the spans are inside it.
        assert!(!d.spans().is_empty());
        let len = b.buffer.text().len();
        assert!(d.spans().iter().all(|s| s.start < s.end && s.end <= len));
        // The caret starts on the first real line, not on a placeholder.
        assert!(!b.diff_on_placeholder());
        // The list was asked to show, and the text has the keys.
        assert!(b.tree.is_some() && !b.tree_focused);
    }

    #[test]
    fn left_at_the_first_column_goes_to_the_list_and_enter_comes_back() {
        let (root, mut b) = repo();
        let a_rs = format!("{}/a.rs", root.display());
        b.diff_start(Some(&a_rs), true);
        let line_before = b.buffer.line_of(b.buffer.cursor());
        // Mid-line, Left is the caret's.
        let at = b.buffer.cursor() + 3;
        b.buffer.set_cursor(at);
        assert!(!b.diff_left_to_list(false));
        // At column 0 it goes to the list; with a selection or Shift it does not.
        b.buffer.set_cursor(b.buffer.line_start(line_before));
        assert!(!b.diff_left_to_list(true), "shift selects");
        assert!(b.diff_left_to_list(false));
        assert!(b.tree_focused && b.focus == Focus::Tree);
        // Opening the file on show keeps the place.
        let moved = b.buffer.line_start(line_before + 2);
        b.buffer.set_cursor(moved);
        b.diff_open(&a_rs);
        assert!(!b.tree_focused && b.focus == Focus::Buffer);
        assert_eq!(b.buffer.cursor(), moved, "same file: not reloaded");
        // Another file replaces the text.
        let b_txt = format!("{}/b.txt", root.display());
        b.diff_open(&b_txt);
        assert_eq!(b.diff.as_ref().unwrap().shown.as_deref(), Some("b.txt"));
        assert_eq!(b.buffer.text(), "new\nfile");
        assert!(b
            .diff
            .as_ref()
            .unwrap()
            .lines
            .iter()
            .all(|l| l.kind == LineKind::Added));
        // With the list hidden, Left stays the caret's.
        b.diff_toggle_list(); // focused
        b.diff_toggle_list(); // hidden
        assert!(b.tree.is_none());
        b.buffer.set_cursor(0);
        assert!(!b.diff_left_to_list(false));
    }

    fn key(name: &str) -> Keystroke {
        Keystroke::parse(name).unwrap()
    }

    // A diff opened on a folder starts on the list with the keys; Down
    // chooses, Right (or Enter) opens the file and moves into its diff,
    // Escape leaves the list for the text. The list never lists the disk.
    #[test]
    fn the_list_has_the_keys_when_there_is_no_file_and_right_opens_a_row() {
        let (_root, mut b) = repo();
        b.diff_start(None, false);
        assert!(b.tree.is_some() && b.tree_focused && b.focus == Focus::Tree);
        assert!(b.diff.as_ref().unwrap().shown.is_none());
        b.key_tree(&key("down"), 0.);
        assert_eq!(b.tree.as_ref().unwrap().cursor, 1);
        b.key_tree(&key("up"), 0.);
        // Right goes into the file under the cursor.
        b.key_tree(&key("right"), 0.);
        assert_eq!(b.diff.as_ref().unwrap().shown.as_deref(), Some("a.rs"));
        assert!(!b.tree_focused && b.focus == Focus::Buffer);
        // Back to the list with Cmd+Alt+Left (the editor's own move), and
        // Escape from the list hands the keys back to the text.
        assert!(b.move_focus_within(infiniterm_core::navigate::Direction::Left));
        assert!(b.tree_focused);
        b.key_tree(&key("escape"), 0.);
        assert!(!b.tree_focused && b.focus == Focus::Buffer);
        // Enter on a row opens it too.
        b.tree_focused = true;
        b.focus = Focus::Tree;
        b.key_tree(&key("down"), 0.);
        b.key_tree(&key("enter"), 0.);
        assert_eq!(b.diff.as_ref().unwrap().shown.as_deref(), Some("b.txt"));
        // A click on a row is the same as Enter, a click on the heading is not.
        b.tree_focused = true;
        b.focus = Focus::Tree;
        b.sidebar_w = 200.;
        assert_eq!(
            b.tree_row_at(Point { x: 20., y: PAD_Y }),
            None,
            "the heading"
        );
        let row = b.tree_row_at(Point {
            x: 20.,
            y: PAD_Y + b.line_h() * 1.5,
        });
        assert_eq!(row, Some(0), "the first file is the second row");
    }

    // The ordinary editor's file machinery must not touch a diff's merged
    // text: the disk poll, a draft, a save, the git gutter.
    #[test]
    fn a_diff_never_writes_reads_back_or_marks_the_merged_text() {
        let (root, mut b) = repo();
        let a_rs = format!("{}/a.rs", root.display());
        b.diff_start(Some(&a_rs), true);
        let text = b.buffer.text();
        // The file changes on disk: the diff's own poll rebuilds the merged
        // text; the ordinary one would have replaced it with the file.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&a_rs, "only\n").unwrap();
        b.idle(10_000.);
        assert_ne!(b.buffer.text(), text);
        assert!(b.buffer.text().contains("only"));
        assert!(
            b.buffer.text().contains("let v1 = 1;"),
            "HEAD's lines remain"
        );
        assert!(b.git_head.is_none(), "no gutter against HEAD");
        assert!(b.draft_due.is_none() && !b.is_dirty());
        // Save writes nothing.
        b.save(&a_rs, 0.);
        assert_eq!(std::fs::read_to_string(&a_rs).unwrap(), "only\n");
        // Typing into it is refused and says why.
        b.insert_composed("x", 0.);
        assert!(!b.buffer.text().contains('x'));
    }

    // Cmd+K's three states, as the editor's tree.
    #[test]
    fn the_list_toggles_through_its_three_states() {
        let (root, mut b) = repo();
        let a_rs = format!("{}/a.rs", root.display());
        b.diff_start(Some(&a_rs), false);
        // Started hidden? `diff_start` keeps the list the card asked for; the
        // body built it for its own state, the card draws it only when shown.
        assert!(b.diff_toggle_list(), "shown and focused");
        assert!(b.tree_focused);
        b.tree_focused = false;
        assert!(b.diff_toggle_list(), "focused again");
        assert!(b.tree_focused);
        assert!(!b.diff_toggle_list(), "hidden");
        assert!(b.tree.is_none() && b.focus == Focus::Buffer);
    }

    // Copy leaves the "N unchanged lines" lines out: they are in no file.
    #[test]
    fn copy_skips_the_placeholder_lines() {
        let (root, mut b) = repo();
        let a_rs = format!("{}/a.rs", root.display());
        b.diff_start(Some(&a_rs), true);
        b.buffer.select_all();
        let copied = b.selected_for_copy().unwrap();
        assert!(!copied.contains("unchanged lines"));
        assert!(copied.contains("let v10 = 100;"));
        assert!(copied.contains("let v10 = 10;"));
        assert!(
            copied.contains("let v1 = 1;"),
            "hidden lines are still lines"
        );
        assert_eq!(copied.split('\n').count(), 20 + 1 + 1);
    }

    // The old card's heading and counts, in the list.
    #[test]
    fn the_list_heading_has_the_totals() {
        let (root, mut b) = repo();
        let a_rs = format!("{}/a.rs", root.display());
        b.diff_start(Some(&a_rs), true);
        assert_eq!(
            b.diff_list_heading().as_deref(),
            Some("2 changed  +4 −1"),
            "a.rs +2 −1 and b.txt +2"
        );
    }
}
