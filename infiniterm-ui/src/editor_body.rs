//! The editor inside a card: one buffer bound to one file, drawn as lines
//! of shaped text with a gutter, a find panel and a file tree. Port of
//! `EditorCard.svelte` and `Explorer.svelte`; the logic is
//! `infiniterm-editor` (buffer, search, tree, highlighting) and this file
//! is the drawing and the key routing.
//!
//! Treated like a terminal card in every way that shows: the same font at
//! the same metrics (`Metrics`, measured once for both), the terminal
//! theme's colours for text and syntax (`editor_theme.rs`), a block
//! cursor in the cursor colour. What it does not have: LSP, completion,
//! lint, by decision.
//!
//! Files: loaded here, saved atomically, polled every two seconds so a
//! clean buffer takes the disk's version and a dirty one is told once and
//! keeps its text. A draft goes out half a second after a change and is
//! deleted when the buffer matches the file again; `closeCard` deletes
//! the rest. The card's `dirty`, `language` and `read_only` are copied
//! from here by `editors.rs` each frame, which is how the badges know.
//!
//! A picture (`Language::is_image`) is the same card with the text pane
//! showing the image instead: gpui decodes it through its asset cache
//! (`ImageAssetLoader`), it is fitted to the pane and never enlarged past
//! its own pixels at the canvas zoom, and the buffer stays empty and
//! read-only so nothing can save text over it. The tree, Cmd+K and the
//! disk poll work as for a file, so a screenshot taken again shows again.
use crate::body::{BodyAction, CardBody};
use crate::field::{Edit, Field};
use crate::terminal_body::Metrics;
use gpui::{
    fill, outline, point, px, size, App, Bounds, ClipboardItem, Corners, FontStyle, Hsla,
    ImageAssetLoader, Keystroke, Pixels, Resource, SharedString, TextRun, Window,
};
use infiniterm_core::editor_theme::{EditorColors, SyntaxRule};
use infiniterm_core::files::{
    dir_list, draft_delete, draft_read, draft_write, file_mtime, file_read, file_write,
};
use infiniterm_core::grid::{Point, Size};
use infiniterm_editor::buffer::Buffer;
use infiniterm_editor::explorer::{Entry, Tree, TreeAction};
use infiniterm_editor::highlight::{Highlighting, Span};
use infiniterm_editor::language::Language;
use infiniterm_editor::search::Search;
use infiniterm_editor::wrap::wrap_line;
use std::collections::HashMap;

/// One row on screen: a line, or a piece of a wrapped one, as a char range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VRow {
    line: usize,
    a: usize,
    b: usize,
}

/// The terminal host's inset, so the first column lines up across cards.
const PAD_X: f64 = 8.;
const PAD_Y: f64 = 6.;
/// A blink is slower than the terminal's: CodeMirror's 1200 ms period.
const BLINK_MS: f64 = 600.;
const DISK_POLL_MS: f64 = 2000.;
const DRAFT_MS: f64 = 500.;

/// Digits reserved in the gutter before it grows past three: line numbers
/// up to 999 fit without a mid-file resize.
const MIN_GUTTER_DIGITS: usize = 3;
/// Breathing room between the gutter's line numbers and the text that
/// follows; also the margin a line number is right-aligned by.
const GUTTER_EXTRA_PAD_PX: f64 = 16.;
/// The offset from the gutter's right edge to its separator line: the same
/// margin as the padding reserved beyond the line numbers.
const GUTTER_SEPARATOR_OFFSET_PX: f64 = 8.;
/// The folded-block chip after a header: this many cells wide, at this
/// alpha of the text colour.
const FOLD_CHIP_CELLS: f32 = 3.;
const FOLD_CHIP_ALPHA: f32 = 0.25;
/// The find-and-replace panel's height in line-heights: two field rows.
const SEARCH_PANEL_ROWS_REPLACE: f64 = 2.6;
/// The find-only panel's height in line-heights: one field row.
const SEARCH_PANEL_ROWS_FIND: f64 = 1.6;
/// Two spaces per tree depth level, CodeMirror's indent.
const TREE_INDENT: &str = "  ";
/// The search field's label text is smaller than the buffer's, like a
/// caption next to the value it labels.
const SEARCH_LABEL_FONT_SCALE: f64 = 0.85;
/// A search-panel row is taller than a line, so its field has click padding.
const SEARCH_ROW_HEIGHT_SCALE: f64 = 1.3;
/// Columns reserved for the "find"/"replace" label plus its gap before the
/// field box starts.
const SEARCH_LABEL_COLS: f64 = 9.;
/// The panel's inset from its own top edge.
const SEARCH_PANEL_TOP_PAD_PX: f64 = 3.;
/// A field box sits this far inside its row, top and bottom.
const FIELD_ROW_INSET_PX: f64 = 2.;
/// A field box is shorter than its row by both insets combined.
const FIELD_ROW_INSET_TOTAL_PX: f64 = 4.;
/// Gap between a field's border and the text or caret inside it.
const FIELD_TEXT_PAD_PX: f64 = 4.;
/// The selected-match highlight sits this far inside the field box.
const SELECTION_BG_INSET_PX: f64 = 1.;
/// The selected-match highlight is shorter than the field box by both
/// insets combined.
const SELECTION_BG_INSET_TOTAL_PX: f64 = 2.;
/// The caret in a search field sits this far below the field's top.
const CARET_TOP_INSET_PX: f64 = 3.;
/// The caret is narrower than a cell: a hairline-and-a-half reads as a
/// caret rather than a block.
const CARET_WIDTH_PX: f64 = 1.5;
/// The caret is shorter than the field box by its top and bottom insets.
const CARET_HEIGHT_INSET_PX: f64 = 6.;
/// The block cursor is 0.6 em wide, CodeMirror's ratio.
const CURSOR_BLOCK_WIDTH_RATIO: f64 = 0.6;
/// The blinking cursor's alpha when it is on.
const CURSOR_ALPHA: f32 = 0.85;
/// The active-line wash is barely-there: a hint, not a highlight.
const ACTIVE_LINE_ALPHA: f32 = 0.10;
/// A horizontal wheel gesture below this magnitude is noise from a
/// vertical scroll, not an intentional side-scroll.
const WHEEL_HORIZONTAL_THRESHOLD: f64 = 0.5;
/// Scrolling right to follow the cursor leaves this many columns of margin
/// past it, so the next character typed is never flush with the edge.
const CURSOR_SCROLL_MARGIN_COLS: f64 = 2.;
/// A search field's border is dimmer than its focused text.
const FIELD_BORDER_ALPHA: f32 = 0.6;
/// The matching-bracket outline is a soft hint, not a full-strength one.
const BRACKET_ALPHA: f32 = 0.5;
/// A search match that isn't the current one is dimmed to this alpha, so
/// the current match still reads as the one under the caret.
const MATCH_DIM_ALPHA: f32 = 0.45;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Buffer,
    Query,
    Replace,
    Tree,
}

/// What a body event asks the card frame to do besides repaint.
#[derive(Clone, Debug, PartialEq)]
pub enum EditorEvent {
    None,
    /// A line for the status bar.
    Notice(String),
    /// The card's path and directory changed (a save-as, a picture the
    /// tree's cursor landed on).
    PathChanged {
        path: String,
        cwd: String,
    },
    /// Enter on a file in the tree: a tab for it, or a switch to the tab
    /// that already shows it. `editor_tabs.rs` turns it into `Card.tabs`.
    OpenTab(String),
}

pub struct EditorBody {
    pub card_id: String,
    pub buffer: Buffer,
    pub path: Option<String>,
    pub cwd: String,
    /// The text as last read from or written to disk; dirty is "differs".
    saved: String,
    disk_stamp: Option<u64>,
    warned_stale: bool,
    last_disk_check: f64,
    draft_due: Option<f64>,
    pub language: Option<Language>,
    highlighting: Highlighting,
    /// Keyed by the buffer's version AND a load count: every loaded file
    /// starts a fresh buffer at version 0, and a cache keyed by version
    /// alone handed the previous file's spans (or none) to the next one.
    spans: (u64, u64, Vec<Span>),
    /// Bumped by every load, for the span cache's key.
    loads: u64,
    /// The column a visual up/down keeps aiming at, with the caret it was
    /// set for: a caret moved by anything else forgets it.
    visual_goal: Option<(usize, usize)>,
    pub search: Option<Search>,
    query: Field,
    replacement: Field,
    focus: Focus,
    /// First visible line, and the horizontal offset in world px.
    scroll_line: usize,
    scroll_x: f64,
    pub wrap: bool,
    pub highlight_line: bool,
    pub read_only: bool,
    pub colors: EditorColors,
    pub rules: Vec<SyntaxRule>,
    pub metrics: Metrics,
    pub blink: bool,
    blink_epoch: f64,
    painted_phase: bool,
    /// The caret's cell at the last paint, in window pixels; `None` while
    /// the tree has the focus. What `caret_bounds` answers with.
    painted_caret: Option<Bounds<Pixels>>,
    painted_focused: bool,
    dirty: bool,
    /// The wheel's fraction of a line, carried between events.
    wheel_carry: crate::chrome::WheelCarry,
    /// The file tree, once Cmd+K or `ift <dir>` asked for one.
    pub tree: Option<Tree>,
    pub tree_focused: bool,
    /// The tree's width in world px; `sidebar.rs` decides from the card.
    pub sidebar_w: f64,
    pub sidebar_top: bool,
    selecting: bool,
    /// The rows visible at the last paint, for scrolling the cursor into view.
    rows_visible: usize,
    /// A `go_to_line` that ran before the first paint, when `rows_visible`
    /// was not known yet: the paint that learns it centres the line then.
    /// Without this `file:20` opened with line 20 at the TOP (2026-09-26).
    centre_pending: bool,
    /// Shaped rows by (line, first char), keyed by what they were shaped from.
    shaped: HashMap<(usize, usize), (u64, gpui::ShapedLine)>,
    /// Pending events for the frame, drained by `take_event`.
    events: Vec<EditorEvent>,
    /// The line a link or `ift file:42` asked for, once the text is in.
    pub pending_line: Option<u64>,
    /// The file is a picture: shown, not read. `image_stale` says the
    /// disk poll saw it change, so the next paint drops gpui's cached copy.
    image: Option<String>,
    image_stale: bool,
    /// The card's size in world px, for hit tests between paints.
    world: Size,
}

fn hex(s: &str) -> Hsla {
    crate::chrome::hex(s).unwrap_or(gpui::white())
}

