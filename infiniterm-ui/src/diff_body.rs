//! The diff card: the working tree against git HEAD, read-only. Port of
//! `DiffCard.svelte` and `DiffTree.svelte`.
//!
//! The editor card's shape with its parts swapped: where the editor has a
//! file tree this has the list of CHANGED files with `+added −removed`
//! beside each (a flat list on purpose: a tree of mostly-empty folders
//! hides the counts), and where the editor has a buffer this has the
//! unified diff of one file (`infiniterm-editor::diff`), deletions and
//! insertions washed in the theme's red and green, unchanged stretches
//! folded to a marker. Nothing here writes; the editor is one Cmd+click
//! away. Cmd+B adds the blame gutter (`git.rs`, `blame.rs`): hash, first
//! name, age in fixed columns, uncommitted lines as `······· you now`.
//!
//! Refreshed by Cmd+K (the same three tree states as the editor) and when
//! the shown file changes on disk, polled every two seconds.
use crate::body::{BodyAction, CardBody};
use crate::editor_body::EditorEvent;
use crate::terminal_body::Metrics;
use gpui::{
    fill, point, px, size, App, Bounds, FontStyle, Hsla, Keystroke, Pixels, SharedString, TextRun,
    Window,
};
use infiniterm_core::blame::{blame_text, BlameLine};
use infiniterm_core::editor_theme::{EditorColors, SyntaxRule};
use infiniterm_core::files::{file_mtime, file_read};
use infiniterm_core::git::BlameLine as GitBlame;
use infiniterm_core::git::{git_blame, git_changes, git_show_head, ChangedFile};
use infiniterm_core::grid::{Point, Size};
use infiniterm_editor::diff::{diff_rows, DiffRow};
use infiniterm_editor::highlight::{Highlighting, Span};
use infiniterm_editor::language::Language;
use std::collections::HashMap;

const PAD_X: f64 = 8.;
const PAD_Y: f64 = 6.;
const DISK_POLL_MS: f64 = 2000.;

/// The blame gutter's width in cells: a short hash, a first name, an age.
const BLAME_GUTTER_COLS: f64 = 22.;
/// The gap between the blame text and the line number that follows it.
const BLAME_GUTTER_GAP_COLS: f64 = 2.;
/// Breathing room beyond the gutter's digits (and blame, when it shows)
/// before the text starts; wider than the editor's since this gutter also
/// carries the change bar.
const DIFF_GUTTER_EXTRA_PAD_PX: f64 = 24.;
/// A line number sits this far left of the gutter's edge, right-aligned.
const DIFF_LINE_NUM_MARGIN_PX: f64 = 16.;
/// Gap between a tree row's file name and its `+added −removed` counts.
const TREE_NAME_COUNT_GAP_PX: f32 = 8.;
/// The change bar sits this far left of the text, clear of the gutter.
const DIFF_BAR_OFFSET_PX: f64 = 6.;
/// The change bar's width.
const DIFF_BAR_WIDTH_PX: f64 = 3.;
/// An added/removed line's background wash: a hint of colour, not a block.
const DIFF_WASH_ALPHA: f32 = 0.18;
/// An uncommitted blame line's colour already marks it; a committed one is
/// dimmed so the code, not the attribution, reads first.
const BLAME_DIM_ALPHA: f32 = 0.85;

pub struct DiffBody {
    pub card_id: String,
    pub root: String,
    repo: String,
    files: Vec<ChangedFile>,
    tree_cursor: usize,
    tree_scroll: usize,
    pub tree_shown: bool,
    pub tree_focused: bool,
    pub sidebar_w: f64,
    pub sidebar_top: bool,
    /// The file shown, relative to the repo.
    shown: Option<String>,
    rows: Vec<DiffRow>,
    current: String,
    language: Option<Language>,
    highlighting: Highlighting,
    spans: Vec<Span>,
    line_byte_starts: Vec<usize>,
    pub blame_on: bool,
    blame: Vec<BlameLine>,
    disk_stamp: Option<u64>,
    last_disk_check: f64,
    scroll: usize,
    rows_visible: usize,
    pub colors: EditorColors,
    pub rules: Vec<SyntaxRule>,
    pub metrics: Metrics,
    pub inactive_dim: f64,
    world: Size,
    dirty: bool,
    /// The wheel's fraction of a line, carried between events.
    wheel_carry: crate::chrome::WheelCarry,
    /// Bumped when the rows change, for the shaping cache.
    version: u64,
    shaped: HashMap<usize, (u64, gpui::ShapedLine)>,
    events: Vec<EditorEvent>,
}