impl EditorBody {
    pub fn new(
        card_id: &str,
        path: Option<String>,
        cwd: String,
        metrics: &Metrics,
        world: Size,
    ) -> EditorBody {
        EditorBody {
            card_id: card_id.to_string(),
            buffer: Buffer::new(""),
            language: path.as_deref().and_then(Language::for_path),
            path,
            cwd,
            saved: String::new(),
            disk_stamp: None,
            warned_stale: false,
            last_disk_check: 0.,
            draft_due: None,
            highlighting: Highlighting::default(),
            spans: (u64::MAX, 0, vec![]),
            loads: 0,
            visual_goal: None,
            search: None,
            query: Field::default(),
            replacement: Field::default(),
            focus: Focus::Buffer,
            scroll_line: 0,
            scroll_x: 0.,
            wrap: false,
            highlight_line: false,
            read_only: false,
            colors: EditorColors {
                background: "#0e101a".into(),
                foreground: "#b9c4d2".into(),
                cursor: "#b9c4d2".into(),
                selection: "#e39500".into(),
                selection_text: "#0e101a".into(),
                gutter: "#5a6472".into(),
            },
            rules: vec![],
            metrics: metrics.clone(),
            blink: true,
            blink_epoch: 0.,
            painted_phase: true,
            painted_caret: None,
            painted_focused: false,
            dirty: true,
            wheel_carry: crate::chrome::WheelCarry::default(),
            tree: None,
            tree_focused: false,
            sidebar_w: 0.,
            sidebar_top: false,
            selecting: false,
            rows_visible: 1,
            centre_pending: false,
            shaped: HashMap::new(),
            events: vec![],
            pending_line: None,
            image: None,
            image_stale: false,
            world,
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.shaped.clear();
    }

    pub fn take_events(&mut self) -> Vec<EditorEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn is_dirty(&self) -> bool {
        !self.buffer.equals(&self.saved)
    }

    fn line_h(&self) -> f64 {
        self.metrics.font_px * self.metrics.line_height
    }

    fn now_dirty(&mut self, now: f64) {
        self.dirty = true;
        self.blink_epoch = now;
        self.draft_due = Some(now + DRAFT_MS);
    }

    /// Loads a file into this card: at creation, and from the tree. With
    /// the draft only at creation; a file picked from the tree is a fresh
    /// read. The card's path and directory follow.
    pub fn load(&mut self, path: &str, with_draft: bool, now: f64) {
        self.path = Some(path.to_string());
        self.cwd = parent_of(path);
        self.language = Language::for_path(path);
        self.warned_stale = false;
        self.scroll_line = 0;
        self.scroll_x = 0.;
        self.shaped.clear();
        self.loads += 1;
        self.image = Language::is_image(path).then(|| path.to_string());
        if self.image.is_some() {
            // Nothing to read: the decoder reads the file at paint time.
            self.buffer = Buffer::new("");
            self.saved.clear();
            self.disk_stamp = file_mtime(path);
            self.dirty = true;
            self.blink_epoch = now;
            return;
        }
        match file_read(path) {
            Ok(text) => {
                self.saved = text.clone();
                self.disk_stamp = file_mtime(path);
                let draft = if with_draft {
                    draft_read(&self.card_id).ok().flatten()
                } else {
                    None
                };
                self.buffer = Buffer::new(draft.as_deref().unwrap_or(&text));
            }
            // Not there yet: a new file, vim's way. The buffer starts
            // empty (or from its draft) and the first save creates it;
            // closing without one leaves nothing on disk.
            Err(_) if !std::path::Path::new(path).exists() => {
                self.saved.clear();
                self.disk_stamp = None;
                let draft = if with_draft {
                    draft_read(&self.card_id).ok().flatten()
                } else {
                    None
                };
                self.buffer = Buffer::new(draft.as_deref().unwrap_or(""));
                self.events.push(EditorEvent::Notice(format!(
                    "new file: {}",
                    path.rsplit('/').next().unwrap_or(path)
                )));
            }
            Err(e) => {
                eprintln!("[infiniterm/warn] could not read {path}: {e}");
                self.events.push(EditorEvent::Notice(format!(
                    "could not open {}",
                    path.rsplit('/').next().unwrap_or(path)
                )));
            }
        }
        if let Some(line) = self.pending_line.take() {
            self.go_to_line(line as usize);
        }
        self.dirty = true;
        self.blink_epoch = now;
    }

    /// Untitled: the draft is all there is.
    pub fn load_untitled(&mut self) {
        if let Ok(Some(draft)) = draft_read(&self.card_id) {
            self.buffer = Buffer::new(&draft);
        }
        self.loads += 1;
        self.dirty = true;
    }

    /// Writes the buffer to `path`, which the card now carries. A picture
    /// has an empty buffer, and writing that would erase the file.
    pub fn save(&mut self, path: &str, now: f64) {
        if self.image.is_some() {
            return;
        }
        if self.path.as_deref() != Some(path) {
            self.path = Some(path.to_string());
            self.cwd = parent_of(path);
            self.language = Language::for_path(path);
            // A save-as can change the grammar without changing the text.
            self.loads += 1;
        }
        let text = self.buffer.text();
        match file_write(path, &text) {
            Ok(()) => {
                self.saved = text;
                self.disk_stamp = file_mtime(path);
                self.draft_due = None;
                let _ = draft_delete(&self.card_id);
                self.events.push(EditorEvent::Notice(format!(
                    "saved {}",
                    path.rsplit('/').next().unwrap_or(path)
                )));
            }
            Err(e) => {
                eprintln!("[infiniterm/warn] could not save {path}: {e}");
                self.events
                    .push(EditorEvent::Notice(format!("could not save: {e}")));
            }
        }
        self.dirty = true;
        self.blink_epoch = now;
    }

    /// Housekeeping between frames: the draft, the disk poll.
    pub fn idle(&mut self, now: f64) {
        if self.draft_due.is_some_and(|due| now >= due) {
            self.draft_due = None;
            if self.is_dirty() {
                if let Err(e) = draft_write(&self.card_id, &self.buffer.text()) {
                    eprintln!("[infiniterm/warn] could not keep the draft: {e}");
                }
            } else {
                let _ = draft_delete(&self.card_id);
            }
        }
        if now - self.last_disk_check >= DISK_POLL_MS {
            self.last_disk_check = now;
            self.check_disk(now);
            // The tree follows the disk too: a file made outside the app
            // (a build, a download, another card's shell) appears.
            if let Some(tree) = self.tree.as_mut() {
                if tree.refresh(&mut list_dir) {
                    self.dirty = true;
                }
            }
        }
    }

    /// The file changing under the editor: the theme picker writes
    /// settings.json while it may be open. A clean buffer takes the disk
    /// version silently; a dirty one is told once and keeps its changes.
    fn check_disk(&mut self, now: f64) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let stamp = file_mtime(&path);
        if stamp.is_none() || stamp == self.disk_stamp {
            return;
        }
        self.disk_stamp = stamp;
        if self.image.is_some() {
            self.image_stale = true;
            self.dirty = true;
            return;
        }
        let Ok(text) = file_read(&path) else { return };
        if text == self.saved {
            return;
        }
        if self.is_dirty() {
            if !self.warned_stale {
                self.events.push(EditorEvent::Notice(format!(
                    "{} changed on disk; your unsaved changes are kept",
                    path.rsplit('/').next().unwrap_or(&path)
                )));
            }
            self.warned_stale = true;
            self.saved = text; // dirty now means "differs from the disk now"
            return;
        }
        self.saved = text.clone();
        self.buffer.set_text(&text, now);
        self.dirty = true;
    }

    pub fn go_to_line(&mut self, line: usize) {
        self.buffer.go_to_line(line);
        self.buffer.reveal_cursors();
        // Centre it: the cursor row halfway down the visible rows.
        self.centre_cursor();
        self.centre_pending = true;
        self.dirty = true;
    }

    fn centre_cursor(&mut self) {
        let target = self.buffer.line_of(self.buffer.cursor());
        self.scroll_line = target.saturating_sub(self.rows_visible / 2);
    }

    /// The paint's measure of the view; a pending `go_to_line` is centred
    /// against it once.
    fn set_rows_visible(&mut self, rows: usize) {
        self.rows_visible = rows;
        if std::mem::take(&mut self.centre_pending) {
            self.centre_cursor();
        }
    }

    /// A palette-only transform (Batch 1, 2026-09-24): runs the pure
    /// function on the selection, or the whole document when there is
    /// none, and puts the result back with the same range selected so the
    /// next transform can chain onto it. A no-op change (already sorted,
    /// already upper case) still records an undo step the way replacing a
    /// selection with itself would; harmless, and simpler than detecting it.
    pub fn apply_transform(&mut self, t: infiniterm_core::model::TextTransform, now: f64) {
        if self.read_only {
            return;
        }
        use infiniterm_core::model::TextTransform;
        use infiniterm_editor::buffer::INDENT;
        use infiniterm_editor::transforms as tf;
        let width = INDENT.chars().count();
        let f: fn(&str) -> String = match t {
            TextTransform::Upper => tf::to_upper,
            TextTransform::Lower => tf::to_lower,
            TextTransform::Title => tf::to_title_case,
            TextTransform::Snake => tf::to_snake_case,
            TextTransform::Kebab => tf::to_kebab_case,
            TextTransform::Camel => tf::to_camel_case,
            TextTransform::SortLines => tf::sort_lines,
            TextTransform::UniqueLines => tf::unique_lines,
            TextTransform::ReverseLines => tf::reverse_lines,
            TextTransform::TrimTrailingWhitespace => tf::trim_trailing_whitespace,
            TextTransform::IndentTabsToSpaces => {
                return self.replace_transform_range(|s| tf::indent_tabs_to_spaces(s, width), now)
            }
            TextTransform::IndentSpacesToTabs => {
                return self.replace_transform_range(|s| tf::indent_spaces_to_tabs(s, width), now)
            }
        };
        self.replace_transform_range(f, now);
    }

    fn replace_transform_range(&mut self, f: impl FnOnce(&str) -> String, now: f64) {
        let r = self
            .buffer
            .selection()
            .unwrap_or(0..self.buffer.len_chars());
        let text = self.buffer.slice(r.clone());
        let new = f(&text);
        self.buffer.replace_range(r.clone(), &new, now);
        self.buffer
            .select_range(r.start..r.start + new.chars().count());
        self.now_dirty(now);
    }

    pub fn open_search(&mut self, replacing: bool) {
        let mut s = self.search.take().unwrap_or_default();
        // The selection seeds the query, as every editor's find does.
        if let Some(sel) = self.buffer.selected_text().filter(|s| !s.contains('\n')) {
            s.query = sel;
        }
        s.replacing = s.replacing || replacing;
        self.query = Field::open(&s.query, !s.query.is_empty());
        self.replacement = Field::open(&s.replacement, false);
        s.refresh(&self.buffer.text(), self.buffer.cursor());
        self.search = Some(s);
        self.focus = if replacing {
            Focus::Replace
        } else {
            Focus::Query
        };
        self.dirty = true;
    }

    fn close_search(&mut self) {
        self.search = None;
        self.focus = Focus::Buffer;
        self.dirty = true;
    }

    fn refresh_search(&mut self) {
        if let Some(s) = self.search.as_mut() {
            s.query = self.query.text.clone();
            s.replacement = self.replacement.text.clone();
            s.refresh(&self.buffer.text(), self.buffer.cursor());
        }
    }

    /// Puts the caret on the current match and shows it.
    fn show_match(&mut self) {
        if let Some((a, b)) = self.search.as_ref().and_then(|s| s.current_range()) {
            // Jumping to a match is a reset, the same as `go_to_line`.
            self.buffer.collapse_to_primary();
            self.buffer.select_range(a..b);
            self.buffer.reveal_cursors();
            self.ensure_cursor_visible();
        }
    }