fn hex(s: &str) -> Hsla {
    crate::chrome::hex(s).unwrap_or(gpui::white())
}

impl DiffBody {
    pub fn new(card_id: &str, root: &str, metrics: &Metrics, world: Size) -> DiffBody {
        DiffBody {
            card_id: card_id.to_string(),
            root: root.to_string(),
            repo: String::new(),
            files: vec![],
            tree_cursor: 0,
            tree_scroll: 0,
            tree_shown: false,
            tree_focused: false,
            sidebar_w: 0.,
            sidebar_top: false,
            shown: None,
            rows: vec![],
            current: String::new(),
            language: None,
            highlighting: Highlighting::default(),
            spans: vec![],
            line_byte_starts: vec![],
            blame_on: false,
            blame: vec![],
            disk_stamp: None,
            last_disk_check: 0.,
            scroll: 0,
            rows_visible: 1,
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
            inactive_dim: crate::chrome::INACTIVE_DIM_DEFAULT,
            world,
            dirty: true,
            wheel_carry: crate::chrome::WheelCarry::default(),
            version: 0,
            shaped: HashMap::new(),
            events: vec![],
        }
    }

    pub fn take_events(&mut self) -> Vec<EditorEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.shaped.clear();
    }

    pub fn repo(&self) -> &str {
        &self.repo
    }

    fn line_h(&self) -> f64 {
        self.metrics.font_px * self.metrics.line_height
    }

    /// Re-reads the changed files. Cmd+K, the disk poll, and once at start.
    pub fn refresh(&mut self) {
        match git_changes(&self.root) {
            Ok(changes) => {
                self.repo = changes.repo;
                self.files = changes.files;
                self.tree_cursor = self.tree_cursor.min(self.files.len().saturating_sub(1));
            }
            Err(e) => {
                eprintln!("[infiniterm/warn] git: {e}");
                self.events.push(EditorEvent::Notice(format!(
                    "not a git repository: {}",
                    self.root.rsplit('/').next().unwrap_or(&self.root)
                )));
                self.files.clear();
            }
        }
        self.dirty = true;
    }

    /// Shows one file's diff: HEAD's text as the original, the disk's as
    /// the current. A deleted file is empty now.
    pub fn show(&mut self, rel: &str) {
        if self.repo.is_empty() {
            return;
        }
        let full = format!("{}/{rel}", self.repo.trim_end_matches('/'));
        let original = git_show_head(&self.repo, rel).unwrap_or_default();
        let current = file_read(&full).unwrap_or_default();
        self.disk_stamp = file_mtime(&full);
        self.rows = diff_rows(&original, &current);
        self.language = Language::for_path(&full);
        self.spans = match self.language {
            Some(l) => self.highlighting.spans(l, &current),
            None => vec![],
        };
        self.line_byte_starts = vec![0];
        for (i, b) in current.bytes().enumerate() {
            if b == b'\n' {
                self.line_byte_starts.push(i + 1);
            }
        }
        self.current = current;
        self.shown = Some(rel.to_string());
        self.scroll = 0;
        self.version += 1;
        self.shaped.clear();
        eprintln!(
            "[infiniterm] diff {rel}: {} rows, original {}b, current {}b",
            self.rows.len(),
            original.len(),
            self.current.len()
        );
        if self.blame_on {
            self.load_blame();
        }
        self.events.push(EditorEvent::PathChanged {
            path: full.clone(),
            cwd: full
                .rsplit_once('/')
                .map(|(d, _)| d.to_string())
                .unwrap_or_else(|| "/".into()),
        });
        self.dirty = true;
    }

    fn load_blame(&mut self) {
        let Some(rel) = self.shown.clone() else {
            return;
        };
        // git.rs parses what git said; blame.rs formats it and takes its
        // own record shape (the reference had the same two).
        self.blame = git_blame(&self.repo, &rel)
            .unwrap_or_else(|e| {
                eprintln!("[infiniterm/warn] blame: {e}");
                vec![]
            })
            .into_iter()
            .map(|b: GitBlame| BlameLine {
                line: b.line as usize,
                hash: b.hash,
                author: b.author,
                time: b.time as f64,
                summary: b.summary,
            })
            .collect();
        self.version += 1;
        self.shaped.clear();
    }

    pub fn toggle_blame(&mut self) {
        self.blame_on = !self.blame_on;
        if self.blame_on {
            self.load_blame();
        } else {
            self.blame.clear();
        }
        self.dirty = true;
    }

    /// Cmd+K's three states, as the editor's tree.
    pub fn toggle_tree(&mut self) -> bool {
        if !self.tree_shown {
            self.tree_shown = true;
            self.tree_focused = true;
            self.refresh();
        } else if !self.tree_focused {
            self.tree_focused = true;
            self.refresh();
        } else {
            self.tree_shown = false;
            self.tree_focused = false;
        }
        self.dirty = true;
        self.tree_shown
    }

    pub fn idle(&mut self, now: f64) {
        if now - self.last_disk_check < DISK_POLL_MS {
            return;
        }
        self.last_disk_check = now;
        let Some(rel) = self.shown.clone() else {
            return;
        };
        let full = format!("{}/{rel}", self.repo.trim_end_matches('/'));
        let stamp = file_mtime(&full);
        if stamp == self.disk_stamp {
            return;
        }
        self.disk_stamp = stamp;
        self.refresh();
        self.show(&rel);
    }

    fn open_row(&mut self, index: usize) {
        if let Some(f) = self.files.get(index) {
            let rel = f.path.clone();
            self.tree_cursor = index;
            self.show(&rel);
            self.tree_focused = false;
        }
    }

    fn key_tree(&mut self, k: &Keystroke) {
        let last = self.files.len().saturating_sub(1);
        match k.key.as_str() {
            "down" => self.tree_cursor = (self.tree_cursor + 1).min(last),
            "up" => self.tree_cursor = self.tree_cursor.saturating_sub(1),
            "enter" => self.open_row(self.tree_cursor),
            "escape" => self.tree_focused = false,
            _ => return,
        }
        self.dirty = true;
    }

    fn text_area(&self, world: Size) -> (Point, Size) {
        let (mut x, mut y, mut w, mut h) = (0., 0., world.w, world.h);
        if self.tree_shown {
            if self.sidebar_top {
                y += self.sidebar_w;
                h -= self.sidebar_w;
            } else {
                x += self.sidebar_w;
                w -= self.sidebar_w;
            }
        }
        (Point { x, y }, Size { w, h })
    }

    fn gutter_w(&self) -> f64 {
        let digits = self
            .rows
            .iter()
            .filter_map(DiffRow::line)
            .max()
            .unwrap_or(1)
            .to_string()
            .len()
            .max(3);
        let blame = if self.blame_on {
            BLAME_GUTTER_COLS + BLAME_GUTTER_GAP_COLS
        } else {
            0.
        };
        (digits as f64 + blame) * self.metrics.cell_w + DIFF_GUTTER_EXTRA_PAD_PX
    }

    fn paint_tree(
        &mut self,
        area: Bounds<Pixels>,
        scale: f64,
        focused: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let line_h = px((self.line_h() * scale) as f32);
        let font_size = px((self.metrics.font_px * scale) as f32);
        let f = self.metrics.font();
        let fg = hex(&self.colors.foreground);
        let dim = hex(&self.colors.gutter);
        let sel_bg = hex(&self.colors.selection);
        let sel_fg = hex(&self.colors.selection_text);
        let green = hex(&self.green());
        let red = hex(&self.red());
        let pad = px((PAD_X * scale) as f32);
        let visible = ((f32::from(area.size.height) / f32::from(line_h)).floor() as usize)
            .saturating_sub(1)
            .max(1);
        if self.tree_cursor < self.tree_scroll {
            self.tree_scroll = self.tree_cursor;
        } else if self.tree_cursor >= self.tree_scroll + visible {
            self.tree_scroll = self.tree_cursor + 1 - visible;
        }
        let mut y = area.origin.y + px((PAD_Y * scale) as f32);
        // The totals, as the list's heading.
        let (added, removed) = self
            .files
            .iter()
            .fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
        let head = if self.files.is_empty() {
            "no changes".to_string()
        } else {
            format!("{} changed  +{added} −{removed}", self.files.len())
        };
        let l = crate::text::shape(window, &head, font_size, &f, dim);
        let _ = l.paint(point(area.origin.x + pad, y), line_h, window, cx);
        y += line_h;
        let files = self.files.clone();
        for (i, file) in files
            .iter()
            .enumerate()
            .skip(self.tree_scroll)
            .take(visible)
        {
            let is_cursor = i == self.tree_cursor;
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
            if self.shown.as_deref() == Some(file.path.as_str()) && color == fg {
                color = sel_bg;
            }
            let counts = format!("+{} −{}", file.added, file.removed);
            let c = crate::text::shape(
                window,
                &counts,
                font_size,
                &f,
                if file.status == "deleted" { red } else { green },
            );
            // The name keeps its tail: the file, then as many parents as
            // fit before the counts.
            let avail = area.size.width - pad * 2. - c.width - px(TREE_NAME_COUNT_GAP_PX);
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
            let _ = name.paint(point(area.origin.x + pad, y), line_h, window, cx);
            let _ = c.paint(
                point(area.origin.x + area.size.width - pad - c.width, y),
                line_h,
                window,
                cx,
            );
            y += line_h;
        }
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
            crate::chrome::with_alpha(dim, crate::chrome::HAIRLINE_ALPHA),
        ));
    }

    /// The theme's green and red, for the washes and the counts.
    fn green(&self) -> String {
        self.rules
            .iter()
            .find(|r| r.tag == "string")
            .map(|r| r.color.clone())
            .unwrap_or_else(|| "#a6e3a1".into())
    }

    fn red(&self) -> String {
        self.rules
            .iter()
            .find(|r| r.tag == "operator")
            .map(|r| r.color.clone())
            .unwrap_or_else(|| "#f38ba8".into())
    }
}