    fn ensure_cursor_visible(&mut self) {
        let line = self.buffer.line_of(self.buffer.cursor());
        if line < self.scroll_line {
            self.scroll_line = line;
        } else {
            // The first line that still lets the cursor's line fit, found
            // by walking BACK from the cursor's line summing the rows
            // wrapped lines take: a walk of at most one screen. Walking
            // forward from `scroll_line` and re-summing each step was
            // quadratic, and Cmd+End in a long file hung for it.
            let cols = self.cols_visible(self.world);
            let mut rows = self.rows_of(line, cols);
            let mut first = line;
            while first > self.scroll_line {
                let above = self.rows_of(first - 1, cols);
                if rows + above > self.rows_visible {
                    break;
                }
                rows += above;
                first -= 1;
            }
            if first > self.scroll_line {
                self.scroll_line = first;
            }
        }
        // Horizontal: the cursor's column, when lines do not wrap.
        if !self.wrap {
            let col = self.buffer.col_of(self.buffer.cursor()) as f64 * self.metrics.cell_w;
            let width = self.cols_visible(self.world) as f64 * self.metrics.cell_w;
            if col < self.scroll_x {
                self.scroll_x = col;
            } else if col > self.scroll_x + width - self.metrics.cell_w {
                self.scroll_x = col - width + self.metrics.cell_w * CURSOR_SCROLL_MARGIN_COLS;
            }
        }
        self.dirty = true;
    }

    /// Cmd+K's three states: tree hidden -> shown and focused -> (from the
    /// buffer) focused again -> (from the tree) hidden. `root` is the
    /// card's, defaulting to its directory. Returns whether the tree shows.
    pub fn toggle_tree(&mut self, root: &str) -> bool {
        let shown = match self.tree.as_mut() {
            Some(t) => {
                t.root = root.to_string();
                if self.tree_focused {
                    self.tree = None;
                    self.tree_focused = false;
                    false
                } else {
                    self.tree_focused = true;
                    true
                }
            }
            None => {
                let mut t = Tree::new(root);
                t.ensure_root(&mut list_dir);
                self.tree = Some(t);
                self.tree_focused = true;
                true
            }
        };
        self.focus = if shown { Focus::Tree } else { Focus::Buffer };
        self.dirty = true;
        shown
    }

    /// The tree is on because the card says so (`ift <dir>`, a restore).
    pub fn show_tree(&mut self, root: &str) {
        if self.tree.is_none() {
            let mut t = Tree::new(root);
            t.ensure_root(&mut list_dir);
            self.tree = Some(t);
            self.tree_focused = self.path.is_none();
            self.focus = if self.tree_focused {
                Focus::Tree
            } else {
                Focus::Buffer
            };
            self.dirty = true;
        }
    }

    /// A file picked in the tree. Into this card when its buffer is clean;
    /// a dirty buffer is left alone and the file opens beside, because
    /// replacing text you have not saved is the one thing a tree click
    /// must not do.
    /// Enter on a file in the tree: always a tab (Ekin's rule), never a
    /// swap of this buffer, so nothing unsaved is ever in the way.
    fn open_from_tree(&mut self, path: String, _now: f64) -> BodyAction {
        self.tree_focused = false;
        self.focus = Focus::Buffer;
        self.events.push(EditorEvent::OpenTab(path));
        BodyAction::None
    }

    /// Cmd+Alt+Arrow inside the card: between the text and the tree, in
    /// the direction the tree lies (left of the text, or above it). True
    /// when the focus moved; false at the card's edge, where the same key
    /// moves to the next CARD, the app's own rule one level down. Ekin's
    /// ask: one fewer key to remember than a chord for the tree.
    pub fn move_focus_within(&mut self, dir: infiniterm_core::navigate::Direction) -> bool {
        use infiniterm_core::navigate::Direction;
        if self.tree.is_none() {
            return false;
        }
        let toward_tree = match (self.sidebar_top, dir) {
            (false, Direction::Left) | (true, Direction::Up) => true,
            (false, Direction::Right) | (true, Direction::Down) => false,
            _ => return false,
        };
        match (toward_tree, self.tree_focused) {
            (true, false) => {
                self.tree_focused = true;
                self.focus = Focus::Tree;
            }
            (false, true) => {
                self.tree_focused = false;
                self.focus = Focus::Buffer;
            }
            _ => return false,
        }
        self.dirty = true;
        true
    }

    /// The tree moves to the tab that becomes active (`editor_tabs.rs`):
    /// taken from the one that was, with whether it had the focus.
    pub fn take_tree(&mut self) -> Option<(Tree, bool, f64, bool)> {
        let tree = self.tree.take()?;
        let focused = self.tree_focused;
        self.tree_focused = false;
        if self.focus == Focus::Tree {
            self.focus = Focus::Buffer;
        }
        self.dirty = true;
        Some((tree, focused, self.sidebar_w, self.sidebar_top))
    }

    pub fn put_tree(&mut self, tree: Tree, focused: bool, sidebar_w: f64, sidebar_top: bool) {
        self.tree = Some(tree);
        self.tree_focused = focused;
        self.sidebar_w = sidebar_w;
        self.sidebar_top = sidebar_top;
        if focused {
            self.focus = Focus::Tree;
        }
        self.dirty = true;
    }

    /// Whether a point in the text area (below any strip) is on the tree
    /// rather than the text, for the lock: clicking the tree must not lock.
    pub fn is_on_tree(&self, local: Point) -> bool {
        self.tree.is_some()
            && if self.sidebar_top {
                local.y < self.sidebar_w
            } else {
                local.x < self.sidebar_w
            }
    }

    fn blink_on(&self, now: f64) -> bool {
        !self.blink || (((now - self.blink_epoch) / BLINK_MS) as u64).is_multiple_of(2)
    }

    // ----- geometry -----

    fn gutter_w(&self) -> f64 {
        let digits = self
            .buffer
            .line_count()
            .max(1)
            .to_string()
            .len()
            .max(MIN_GUTTER_DIGITS);
        digits as f64 * self.metrics.cell_w + GUTTER_EXTRA_PAD_PX
    }

    fn panel_h(&self) -> f64 {
        match &self.search {
            None => 0.,
            Some(s) => {
                self.line_h()
                    * if s.replacing {
                        SEARCH_PANEL_ROWS_REPLACE
                    } else {
                        SEARCH_PANEL_ROWS_FIND
                    }
            }
        }
    }

    /// The text area's origin and size in world px, inside the card.
    fn text_area(&self, world: Size) -> (Point, Size) {
        let (mut x, mut y, mut w, mut h) = (0., 0., world.w, world.h);
        if self.tree.is_some() {
            if self.sidebar_top {
                y += self.sidebar_w;
                h -= self.sidebar_w;
            } else {
                x += self.sidebar_w;
                w -= self.sidebar_w;
            }
        }
        y += self.panel_h();
        h -= self.panel_h();
        (Point { x, y }, Size { w, h })
    }

    fn cols_visible(&self, world: Size) -> usize {
        let (_, size) = self.text_area(world);
        (((size.w - PAD_X * 2. - self.gutter_w()) / self.metrics.cell_w).floor()).max(1.) as usize
    }

    /// The rows on screen from `scroll_line`: one per line, or several when
    /// a prose line wraps at `cols`.
    fn visual_rows(&self, cols: usize, max_rows: usize) -> Vec<VRow> {
        let mut out = vec![];
        let count = self.buffer.line_count();
        let mut line = self.scroll_line.min(count.saturating_sub(1));
        while line < count && out.len() < max_rows {
            if self.buffer.is_line_hidden(line) {
                line += 1;
                continue;
            }
            if self.wrap {
                for (a, b) in wrap_line(&self.buffer.line(line), cols) {
                    if out.len() >= max_rows {
                        break;
                    }
                    out.push(VRow { line, a, b });
                }
            } else {
                let len = self.buffer.line(line).chars().count();
                out.push(VRow { line, a: 0, b: len });
            }
            line += 1;
        }
        out
    }

    /// How many visual rows `line` takes.
    fn rows_of(&self, line: usize, cols: usize) -> usize {
        if self.wrap {
            wrap_line(&self.buffer.line(line), cols).len()
        } else {
            1
        }
    }

    /// Up and down by VISUAL row: on a wrapped line the caret moves to the
    /// row above or below within the line, and only past the line's first
    /// or last row to the neighbouring line, which is how every wrapping
    /// editor behaves. Ekin: "it treats multiline wrapped text as a single
    /// line". Unwrapped, this is the buffer's own move. The column aimed
    /// at is kept across rows (`visual_goal`) the way the buffer keeps it
    /// across lines, and forgotten when anything else moves the caret.
    fn move_visual(&mut self, delta: i64, select: bool) {
        if !self.wrap {
            // Multi-cursor (Batch 2, 2026-09-25): every cursor moves, not
            // only the primary. The wrapped path below stays primary-only
            // for now — `visual_goal` is keyed to one cursor and `wrap` is
            // off by default, so this covers the common case.
            self.buffer.for_each_cursor(|b| {
                if delta < 0 {
                    b.move_up(select);
                } else {
                    b.move_down(select);
                }
            });
            return;
        }
        let cols = self.cols_visible(self.world);
        let cursor = self.buffer.cursor();
        let line = self.buffer.line_of(cursor);
        let col = self.buffer.col_of(cursor);
        let rows = wrap_line(&self.buffer.line(line), cols);
        // The row the caret is on: the one whose range holds the column,
        // the last row taking the column at the line's very end.
        let r = rows
            .iter()
            .position(|(a, b)| col >= *a && col < *b)
            .unwrap_or(rows.len() - 1);
        let goal = match self.visual_goal {
            Some((at, g)) if at == cursor => g,
            _ => col - rows[r].0,
        };
        let (target_line, target_row) = if delta < 0 {
            if r > 0 {
                (line, r - 1)
            } else if line == 0 {
                self.place_visual(0, select, goal);
                return;
            } else {
                let prev = wrap_line(&self.buffer.line(line - 1), cols);
                (line - 1, prev.len() - 1)
            }
        } else if r + 1 < rows.len() {
            (line, r + 1)
        } else if line + 1 >= self.buffer.line_count() {
            self.place_visual(self.buffer.len_chars(), select, goal);
            return;
        } else {
            (line + 1, 0)
        };
        let target = wrap_line(&self.buffer.line(target_line), cols)[target_row];
        let len = target.1 - target.0;
        let idx = self.buffer.line_start(target_line) + target.0 + goal.min(len);
        self.place_visual(idx, select, goal);
    }

    fn place_visual(&mut self, idx: usize, select: bool, goal: usize) {
        if select {
            self.buffer.select_to(idx);
        } else {
            self.buffer.set_cursor(idx);
        }
        self.visual_goal = Some((self.buffer.cursor(), goal));
    }

    /// The char index under a point in the text area.
    fn index_at(&self, local: Point, world: Size) -> usize {
        let (origin, _) = self.text_area(world);
        let x = local.x - origin.x - PAD_X - self.gutter_w() + self.scroll_x;
        let y = local.y - origin.y - PAD_Y;
        let row = (y / self.line_h()).floor().max(0.) as usize;
        let cols = self.cols_visible(world);
        let rows = self.visual_rows(cols, row + 1);
        let Some(vrow) = rows.get(row).or(rows.last()) else {
            return 0;
        };
        let col = ((x / self.metrics.cell_w) + 0.5).floor().max(0.) as usize;
        let len = vrow.b - vrow.a;
        self.buffer.line_start(vrow.line) + vrow.a + col.min(len)
    }