impl CardBody for DiffBody {
    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        scale: f64,
        focused: bool,
        _now: f64,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.dirty = false;
        let world = Size {
            w: f32::from(bounds.size.width) as f64 / scale,
            h: f32::from(bounds.size.height) as f64 / scale,
        };
        self.world = world;
        let bg = hex(&self.colors.background);
        window.paint_quad(fill(bounds, bg));
        let s = |v: f64| px((v * scale) as f32);
        let font_size = px((self.metrics.font_px * scale) as f32);
        let line_h = px((self.line_h() * scale) as f32);
        let cell_w = px((self.metrics.cell_w * scale) as f32);
        let legible = font_size >= px(crate::chrome::LEGIBLE_FONT_PX as f32);
        if self.tree_shown {
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
        let rows_visible = ((t_size.h - PAD_Y * 2.) / self.line_h()).floor().max(1.) as usize;
        self.rows_visible = rows_visible;
        let gutter_w = s(self.gutter_w());
        let origin = point(area.origin.x + s(PAD_X), area.origin.y + s(PAD_Y));
        let text_x = origin.x + gutter_w;
        let fg = hex(&self.colors.foreground);
        let gutter_fg = hex(&self.colors.gutter);
        let green = hex(&self.green());
        let red = hex(&self.red());
        let base = self.metrics.font();
        let now_s = crate::now_ms() / 1000.;
        let first = self.scroll.min(self.rows.len().saturating_sub(1));
        let last = (first + rows_visible).min(self.rows.len());
        let rows: Vec<DiffRow> = self.rows[first..last].to_vec();
        let mut span_i = 0;
        for (i, row) in rows.iter().enumerate() {
            let y = origin.y + line_h * i as f32;
            // The wash and the change bar.
            match row {
                DiffRow::Added { .. } => {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(text_x - s(DIFF_BAR_OFFSET_PX), y),
                            size(area.size.width, line_h),
                        ),
                        crate::chrome::with_alpha(green, DIFF_WASH_ALPHA),
                    ));
                    window.paint_quad(fill(
                        Bounds::new(
                            point(text_x - s(DIFF_BAR_OFFSET_PX), y),
                            size(s(DIFF_BAR_WIDTH_PX), line_h),
                        ),
                        green,
                    ));
                }
                DiffRow::Deleted { .. } => {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(text_x - s(DIFF_BAR_OFFSET_PX), y),
                            size(area.size.width, line_h),
                        ),
                        crate::chrome::with_alpha(red, DIFF_WASH_ALPHA),
                    ));
                    window.paint_quad(fill(
                        Bounds::new(
                            point(text_x - s(DIFF_BAR_OFFSET_PX), y),
                            size(s(DIFF_BAR_WIDTH_PX), line_h),
                        ),
                        red,
                    ));
                }
                DiffRow::Collapsed { count } => {
                    let hairline = px(crate::chrome::HAIRLINE_PX as f32);
                    for dy in [px(0.), line_h - hairline] {
                        window.paint_quad(fill(
                            Bounds::new(point(origin.x, y + dy), size(area.size.width, hairline)),
                            crate::chrome::with_alpha(gutter_fg, crate::chrome::HAIRLINE_ALPHA),
                        ));
                    }
                    if legible {
                        let l = crate::text::shape(
                            window,
                            &format!("⋯ {count} unchanged lines"),
                            font_size,
                            &base,
                            gutter_fg,
                        );
                        let _ = l.paint(point(text_x, y), line_h, window, cx);
                    }
                    continue;
                }
                DiffRow::Context { .. } => {}
            }
            if !legible {
                continue;
            }
            // Gutter: blame, then the line number.
            if let Some(line) = row.line() {
                let num = line.to_string();
                let l = crate::text::shape(window, &num, font_size, &base, gutter_fg);
                let _ = l.paint(
                    point(
                        origin.x + gutter_w - s(DIFF_LINE_NUM_MARGIN_PX) - l.width,
                        y,
                    ),
                    line_h,
                    window,
                    cx,
                );
                if self.blame_on {
                    if let Some(b) = self.blame.iter().find(|b| b.line == line) {
                        let uncommitted = b.hash == "0000000";
                        let l = crate::text::shape(
                            window,
                            &blame_text(b, now_s),
                            font_size,
                            &base,
                            if uncommitted {
                                green
                            } else {
                                crate::chrome::with_alpha(gutter_fg, BLAME_DIM_ALPHA)
                            },
                        );
                        let _ = l.paint(point(origin.x, y), line_h, window, cx);
                    }
                }
            }
            let text = row.text();
            if text.trim().is_empty() {
                continue;
            }
            // Runs: syntax for lines of the current file, plain for deleted.
            let mut runs: Vec<TextRun> = vec![];
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
            match row.line() {
                Some(line) if !self.spans.is_empty() => {
                    let byte_start = self.line_byte_starts.get(line - 1).copied().unwrap_or(0);
                    let byte_end = byte_start + text.len();
                    while span_i < self.spans.len() && self.spans[span_i].end <= byte_start {
                        span_i += 1;
                    }
                    let mut pos = 0;
                    let mut j = span_i;
                    while j < self.spans.len() && self.spans[j].start < byte_end {
                        let sp = &self.spans[j];
                        let a = sp.start.max(byte_start) - byte_start;
                        let b = sp.end.min(byte_end) - byte_start;
                        if a > pos {
                            push(&mut runs, a - pos, fg, false);
                            pos = a;
                        }
                        if b > pos {
                            let (color, italic) =
                                match self.rules.iter().find(|r| r.tag == sp.capture) {
                                    Some(r) => (hex(&r.color), r.italic),
                                    None => (fg, false),
                                };
                            push(&mut runs, b - pos, color, italic);
                            pos = b;
                        }
                        j += 1;
                    }
                    if pos < text.len() {
                        push(&mut runs, text.len() - pos, fg, false);
                    }
                }
                _ => push(&mut runs, text.len(), fg, false),
            }
            let key = self.version ^ ((f32::from(font_size).to_bits() as u64) << 32);
            let idx = first + i;
            let shaped = match self.shaped.get(&idx) {
                Some((k, line)) if *k == key => line.clone(),
                _ => {
                    let line = window.text_system().shape_line(
                        SharedString::from(text.to_string()),
                        font_size,
                        &runs,
                        None,
                    );
                    self.shaped.insert(idx, (key, line.clone()));
                    line
                }
            };
            let _ = shaped.paint(point(text_x, y), line_h, window, cx);
        }
        let _ = cell_w;
        self.shaped.retain(|k, _| *k >= first && *k < last);
        if !focused && self.inactive_dim > 0. {
            window.paint_quad(fill(
                bounds,
                crate::chrome::with_alpha(bg, self.inactive_dim as f32),
            ));
        }
    }

    fn resized(&mut self, world: Size) {
        self.world = world;
        self.dirty = true;
    }

    fn key(&mut self, k: &Keystroke, _now: f64, _cx: &mut App) -> BodyAction {
        if k.modifiers.platform || k.modifiers.alt || k.modifiers.control {
            return BodyAction::None;
        }
        if self.tree_focused {
            self.key_tree(k);
            return BodyAction::None;
        }
        let max = self.rows.len().saturating_sub(1);
        match k.key.as_str() {
            "down" => self.scroll = (self.scroll + 1).min(max),
            "up" => self.scroll = self.scroll.saturating_sub(1),
            "pagedown" => self.scroll = (self.scroll + self.rows_visible).min(max),
            "pageup" => self.scroll = self.scroll.saturating_sub(self.rows_visible),
            "home" => self.scroll = 0,
            "end" => self.scroll = max.saturating_sub(self.rows_visible.saturating_sub(1)),
            _ => return BodyAction::None,
        }
        self.dirty = true;
        BodyAction::None
    }

    fn mouse_down(
        &mut self,
        local: Point,
        button: gpui::MouseButton,
        _modifiers: &gpui::Modifiers,
        _clicks: usize,
    ) -> BodyAction {
        if button != gpui::MouseButton::Left {
            return BodyAction::None;
        }
        if self.tree_shown {
            let in_tree = if self.sidebar_top {
                local.y < self.sidebar_w
            } else {
                local.x < self.sidebar_w
            };
            if in_tree {
                let row = ((local.y - PAD_Y) / self.line_h()).floor() as i64 - 1;
                self.tree_focused = true;
                self.dirty = true;
                if row >= 0 {
                    self.open_row(row as usize + self.tree_scroll);
                }
                return BodyAction::None;
            }
        }
        self.tree_focused = false;
        self.dirty = true;
        BodyAction::None
    }

    fn wheel(&mut self, _local: Point, _dx: f64, dy: f64, _modifiers: &gpui::Modifiers) {
        let lines = self.wheel_carry.lines(dy, self.line_h());
        if lines != 0 {
            let max = self.rows.len().saturating_sub(1) as i64;
            self.scroll = (self.scroll as i64 - lines).clamp(0, max) as usize;
            self.dirty = true;
        }
    }

    fn wants_frame(&self, _now: f64) -> bool {
        self.dirty
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