    /// The picture, fitted to the pane and centred. Its natural size is
    /// its pixels at the window's scale, times the zoom, so a screenshot
    /// at 100% is pixel for pixel and zooming out shrinks it with the
    /// card; a pane smaller than that shrinks it further, aspect kept.
    fn paint_image(
        &mut self,
        path: &str,
        area: Bounds<Pixels>,
        scale: f64,
        window: &mut Window,
        cx: &mut App,
    ) {
        let source = Resource::Path(std::path::Path::new(path).into());
        if self.image_stale {
            self.image_stale = false;
            cx.remove_asset::<ImageAssetLoader>(&source);
        }
        let pad = px((PAD_X * scale) as f32);
        let font_size = px((self.metrics.font_px * scale) as f32);
        let line_h = px((self.line_h() * scale) as f32);
        let say = |text: &str, window: &mut Window, cx: &mut App| {
            let run = TextRun {
                len: text.len(),
                font: self.metrics.font(),
                color: hex(&self.colors.gutter),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window.text_system().shape_line(
                SharedString::from(text.to_string()),
                font_size,
                &[run],
                None,
            );
            let _ = line.paint(
                point(
                    area.origin.x + pad,
                    area.origin.y + px((PAD_Y * scale) as f32),
                ),
                line_h,
                window,
                cx,
            );
        };
        let image = match window.use_asset::<ImageAssetLoader>(&source, cx) {
            None => return, // decoding; gpui redraws the view when it lands
            Some(Err(e)) => {
                say(&format!("could not show this picture: {e}"), window, cx);
                return;
            }
            Some(Ok(image)) => image,
        };
        let natural = image.size(0);
        let device_scale = window.scale_factor() as f64;
        let (w, h) = (
            natural.width.0 as f64 / device_scale * scale,
            natural.height.0 as f64 / device_scale * scale,
        );
        if w <= 0. || h <= 0. {
            return;
        }
        let fit_w = (f32::from(area.size.width) as f64 - 2. * PAD_X * scale).max(1.);
        let fit_h = (f32::from(area.size.height) as f64 - 2. * PAD_Y * scale).max(1.);
        let shrink = (fit_w / w).min(fit_h / h).min(1.);
        let (w, h) = (w * shrink, h * shrink);
        let rect = Bounds::new(
            point(
                area.origin.x + px(((f32::from(area.size.width) as f64 - w) / 2.) as f32),
                area.origin.y + px(((f32::from(area.size.height) as f64 - h) / 2.) as f32),
            ),
            size(px(w as f32), px(h as f32)),
        );
        if let Err(e) = window.paint_image(rect, Corners::default(), image, 0, false) {
            say(&format!("could not show this picture: {e}"), window, cx);
        }
    }

    // ----- keys -----

    fn key_buffer(&mut self, k: &Keystroke, now: f64, cx: &mut App) {
        let m = &k.modifiers;
        let shift = m.shift;
        let key = k.key.as_str();
        let ro = self.read_only;
        if m.platform {
            match key {
                "z" if shift => self.buffer.redo(),
                "z" => self.buffer.undo(),
                "a" => self.buffer.select_all(),
                // A whole line, unselected, is what Sublime's
                // `copy_with_empty_selection` copies and cuts.
                "c" => {
                    let t = self.buffer.selected_text().or_else(|| {
                        Some(self.buffer.line(self.buffer.line_of(self.buffer.cursor())))
                    });
                    if let Some(t) = t {
                        cx.write_to_clipboard(ClipboardItem::new_string(t));
                    }
                }
                "x" => {
                    if self.buffer.selection().is_some() {
                        if let Some(t) = self.buffer.selected_text() {
                            cx.write_to_clipboard(ClipboardItem::new_string(t));
                            if !ro {
                                self.buffer.backspace(now);
                            }
                        }
                    } else {
                        let t = self.buffer.line(self.buffer.line_of(self.buffer.cursor()));
                        cx.write_to_clipboard(ClipboardItem::new_string(t));
                        if !ro {
                            self.buffer.delete_line(now);
                        }
                    }
                }
                "v" => {
                    if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        if !ro {
                            self.buffer.insert(&t, now);
                        }
                    }
                }
                "f" if m.alt => self.open_search(true),
                "f" => self.open_search(false),
                // Ctrl+Cmd+G: every occurrence of the selection becomes
                // its own cursor (Batch 2, 2026-09-25); plain Cmd+G stays
                // find-next, unrelated, and only makes sense with a
                // search open.
                "g" if m.control => self.buffer.select_all_occurrences(),
                "g" => {
                    if let Some(s) = self.search.as_mut() {
                        if shift {
                            s.prev()
                        } else {
                            s.next()
                        }
                        self.show_match();
                    }
                }
                "/" => {
                    if !ro {
                        let token = self.language.map_or("//", Language::comment_token);
                        self.buffer.toggle_comment(token, now);
                    }
                }
                "left" => self.buffer.for_each_cursor(|b| b.move_line_start(shift)),
                "right" => self.buffer.for_each_cursor(|b| b.move_line_end(shift)),
                "up" if m.control => {
                    if !ro {
                        self.buffer.swap_line_up(now)
                    }
                }
                "down" if m.control => {
                    if !ro {
                        self.buffer.swap_line_down(now)
                    }
                }
                "up" => self.buffer.for_each_cursor(|b| b.move_doc_start(shift)),
                "down" => self.buffer.for_each_cursor(|b| b.move_doc_end(shift)),
                "backspace" => {
                    if !ro {
                        self.buffer.for_each_cursor(|b| b.delete_to_line_start(now))
                    }
                }
                "d" if shift => {
                    if !ro {
                        self.buffer.duplicate_line(now)
                    }
                }
                "d" => self.buffer.select_word_or_next(),
                // Cmd+Shift+L: split first (Ekin's most-used multi-cursor
                // move, 2026-09-27); plain Cmd+L stays expand-to-line.
                "l" if shift => self.buffer.split_into_lines(),
                "l" => self.buffer.expand_line_selection(),
                "j" if shift => {
                    if !ro {
                        self.buffer.join_lines(now)
                    }
                }
                // Cmd+Alt+[ / ] fold and unfold the block at the caret, with
                // Shift every outermost block. By physical key: Option turns
                // the bracket into another glyph on most layouts.
                _ if m.alt
                    && matches!(
                        crate::keycode::last_code(),
                        Some("BracketLeft" | "BracketRight")
                    ) =>
                {
                    let open = crate::keycode::last_code() == Some("BracketRight");
                    match (open, crate::keycode::last_shift()) {
                        (false, false) => self.buffer.fold_at_cursor(),
                        (false, true) => self.buffer.fold_all(),
                        (true, false) => self.buffer.unfold_at_cursor(),
                        (true, true) => self.buffer.unfold_all(),
                    }
                }
                "]" => {
                    if !ro {
                        self.buffer.indent_line(now)
                    }
                }
                "[" => {
                    if !ro {
                        self.buffer.outdent(now)
                    }
                }
                "enter" if shift => {
                    if !ro {
                        self.buffer.add_line_above(now)
                    }
                }
                "enter" => {
                    if !ro {
                        self.buffer.add_line_below(now)
                    }
                }
                _ => return,
            }
        } else if m.alt {
            match key {
                "left" => self.buffer.for_each_cursor(|b| b.move_word_left(shift)),
                "right" => self.buffer.for_each_cursor(|b| b.move_word_right(shift)),
                "backspace" => {
                    if !ro {
                        self.buffer.for_each_cursor(|b| b.delete_word_back(now))
                    }
                }
                "up" => self.buffer.for_each_cursor(|b| b.move_up(shift)),
                "down" => self.buffer.for_each_cursor(|b| b.move_down(shift)),
                _ => {
                    // Option+letter types the character macOS composed.
                    if let Some(ch) = k.key_char.as_deref().filter(|c| !ro && !c.is_empty()) {
                        self.buffer.for_each_cursor(|b| {
                            for c in ch.chars() {
                                b.type_char(c, now);
                            }
                        });
                    } else {
                        return;
                    }
                }
            }
        } else if m.control {
            match key {
                "a" => self.buffer.for_each_cursor(|b| b.move_line_start(shift)),
                "e" => self.buffer.for_each_cursor(|b| b.move_line_end(shift)),
                "k" if shift => {
                    if !ro {
                        self.buffer.delete_line(now)
                    }
                }
                "m" if shift => self.buffer.expand_to_brackets(),
                "m" => {
                    if let Some((a, b)) = self.buffer.matching_bracket() {
                        let c = self.buffer.cursor();
                        let at_open = c == a || c == a + 1;
                        self.buffer.set_cursor(if at_open { b + 1 } else { a + 1 });
                    }
                }
                // Multi-cursor (Batch 2, 2026-09-25): a column of cursors,
                // one more per press.
                "down" if shift => self.buffer.add_cursor_line(true),
                "up" if shift => self.buffer.add_cursor_line(false),
                _ => return,
            }
        } else {
            match key {
                "left" => self.buffer.for_each_cursor(|b| b.move_left(shift)),
                "right" => self.buffer.for_each_cursor(|b| b.move_right(shift)),
                "up" => self.move_visual(-1, shift),
                "down" => self.move_visual(1, shift),
                "home" => self.buffer.for_each_cursor(|b| b.move_line_start(shift)),
                "end" => self.buffer.for_each_cursor(|b| b.move_line_end(shift)),
                "pageup" => {
                    let rows = self.rows_visible;
                    self.buffer
                        .for_each_cursor(|b| b.move_page(rows, false, shift));
                }
                "pagedown" => {
                    let rows = self.rows_visible;
                    self.buffer
                        .for_each_cursor(|b| b.move_page(rows, true, shift));
                }
                "backspace" => {
                    if !ro {
                        self.buffer.for_each_cursor(|b| b.backspace(now))
                    }
                }
                "delete" => {
                    if !ro {
                        self.buffer.for_each_cursor(|b| b.delete_forward(now))
                    }
                }
                "enter" => {
                    if !ro {
                        self.buffer.for_each_cursor(|b| b.newline(now))
                    }
                }
                "tab" if shift => {
                    if !ro {
                        self.buffer.for_each_cursor(|b| b.outdent(now))
                    }
                }
                "tab" => {
                    if !ro {
                        self.buffer.for_each_cursor(|b| b.tab(now))
                    }
                }
                "escape" => {
                    if self.search.is_some() {
                        self.close_search();
                    } else if self.buffer.cursor_count() > 1 {
                        // Down to one cursor first, Sublime's rule; a
                        // second Escape then drops its selection too.
                        self.buffer.collapse_to_primary();
                    } else {
                        let c = self.buffer.cursor();
                        self.buffer.set_cursor(c);
                    }
                }
                _ => {
                    if let Some(ch) = k.key_char.as_deref().filter(|c| !c.is_empty()) {
                        if ro {
                            self.events.push(EditorEvent::Notice(
                                "read-only: the defaults file is rewritten every launch".into(),
                            ));
                            return;
                        }
                        self.buffer.for_each_cursor(|b| {
                            for c in ch.chars() {
                                b.type_char(c, now);
                            }
                        });
                    } else {
                        return;
                    }
                }
            }
        }
        self.buffer.reveal_cursors();
        self.ensure_cursor_visible();
        self.now_dirty(now);
        if self.search.is_some() {
            self.refresh_search();
        }
    }

    fn key_search(&mut self, k: &Keystroke, now: f64, cx: &mut App) {
        let m = &k.modifiers;
        let key = k.key.as_str();
        match key {
            "escape" => {
                self.close_search();
                return;
            }
            "tab" => {
                if let Some(s) = self.search.as_mut() {
                    s.replacing = true;
                }
                self.focus = if self.focus == Focus::Query {
                    Focus::Replace
                } else {
                    Focus::Query
                };
                self.dirty = true;
                return;
            }
            "enter" if self.focus == Focus::Query => {
                if let Some(s) = self.search.as_mut() {
                    if m.shift {
                        s.prev()
                    } else {
                        s.next()
                    }
                }
                self.show_match();
                return;
            }
            "enter" if self.focus == Focus::Replace => {
                if self.read_only {
                    return;
                }
                self.refresh_search();
                let replacement = self.replacement.text.clone();
                if m.platform {
                    // Cmd+Enter: every match, last to first so the ranges hold.
                    let matches = self
                        .search
                        .as_ref()
                        .map(|s| s.matches.clone())
                        .unwrap_or_default();
                    for (a, b) in matches.into_iter().rev() {
                        self.buffer.replace_range(a..b, &replacement, now);
                    }
                } else if let Some((a, b)) = self.search.as_ref().and_then(|s| s.current_range()) {
                    self.buffer.replace_range(a..b, &replacement, now);
                }
                self.now_dirty(now);
                self.refresh_search();
                self.show_match();
                return;
            }
            "f" if m.platform && m.alt => {
                if let Some(s) = self.search.as_mut() {
                    s.replacing = !s.replacing;
                    if !s.replacing {
                        self.focus = Focus::Query;
                    }
                }
                self.dirty = true;
                return;
            }
            "g" if m.platform => {
                if let Some(s) = self.search.as_mut() {
                    if m.shift {
                        s.prev()
                    } else {
                        s.next()
                    }
                }
                self.show_match();
                return;
            }
            _ => {}
        }
        let paste = (m.platform && key == "v")
            .then(|| cx.read_from_clipboard().and_then(|c| c.text()))
            .flatten();
        let field = if self.focus == Focus::Query {
            &mut self.query
        } else {
            &mut self.replacement
        };
        let edit = field.key(k, paste.as_deref());
        if let Some(text) = edit.clipboard() {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        }
        match edit {
            Edit::Changed | Edit::Cut(_) => {
                self.refresh_search();
                if self.focus == Focus::Query {
                    self.show_match();
                }
                self.dirty = true;
            }
            Edit::Handled | Edit::Copy(_) => self.dirty = true,
            Edit::Ignored => {}
        }
    }

    fn key_tree(&mut self, k: &Keystroke, now: f64) -> BodyAction {
        if k.modifiers.platform || k.modifiers.control || k.modifiers.alt {
            return BodyAction::None;
        }
        let Some(tree) = self.tree.as_mut() else {
            return BodyAction::None;
        };
        let action = tree.key(&k.key, &mut list_dir);
        self.dirty = true;
        match action {
            TreeAction::None => BodyAction::None,
            TreeAction::Close => {
                self.tree_focused = false;
                self.focus = Focus::Buffer;
                BodyAction::None
            }
            TreeAction::Open(path) => self.open_from_tree(path, now),
            TreeAction::Landed(path) => {
                self.preview_from_tree(path, now);
                BodyAction::None
            }
        }
    }

    /// A picture the tree's cursor landed on is shown at once, the tree
    /// keeping the focus so the next arrow shows the next one. Text is
    /// not: it waits for Enter. Unsaved text is never swapped out from
    /// under the reader for a preview.
    fn preview_from_tree(&mut self, path: String, now: f64) {
        if !Language::is_image(&path) || (self.is_dirty() && self.path.is_some()) {
            return;
        }
        self.load(&path, false, now);
        self.events.push(EditorEvent::PathChanged {
            path,
            cwd: self.cwd.clone(),
        });
    }

    /// Keys with a possible action for the frame (a file opened beside).
    pub fn key_action(&mut self, k: &Keystroke, now: f64, cx: &mut App) -> BodyAction {
        // A key that produces an EMPTY character and carries no chord is a
        // composition prefix: a dead key on its own (Option+E waiting for
        // its vowel). Nothing here can do anything with it, and taking it
        // would stop macOS composing, so it is left for the input context.
        // The composed text comes back through `insert_text`.
        let m = &k.modifiers;
        if !m.platform && !m.control && k.key_char.as_deref() == Some("") {
            return BodyAction::Ignored;
        }
        match self.focus {
            Focus::Tree => self.key_tree(k, now),
            Focus::Query | Focus::Replace => {
                self.key_search(k, now, cx);
                BodyAction::None
            }
            Focus::Buffer => {
                self.key_buffer(k, now, cx);
                BodyAction::None
            }
        }
    }

    /// The emoji panel, a finished composition, an input method's commit:
    /// into the buffer, or into whichever find field has the focus.
    pub fn insert_composed(&mut self, text: &str, now: f64) {
        match self.focus {
            Focus::Buffer => {
                if !self.read_only {
                    self.buffer.insert(text, now);
                    self.dirty = true;
                }
            }
            Focus::Query | Focus::Replace => {
                let field = if self.focus == Focus::Query {
                    &mut self.query
                } else {
                    &mut self.replacement
                };
                field.insert_text(text);
                self.refresh_search();
                if self.focus == Focus::Query {
                    self.show_match();
                }
                self.dirty = true;
            }
            Focus::Tree => {}
        }
    }

    // ----- paint -----

    fn spans_for(&mut self) -> &[Span] {
        if (self.spans.0, self.spans.1) != (self.buffer.version, self.loads) {
            let spans = match self.language {
                Some(l) => self.highlighting.spans(l, &self.buffer.text()),
                None => vec![],
            };
            self.spans = (self.buffer.version, self.loads, spans);
        }
        &self.spans.2
    }

    fn rule_for(&self, capture: &str) -> Option<&SyntaxRule> {
        self.rules.iter().find(|r| r.tag == capture)
    }

    fn paint_tree(
        &mut self,
        area: Bounds<Pixels>,
        scale: f64,
        focused: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(tree) = &self.tree else { return };
        let line_h = px((self.line_h() * scale) as f32);
        let font_size = px((self.metrics.font_px * scale) as f32);
        let f = self.metrics.font();
        let rows = tree.rows();
        let visible = ((f32::from(area.size.height) / f32::from(line_h)).floor() as usize).max(1);
        // The cursor row stays in view.
        let scroll = if tree.cursor < tree.scroll {
            tree.cursor
        } else if tree.cursor >= tree.scroll + visible {
            tree.cursor + 1 - visible
        } else {
            tree.scroll
        };
        let fg = hex(&self.colors.foreground);
        let sel_bg = hex(&self.colors.selection);
        let sel_fg = hex(&self.colors.selection_text);
        let current = self.path.clone();
        let pad = px((PAD_X * scale) as f32);
        let mut y = area.origin.y + px((PAD_Y * scale) as f32);
        for (i, row) in rows.iter().enumerate().skip(scroll).take(visible) {
            let is_cursor = i == tree.cursor;
            let row_b = Bounds::new(point(area.origin.x, y), size(area.size.width, line_h));
            let mut color = fg;
            if is_cursor && self.tree_focused && focused {
                window.paint_quad(fill(row_b, sel_bg));
                color = sel_fg;
            } else if is_cursor {
                window.paint_quad(fill(
                    row_b,
                    crate::chrome::with_alpha(fg, crate::chrome::TREE_CURSOR_UNFOCUSED_ALPHA),
                ));
            }
            if current.as_deref() == Some(row.entry.path.as_str())
                && !(is_cursor && self.tree_focused && focused)
            {
                color = hex(&self.colors.selection);
            }
            let marker = if row.entry.is_dir {
                if tree.is_expanded(&row.entry.path) {
                    "▾ "
                } else {
                    "▸ "
                }
            } else {
                "  "
            };
            let mut text = format!(
                "{}{}{}",
                TREE_INDENT.repeat(row.depth),
                marker,
                row.entry.name
            );
            let mut line = crate::text::shape(window, &text, font_size, &f, color);
            // A name wider than the tree is cut with an ellipsis, not drawn
            // over the buffer beside it. The row's highlight already stopped
            // at the tree's edge; the text kept going.
            let room = area.size.width - pad * 2.;
            if line.width > room {
                text = crate::text::elide(&text, f32::from(room), |t| {
                    f32::from(crate::text::shape(window, t, font_size, &f, color).width)
                });
                line = crate::text::shape(window, &text, font_size, &f, color);
            }
            let _ = line.paint(point(area.origin.x + pad, y), line_h, window, cx);
            y += line_h;
        }
        // A hairline between the tree and the text.
        let hairline = px(crate::chrome::HAIRLINE_PX as f32);
        let edge = if self.sidebar_top {
            Bounds::new(
                point(area.origin.x, area.origin.y + area.size.height - hairline),
                size(area.size.width, hairline),
            )
        } else {
            Bounds::new(
                point(area.origin.x + area.size.width - hairline, area.origin.y),
                size(hairline, area.size.height),
            )
        };
        window.paint_quad(fill(
            edge,
            crate::chrome::with_alpha(hex(&self.colors.gutter), crate::chrome::HAIRLINE_ALPHA),
        ));
        if let Some(t) = self.tree.as_mut() {
            t.scroll = scroll;
        }
    }

    fn paint_panel(
        &self,
        area: Bounds<Pixels>,
        scale: f64,
        focused: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(s) = &self.search else { return };
        let line_h = px((self.line_h() * scale) as f32);
        let font_size = px((self.metrics.font_px * SEARCH_LABEL_FONT_SCALE * scale) as f32);
        let f = self.metrics.font();
        let fg = hex(&self.colors.foreground);
        let dim = hex(&self.colors.gutter);
        let sel_bg = hex(&self.colors.selection);
        let sel_fg = hex(&self.colors.selection_text);
        window.paint_quad(fill(area, hex(&self.colors.background)));
        let pad = px((PAD_X * scale) as f32);
        let row_h = line_h * SEARCH_ROW_HEIGHT_SCALE as f32;
        let label_w = px((self.metrics.cell_w * SEARCH_LABEL_COLS * scale) as f32);
        let mut y = area.origin.y + px((SEARCH_PANEL_TOP_PAD_PX * scale) as f32);
        let rows: Vec<(&str, &Field, Focus)> = if s.replacing {
            vec![
                ("find", &self.query, Focus::Query),
                ("replace", &self.replacement, Focus::Replace),
            ]
        } else {
            vec![("find", &self.query, Focus::Query)]
        };
        for (label, field, which) in rows {
            let l = crate::text::shape(window, label, font_size, &f, dim);
            let _ = l.paint(point(area.origin.x + pad, y), row_h, window, cx);
            let active = self.focus == which && focused;
            let field_b = Bounds::new(
                point(
                    area.origin.x + pad + label_w,
                    y + px(FIELD_ROW_INSET_PX as f32),
                ),
                size(
                    area.size.width
                        - pad * 2.
                        - label_w
                        - px((self.metrics.cell_w * SEARCH_LABEL_COLS * scale) as f32),
                    row_h - px(FIELD_ROW_INSET_TOTAL_PX as f32),
                ),
            );
            window.paint_quad(
                outline(
                    field_b,
                    crate::chrome::with_alpha(if active { fg } else { dim }, FIELD_BORDER_ALPHA),
                    gpui::BorderStyle::Solid,
                )
                .border_widths(px(crate::chrome::HAIRLINE_PX as f32)),
            );
            // Three pieces, measured separately, so the selection's ground
            // covers only the selected part and the caret sits where the
            // caret is rather than at the end.
            let (before, selected, after) = if field.text.is_empty() && !active {
                (String::new(), String::new(), String::new())
            } else {
                field.parts()
            };
            let x0 = field_b.origin.x + px(FIELD_TEXT_PAD_PX as f32);
            let ty = y + px(FIELD_ROW_INSET_PX as f32);
            let th = row_h - px(FIELD_ROW_INSET_TOTAL_PX as f32);
            let mut x = x0;
            if !before.is_empty() {
                let line = crate::text::shape(window, &before, font_size, &f, fg);
                let _ = line.paint(point(x, ty), th, window, cx);
                x += line.width;
            }
            if !selected.is_empty() {
                let line = crate::text::shape(window, &selected, font_size, &f, sel_fg);
                window.paint_quad(fill(
                    Bounds::new(
                        point(x, field_b.origin.y + px(SELECTION_BG_INSET_PX as f32)),
                        size(
                            line.width,
                            field_b.size.height - px(SELECTION_BG_INSET_TOTAL_PX as f32),
                        ),
                    ),
                    sel_bg,
                ));
                let _ = line.paint(point(x, ty), th, window, cx);
                x += line.width;
            } else if active {
                window.paint_quad(fill(
                    Bounds::new(
                        point(x, field_b.origin.y + px(CARET_TOP_INSET_PX as f32)),
                        size(
                            px(CARET_WIDTH_PX as f32),
                            field_b.size.height - px(CARET_HEIGHT_INSET_PX as f32),
                        ),
                    ),
                    fg,
                ));
            }
            if !after.is_empty() {
                let line = crate::text::shape(window, &after, font_size, &f, fg);
                let _ = line.paint(point(x, ty), th, window, cx);
            }
            if which == Focus::Query {
                let count = match (s.current, s.matches.len()) {
                    (_, 0) if s.query.is_empty() => String::new(),
                    (_, 0) => "no matches".into(),
                    (Some(i), n) => format!("{} of {n}", i + 1),
                    (None, n) => format!("{n}"),
                };
                let c = crate::text::shape(window, &count, font_size, &f, dim);
                let _ = c.paint(
                    point(area.origin.x + area.size.width - pad - c.width, y),
                    row_h,
                    window,
                    cx,
                );
            }
            y += row_h;
        }
        let hairline = px(crate::chrome::HAIRLINE_PX as f32);
        window.paint_quad(fill(
            Bounds::new(
                point(area.origin.x, area.origin.y + area.size.height - hairline),
                size(area.size.width, hairline),
            ),
            crate::chrome::with_alpha(dim, crate::chrome::HAIRLINE_ALPHA),
        ));
    }
}

fn parent_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) => "/".into(),
        Some((dir, _)) => dir.into(),
        None => "/".into(),
    }
}

fn list_dir(dir: &str) -> Vec<Entry> {
    match dir_list(dir) {
        Ok(entries) => entries
            .into_iter()
            .map(|e| Entry {
                name: e.name,
                path: e.path.to_string_lossy().into_owned(),
                is_dir: e.is_dir,
            })
            .collect(),
        Err(e) => {
            eprintln!("[infiniterm/warn] explorer: {e}");
            vec![]
        }
    }
}

impl CardBody for EditorBody {
    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        scale: f64,
        focused: bool,
        now: f64,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.dirty = false;
        self.painted_focused = focused;
        self.painted_phase = self.blink_on(now);
        let world = Size {
            w: f32::from(bounds.size.width) as f64 / scale,
            h: f32::from(bounds.size.height) as f64 / scale,
        };
        let bg = hex(&self.colors.background);
        window.paint_quad(fill(bounds, bg));
        let font_size = px((self.metrics.font_px * scale) as f32);
        let line_h = px((self.line_h() * scale) as f32);
        let cell_w = px((self.metrics.cell_w * scale) as f32);
        let legible = font_size >= crate::chrome::legible_font_px(window.scale_factor());
        let s = |v: f64| px((v * scale) as f32);

        // The tree, beside or above.
        if self.tree.is_some() {
            let area = if self.sidebar_top {
                Bounds::new(bounds.origin, size(bounds.size.width, s(self.sidebar_w)))
            } else {
                Bounds::new(bounds.origin, size(s(self.sidebar_w), bounds.size.height))
            };
            if legible {
                self.paint_tree(area, scale, focused, window, cx);
            }
        }
        let (t_origin, t_size) = self.text_area(world);
        let area = Bounds::new(
            point(
                bounds.origin.x + s(t_origin.x),
                bounds.origin.y + s(t_origin.y),
            ),
            size(s(t_size.w), s(t_size.h)),
        );
        if let Some(path) = self.image.clone() {
            self.world = world;
            self.paint_image(&path, area, scale, window, cx);
            return;
        }
        if self.search.is_some() && legible {
            let panel = Bounds::new(
                point(area.origin.x, area.origin.y - s(self.panel_h())),
                size(area.size.width, s(self.panel_h())),
            );
            self.paint_panel(panel, scale, focused, window, cx);
        }
        let rows_visible = ((t_size.h - PAD_Y * 2.) / self.line_h()).floor().max(1.) as usize;
        self.set_rows_visible(rows_visible);
        self.world = world;
        let gutter_w = s(self.gutter_w());
        let origin = point(area.origin.x + s(PAD_X), area.origin.y + s(PAD_Y));
        let text_x = origin.x + gutter_w - s(self.scroll_x);
        let fg = hex(&self.colors.foreground);
        let gutter_fg = hex(&self.colors.gutter);
        let sel_bg = hex(&self.colors.selection);
        let sel_fg = hex(&self.colors.selection_text);
        let cursor_color = hex(&self.colors.cursor);
        let base = self.metrics.font();
        let cols = self.cols_visible(world);
        let vrows = self.visual_rows(cols, rows_visible);
        let first = vrows.first().map(|r| r.line).unwrap_or(0);
        let last = vrows.last().map(|r| r.line + 1).unwrap_or(first);
        let cursor = self.buffer.cursor();
        let cursor_line = self.buffer.line_of(cursor);
        let selection = self.buffer.selection();
        // Every cursor's own selection (Batch 2, 2026-09-25), the primary's
        // included; `selection` above stays the primary's alone for the
        // active-line wash below, which only makes sense for one caret.
        let all_ranges: Vec<_> = self
            .buffer
            .all_selections()
            .iter()
            .filter_map(|(c, a)| a.map(|a| a.min(*c)..a.max(*c)))
            .collect();
        let bracket = self.buffer.matching_bracket();
        let matches: Vec<(usize, usize)> = self
            .search
            .as_ref()
            .map(|s| s.matches.clone())
            .unwrap_or_default();
        let current_match = self.search.as_ref().and_then(|s| s.current_range());
        // The gutter's separator.
        window.paint_quad(fill(
            Bounds::new(
                point(
                    origin.x + gutter_w - s(GUTTER_SEPARATOR_OFFSET_PX),
                    area.origin.y,
                ),
                size(px(crate::chrome::HAIRLINE_PX as f32), area.size.height),
            ),
            crate::chrome::with_alpha(gutter_fg, crate::chrome::HAIRLINE_ALPHA),
        ));
        // Byte offsets of the lines shown, for the spans, which are bytes.
        let spans: Vec<Span> = self.spans_for().to_vec();
        let mut span_i = 0;
        // Runs for the line being drawn, built once per line and sliced
        // per visual row.
        let mut line_runs: Option<(usize, Vec<TextRun>)> = None;
        for (row_i, vrow) in vrows.iter().enumerate() {
            let line_no = vrow.line;
            let y = origin.y + line_h * row_i as f32;
            let line_text = self.buffer.line(line_no);
            let line_start = self.buffer.line_start(line_no);
            let line_len = line_text.chars().count();
            // This row's char range in the buffer.
            let row_start = line_start + vrow.a;
            let row_end = line_start + vrow.b;
            let row_len = vrow.b - vrow.a;
            // The active line wash, gutter, selection, matches, then the text.
            if self.highlight_line
                && line_no == cursor_line
                && selection.is_none()
                && self.buffer.cursor_count() == 1
            {
                window.paint_quad(fill(
                    Bounds::new(point(origin.x + gutter_w, y), size(area.size.width, line_h)),
                    crate::chrome::with_alpha(gpui::rgb(0x808080).into(), ACTIVE_LINE_ALPHA),
                ));
            }
            // A folded block: a chip after the header's text says lines are
            // hidden under it.
            if legible
                && self.buffer.has_folds()
                && vrow.a + row_len == line_len
                && self.buffer.is_fold_header(line_no)
            {
                let chip_x = text_x + cell_w * (row_len as f32 + 1.);
                let chip = Bounds::new(
                    point(chip_x, y + line_h * 0.2),
                    size(cell_w * FOLD_CHIP_CELLS, line_h * 0.6),
                );
                window.paint_quad(fill(chip, crate::chrome::with_alpha(fg, FOLD_CHIP_ALPHA)));
            }
            if legible && vrow.a == 0 {
                let num = (line_no + 1).to_string();
                let l = crate::text::shape(window, &num, font_size, &base, gutter_fg);
                let _ = l.paint(
                    point(origin.x + gutter_w - s(GUTTER_EXTRA_PAD_PX) - l.width, y),
                    line_h,
                    window,
                    cx,
                );
            }
            // A range of buffer chars as a quad on this row.
            let range_quad = |a: usize, b: usize, color: Hsla, window: &mut Window| {
                let a = a.max(row_start).min(row_end) - row_start;
                let b = b.max(row_start).min(row_end + 1) - row_start;
                if b > a {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(text_x + cell_w * a as f32, y),
                            size(cell_w * (b - a) as f32, line_h),
                        ),
                        color,
                    ));
                }
            };
            for (a, b) in &matches {
                if *b > row_start && *a <= row_end {
                    let strong = current_match == Some((*a, *b));
                    range_quad(
                        *a,
                        *b,
                        crate::chrome::with_alpha(
                            sel_bg,
                            if strong { 1. } else { MATCH_DIM_ALPHA },
                        ),
                        window,
                    );
                }
            }
            for sel in &all_ranges {
                if sel.end > row_start && sel.start <= row_end {
                    range_quad(sel.start, sel.end, sel_bg, window);
                }
            }
            if let Some((a, b)) = bracket {
                for i in [a, b] {
                    if i >= row_start && i < row_end {
                        window.paint_quad(
                            outline(
                                Bounds::new(
                                    point(text_x + cell_w * (i - row_start) as f32, y),
                                    size(cell_w, line_h),
                                ),
                                crate::chrome::with_alpha(fg, BRACKET_ALPHA),
                                gpui::BorderStyle::Solid,
                            )
                            .border_widths(px(crate::chrome::HAIRLINE_PX as f32)),
                        );
                    }
                }
            }
            if line_text[..].trim().is_empty() || row_len == 0 {
                continue;
            }
            if !legible {
                // Texture in place of glyphs, as the terminal does.
                let bar_h = (line_h * crate::chrome::TEXTURE_BAR_HEIGHT_RATIO)
                    .max(px(crate::chrome::HAIRLINE_PX as f32));
                let by = y + (line_h - bar_h) / 2.;
                let mut start: Option<usize> = None;
                let chars: Vec<char> = line_text.chars().skip(vrow.a).take(row_len).collect();
                for (i, ch) in chars.iter().enumerate() {
                    match (ch.is_whitespace(), start) {
                        (false, None) => start = Some(i),
                        (true, Some(s)) => {
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(text_x + cell_w * s as f32, by),
                                    size(cell_w * (i - s) as f32, bar_h),
                                ),
                                crate::chrome::with_alpha(fg, crate::chrome::TEXTURE_BAR_ALPHA),
                            ));
                            start = None;
                        }
                        _ => {}
                    }
                }
                if let Some(s) = start {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(text_x + cell_w * s as f32, by),
                            size(cell_w * (chars.len() - s) as f32, bar_h),
                        ),
                        crate::chrome::with_alpha(fg, crate::chrome::TEXTURE_BAR_ALPHA),
                    ));
                }
                continue;
            }
            // Runs from the spans that fall inside this line.
            if line_runs.as_ref().map(|(l, _)| *l) != Some(line_no) {
                // From the rope. This used to come from a byte scan of the
                // whole text that stopped at the last line it had collected,
                // and fell back to 0 for anything past it: a line scrolled
                // into view beyond that point was highlighted with
                // whole-buffer span offsets measured from the file's start,
                // which made runs longer than the line, which made gpui
                // slice past the end of the string and ABORT the app.
                let byte_start = self.buffer.line_byte_start(line_no);
                let byte_end = byte_start + line_text.len();
                while span_i < spans.len() && spans[span_i].end <= byte_start {
                    span_i += 1;
                }
                let mut runs: Vec<TextRun> = vec![];
                let mut pos = 0; // byte offset within the line
                let mut j = span_i;
                let push = |runs: &mut Vec<TextRun>, len: usize, color: Hsla, italic: bool| {
                    if len == 0 {
                        return;
                    }
                    let mut f = base.clone();
                    if italic {
                        f.style = FontStyle::Italic;
                    }
                    runs.push(TextRun {
                        len,
                        font: f,
                        color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    });
                };
                while j < spans.len() && spans[j].start < byte_end {
                    let sp = &spans[j];
                    let a = sp.start.max(byte_start) - byte_start;
                    let b = sp.end.min(byte_end) - byte_start;
                    if a > pos {
                        push(&mut runs, a - pos, fg, false);
                        pos = a;
                    }
                    if b > pos {
                        let (color, italic) = match self.rule_for(sp.capture) {
                            Some(r) => (hex(&r.color), r.italic),
                            None => (fg, false),
                        };
                        push(&mut runs, b - pos, color, italic);
                        pos = b;
                    }
                    j += 1;
                }
                if pos < line_text.len() {
                    push(&mut runs, line_text.len() - pos, fg, false);
                }
                // gpui indexes the string BY these lengths and panics if they
                // do not add up, which aborts the process rather than drawing
                // a line wrong. Nothing that only paints should be able to do
                // that, so the invariant is enforced here rather than hoped
                // for: see `fit_runs`.
                fit_runs(&mut runs, &line_text);
                // Selected text takes the selection colour, split at the edges.
                if let Some(sel) = &selection {
                    if sel.end > line_start && sel.start < line_start + line_len {
                        let a = sel.start.max(line_start) - line_start;
                        let b = sel.end.min(line_start + line_len) - line_start;
                        let (ba, bb) = (char_to_byte(&line_text, a), char_to_byte(&line_text, b));
                        runs = recolor(runs, ba, bb, sel_fg);
                    }
                }
                line_runs = Some((line_no, runs));
            }
            let runs = &line_runs.as_ref().unwrap().1;
            let (ba, bb) = (
                char_to_byte(&line_text, vrow.a),
                char_to_byte(&line_text, vrow.b),
            );
            let shown = &line_text[ba..bb];
            let row_runs = slice_runs(runs, ba, bb);
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            use std::hash::{Hash, Hasher};
            shown.hash(&mut hasher);
            f32::from(font_size).to_bits().hash(&mut hasher);
            for r in &row_runs {
                (
                    r.len,
                    r.color.h.to_bits(),
                    r.color.s.to_bits(),
                    r.color.l.to_bits(),
                    r.font.style == FontStyle::Italic,
                )
                    .hash(&mut hasher);
            }
            let key = hasher.finish();
            let cache_key = (line_no, vrow.a);
            let shaped = match self.shaped.get(&cache_key) {
                Some((k, line)) if *k == key => line.clone(),
                _ => {
                    let line = window.text_system().shape_line(
                        SharedString::from(shown.to_string()),
                        font_size,
                        &row_runs,
                        None,
                    );
                    self.shaped.insert(cache_key, (key, line.clone()));
                    line
                }
            };
            let _ = shaped.paint(point(text_x, y), line_h, window, cx);
        }
        // The cursor: a block 0.6 em wide in the cursor colour, on when
        // focused and the blink says so; hollow when the card is not
        // focused. One per cursor (Batch 2, 2026-09-25); `painted_caret`,
        // the IME candidate window's anchor, stays the PRIMARY's alone —
        // an input method has one caret to sit beside, not several.
        self.painted_caret = None;
        if self.focus != Focus::Tree {
            let primary = self.buffer.cursor();
            for (pos, _) in self.buffer.all_selections() {
                let line = self.buffer.line_of(pos);
                let col = self.buffer.col_of(pos);
                let Some(row_i) = vrows.iter().position(|r| {
                    r.line == line
                        && col >= r.a
                        && (col < r.b
                            || (col == r.b && r.b == self.buffer.line(r.line).chars().count()))
                }) else {
                    continue;
                };
                let vcol = col - vrows[row_i].a;
                let y = origin.y + line_h * row_i as f32;
                let rect = Bounds::new(
                    point(text_x + cell_w * vcol as f32, y),
                    size(
                        px((self.metrics.font_px * CURSOR_BLOCK_WIDTH_RATIO * scale) as f32)
                            .max(px(crate::chrome::HAIRLINE_PX as f32)),
                        line_h,
                    ),
                );
                if pos == primary {
                    // The cell the caret is in, for the input method's
                    // candidate window and the marked text drawn over it
                    // (`caret_bounds`).
                    self.painted_caret = Some(Bounds::new(
                        point(text_x + cell_w * vcol as f32, y),
                        size(cell_w, line_h),
                    ));
                }
                if focused && self.focus == Focus::Buffer {
                    if self.painted_phase {
                        window.paint_quad(fill(
                            rect,
                            crate::chrome::with_alpha(cursor_color, CURSOR_ALPHA),
                        ));
                    }
                } else {
                    window.paint_quad(
                        outline(
                            rect,
                            crate::chrome::with_alpha(cursor_color, CURSOR_ALPHA),
                            gpui::BorderStyle::Solid,
                        )
                        .border_widths(px((scale as f32).max(crate::chrome::HAIRLINE_PX as f32))),
                    );
                }
            }
        }
        // Keep the shaping cache to the visible lines.
        self.shaped.retain(|(l, _), _| *l >= first && *l < last);
    }

    fn caret_bounds(&self) -> Option<Bounds<Pixels>> {
        self.painted_caret
    }

    fn key(&mut self, k: &Keystroke, now: f64, cx: &mut App) -> BodyAction {
        self.key_action(k, now, cx)
    }

    fn insert_text(&mut self, text: &str) {
        self.insert_composed(text, crate::now_ms());
    }

    fn mouse_down(
        &mut self,
        local: Point,
        button: gpui::MouseButton,
        modifiers: &gpui::Modifiers,
        clicks: usize,
    ) -> BodyAction {
        if button != gpui::MouseButton::Left {
            return BodyAction::None;
        }
        let world = self.world;
        // The tree.
        if let Some(tree) = &self.tree {
            let in_tree = if self.sidebar_top {
                local.y < self.sidebar_w
            } else {
                local.x < self.sidebar_w
            };
            if in_tree {
                let row = ((local.y - PAD_Y) / self.line_h()).floor().max(0.) as usize;
                let index = row + tree.scroll;
                self.tree_focused = true;
                self.focus = Focus::Tree;
                self.dirty = true;
                let action = self.tree.as_mut().unwrap().toggle(index, &mut list_dir);
                return match action {
                    TreeAction::Open(path) => self.open_from_tree(path, crate::now_ms()),
                    _ => BodyAction::None,
                };
            }
        }
        let (t_origin, _) = self.text_area(world);
        if self.search.is_some() && local.y < t_origin.y {
            // The panel: the lower row is replace when it shows.
            let row_h = self.line_h() * SEARCH_ROW_HEIGHT_SCALE;
            let in_replace = self.search.as_ref().is_some_and(|s| s.replacing)
                && local.y - (t_origin.y - self.panel_h()) > row_h;
            self.focus = if in_replace {
                Focus::Replace
            } else {
                Focus::Query
            };
            self.dirty = true;
            return BodyAction::None;
        }
        self.focus = Focus::Buffer;
        self.tree_focused = false;
        let idx = self.index_at(local, world);
        // A click in the line-number gutter folds or opens that line's block.
        if local.x - t_origin.x - PAD_X < self.gutter_w() {
            self.buffer.toggle_fold_line(self.buffer.line_of(idx));
            self.dirty = true;
            return BodyAction::None;
        }
        // A plain click is a reset to one cursor; Cmd+Click manages
        // `extra` itself below and must not be collapsed out from under.
        if !modifiers.platform {
            self.buffer.collapse_to_primary();
        }
        match clicks {
            2 => {
                let r = self.buffer.word_at(idx);
                self.buffer.select_range(r);
            }
            n if n >= 3 => {
                let r = self.buffer.line_range(idx);
                self.buffer.select_range(r);
            }
            _ => {
                if modifiers.platform {
                    // Cmd+Click: a cursor added (or, on an existing one,
                    // removed), not a drag, so the primary's selection is
                    // left alone.
                    self.buffer.add_cursor_at(idx);
                    self.selecting = false;
                } else if modifiers.shift {
                    self.buffer.select_to(idx);
                    self.selecting = true;
                } else {
                    self.buffer.set_cursor(idx);
                    self.selecting = true;
                }
            }
        }
        self.blink_epoch = crate::now_ms();
        self.dirty = true;
        BodyAction::None
    }

    fn mouse_up(
        &mut self,
        _local: Point,
        _button: gpui::MouseButton,
        _modifiers: &gpui::Modifiers,
    ) {
        self.selecting = false;
    }

    fn mouse_move(&mut self, local: Point, _modifiers: &gpui::Modifiers) {
        if self.selecting {
            let world = self.world;
            let idx = self.index_at(local, world);
            self.buffer.select_to(idx);
            self.dirty = true;
        }
    }

    fn wheel(&mut self, _local: Point, dx: f64, dy: f64, _modifiers: &gpui::Modifiers) {
        let lines = self.wheel_carry.lines(dy, self.line_h());
        if lines != 0 {
            let max = self.buffer.line_count().saturating_sub(1);
            self.scroll_line = (self.scroll_line as i64 - lines).clamp(0, max as i64) as usize;
            self.dirty = true;
        }
        if dx.abs() > WHEEL_HORIZONTAL_THRESHOLD && !self.wrap {
            self.scroll_x = (self.scroll_x - dx).max(0.);
            self.dirty = true;
        }
    }

    fn wants_frame(&self, now: f64) -> bool {
        self.dirty
            || (self.painted_focused && self.blink && self.blink_on(now) != self.painted_phase)
    }

    fn captures_drag(&self) -> bool {
        self.selecting
    }

    fn resized(&mut self, world: Size) {
        self.world = world;
        self.dirty = true;
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn char_to_byte(text: &str, ch: usize) -> usize {
    text.char_indices()
        .nth(ch)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
}

/// Makes `runs` describe exactly `text`: the lengths must sum to its byte
/// length and none may split a character.
///
/// gpui's `layout_line` slices the string by these lengths, so a mismatch
/// is `slice_error_fail` and an aborted process, not a misdrawn line. A
/// painter has no business being able to kill the app, so this is a clamp
/// and not an assert: the worst case is one line drawn in the wrong colour.
fn fit_runs(runs: &mut Vec<TextRun>, text: &str) {
    let mut pos = 0usize;
    let mut i = 0;
    while i < runs.len() {
        let mut end = (pos + runs[i].len).min(text.len());
        // A run may not end inside a character.
        while end > pos && !text.is_char_boundary(end) {
            end -= 1;
        }
        runs[i].len = end - pos;
        pos = end;
        if runs[i].len == 0 {
            runs.remove(i);
            continue;
        }
        i += 1;
    }
    if pos < text.len() {
        // Short: the tail keeps the last run's styling, or the default.
        if let Some(last) = runs.last_mut() {
            last.len += text.len() - pos;
        }
    }
}

/// The runs covering bytes `a..b` of a line, cut at the edges.
fn slice_runs(runs: &[TextRun], a: usize, b: usize) -> Vec<TextRun> {
    let mut out = vec![];
    let mut pos = 0;
    for r in runs {
        let (s, e) = (pos, pos + r.len);
        pos = e;
        let from = s.max(a);
        let to = e.min(b);
        if to > from {
            out.push(TextRun {
                len: to - from,
                font: r.font.clone(),
                color: r.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            });
        }
    }
    out
}

/// Splits `runs` so the bytes in `a..b` take `color`.
fn recolor(runs: Vec<TextRun>, a: usize, b: usize, color: Hsla) -> Vec<TextRun> {
    let mut out = vec![];
    let mut pos = 0;
    for r in runs {
        let (s, e) = (pos, pos + r.len);
        pos = e;
        let cut = |from: usize, to: usize, c: Hsla, out: &mut Vec<TextRun>| {
            if to > from {
                out.push(TextRun {
                    len: to - from,
                    font: r.font.clone(),
                    color: c,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                });
            }
        };
        let ia = a.max(s).min(e);
        let ib = b.max(s).min(e);
        cut(s, ia, r.color, &mut out);
        cut(ia, ib, color, &mut out);
        cut(ib, e, r.color, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(len: usize) -> TextRun {
        TextRun {
            len,
            font: gpui::font("Menlo"),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }
    }

    fn total(runs: &[TextRun]) -> usize {
        runs.iter().map(|r| r.len).sum()
    }

    // gpui slices the line BY these lengths and aborts the PROCESS on a
    // mismatch. The crash that prompted this used a byte offset of 0 for a
    // line far into the file, which made the runs enormous.
    #[test]
    fn runs_are_made_to_fit_the_line_they_describe() {
        let text = "let x = 1;";
        let mut too_long = vec![run(4), run(9999)];
        fit_runs(&mut too_long, text);
        assert_eq!(total(&too_long), text.len());

        let mut too_short = vec![run(3)];
        fit_runs(&mut too_short, text);
        assert_eq!(total(&too_short), text.len());

        let mut exact = vec![run(4), run(6)];
        fit_runs(&mut exact, text);
        assert_eq!(exact.len(), 2, "a correct set is left alone");
        assert_eq!(exact[0].len, 4);
    }

    // A run may not end inside a character: `ş` is two bytes, `→` three.
    // Cutting one is the same panic by another route.
    #[test]
    fn a_run_never_splits_a_character() {
        let text = "şey → ok";
        for cut in 1..text.len() {
            let mut runs = vec![run(cut), run(text.len())];
            fit_runs(&mut runs, text);
            assert_eq!(total(&runs), text.len(), "cut {cut}");
            let mut pos = 0;
            for r in &runs {
                pos += r.len;
                assert!(
                    text.is_char_boundary(pos),
                    "cut {cut} left a run ending inside a character"
                );
            }
        }
    }

    #[test]
    fn an_empty_line_takes_no_runs() {
        let mut runs = vec![run(5)];
        fit_runs(&mut runs, "");
        assert!(runs.is_empty());
    }

    fn body() -> EditorBody {
        let metrics = Metrics {
            family: "Menlo".into(),
            font_px: 13.,
            line_height: 1.4,
            cell_w: 8.,
            weight: gpui::FontWeight::NORMAL,
            bold_weight: gpui::FontWeight::BOLD,
        };
        EditorBody::new("c1", None, "/".into(), &metrics, Size { w: 400., h: 300. })
    }

    // Cmd+Alt+Left from the text goes into a tree on the left, Right comes
    // back; at the edge (Left again from the tree) the key is not taken, so
    // it goes on to the next card. A tree on top answers to Up and Down.
    #[test]
    fn cmd_alt_arrows_move_between_text_and_tree_then_leave() {
        use infiniterm_core::navigate::Direction;
        let mut b = body();
        assert!(
            !b.move_focus_within(Direction::Left),
            "no tree: nothing to enter"
        );
        let dir = std::env::temp_dir();
        b.show_tree(dir.to_str().unwrap());
        b.tree_focused = false;
        b.focus = Focus::Buffer;
        assert!(b.move_focus_within(Direction::Left));
        assert_eq!(b.focus, Focus::Tree);
        assert!(!b.move_focus_within(Direction::Left), "the card's edge");
        assert!(!b.move_focus_within(Direction::Up), "not that way");
        assert!(b.move_focus_within(Direction::Right));
        assert_eq!(b.focus, Focus::Buffer);
        b.sidebar_top = true;
        assert!(!b.move_focus_within(Direction::Left));
        assert!(b.move_focus_within(Direction::Up));
        assert_eq!(b.focus, Focus::Tree);
        assert!(b.move_focus_within(Direction::Down));
        assert_eq!(b.focus, Focus::Buffer);
    }

    // On a wrapped line Down goes to the next visual row of the SAME line,
    // and only from the last row to the line below; Up the reverse. The
    // column aimed at survives the rows between.
    #[test]
    fn up_and_down_move_by_visual_row_when_wrapping() {
        let mut b = body();
        b.wrap = true;
        let cols = b.cols_visible(b.world);
        assert!(cols >= 8, "{cols}");
        let word = "ab ";
        let long: String = word.repeat(cols); // three rows and change
        let text = format!("{long}\nshort\n");
        b.buffer = Buffer::new(&text);
        b.buffer.set_cursor(4);
        b.move_visual(1, false);
        let c = b.buffer.cursor();
        assert_eq!(b.buffer.line_of(c), 0, "still the first line");
        assert!(
            b.buffer.col_of(c) > 4 && b.buffer.col_of(c) <= cols + 4,
            "{}",
            b.buffer.col_of(c)
        );
        b.move_visual(-1, false);
        assert_eq!(b.buffer.cursor(), 4, "back where it was");
        // Down through every row lands on the second line.
        let rows = wrap_line(&b.buffer.line(0), cols).len();
        for _ in 0..rows {
            b.move_visual(1, false);
        }
        assert_eq!(b.buffer.line_of(b.buffer.cursor()), 1);
        // Shift+Down selects along the way.
        b.buffer.set_cursor(0);
        b.move_visual(1, true);
        assert!(b.buffer.selection().is_some());
        // Unwrapped, a line is a row.
        b.wrap = false;
        b.buffer.set_cursor(0);
        b.move_visual(1, false);
        assert_eq!(b.buffer.line_of(b.buffer.cursor()), 1);
    }

    // The span cache is keyed by the buffer's version, and every loaded
    // file starts at version 0: the second file opened in a card got the
    // first one's highlighting, or none. Ekin: "the syntax highlighting is
    // not loaded all the times".
    #[test]
    fn each_loaded_file_gets_its_own_highlighting() {
        let dir = std::env::temp_dir().join(format!("ift-hl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rs = dir.join("a.rs");
        let txt = dir.join("b.txt");
        std::fs::write(&rs, "fn main() { let x = 1; }\n").unwrap();
        std::fs::write(&txt, "just words\n").unwrap();
        let mut b = body();
        b.load(rs.to_str().unwrap(), false, 0.);
        assert!(!b.spans_for().is_empty(), "rust is highlighted");
        b.load(txt.to_str().unwrap(), false, 0.);
        assert!(b.spans_for().is_empty(), "plain text is not");
        b.load(rs.to_str().unwrap(), false, 0.);
        assert!(!b.spans_for().is_empty(), "and back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // `ift file:20` jumps before the editor has ever been painted, so it
    // does not know its height yet; the first paint centres the line, with
    // the lines above it on screen, and later paints leave the scroll alone.
    #[test]
    fn a_line_jump_before_the_first_paint_is_centred_by_it() {
        let mut b = body();
        let text: String = (0..500).map(|i| format!("line {i}\n")).collect();
        b.buffer = Buffer::new(&text);
        b.go_to_line(20);
        b.set_rows_visible(30);
        assert_eq!(b.scroll_line, 19 - 15, "line 20 mid-view");
        b.scroll_line = 100;
        b.set_rows_visible(30);
        assert_eq!(b.scroll_line, 100, "only once");
    }

    // Cmd+End in a long file: the view lands with the last line at the
    // bottom, by a walk of one screen, not a quadratic re-sum (which hung
    // for seconds at fifty thousand lines).
    #[test]
    fn scrolling_the_cursor_into_view_walks_one_screen() {
        let mut b = body();
        let text: String = (0..50_000).map(|i| format!("line {i}\n")).collect();
        b.buffer = Buffer::new(&text);
        b.rows_visible = 40;
        let started = std::time::Instant::now();
        b.buffer.move_doc_end(false);
        b.ensure_cursor_visible();
        assert!(
            started.elapsed().as_millis() < 200,
            "{:?}",
            started.elapsed()
        );
        let last = b.buffer.line_of(b.buffer.cursor());
        assert_eq!(b.scroll_line, last + 1 - 40);
        // Back up a little: the view stays, the line is already on screen.
        b.buffer.move_up(false);
        b.ensure_cursor_visible();
        assert_eq!(b.scroll_line, last + 1 - 40);
        // Far up: the view follows to the line.
        b.buffer.go_to_line(10);
        b.ensure_cursor_visible();
        assert_eq!(b.scroll_line, 9);
    }

    // A picture is shown, never read into the buffer, and a save must not
    // write the empty buffer over it: the file is still the picture after.
    #[test]
    fn a_picture_is_never_read_as_text_nor_written_back() {
        let dir = std::env::temp_dir().join(format!(
            "ift-editor-image-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shot.png");
        let bytes = b"\x89PNG\r\n\x1a\nnot really a picture";
        std::fs::write(&path, bytes).unwrap();
        let path = path.to_string_lossy().to_string();
        let mut b = body();
        b.load(&path, false, 0.);
        assert!(b.image.is_some());
        assert_eq!(b.buffer.text(), "");
        assert!(!b.is_dirty());
        b.save(&path, 1.);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);

        // Walking the tree shows each picture the cursor lands on without
        // leaving the tree; a text file waits for Enter.
        std::fs::write(dir.join("a.png"), bytes).unwrap();
        std::fs::write(dir.join("notes.txt"), "hi").unwrap();
        let mut b = body();
        b.show_tree(&dir.to_string_lossy());
        assert_eq!(b.focus, Focus::Tree);
        let down = Keystroke::parse("down").unwrap();
        b.key_tree(&down, 0.); // a.png -> notes.txt
        assert_eq!(b.path, None);
        b.key_tree(&down, 0.); // notes.txt -> shot.png
        assert_eq!(
            b.path.as_deref(),
            Some(dir.join("shot.png").to_str().unwrap())
        );
        assert_eq!(b.focus, Focus::Tree);
        assert!(b.image.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
