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
use crate::body::{BodyAction, CardBody};
use crate::field::{Edit, Field};
use crate::terminal_body::Metrics;
use gpui::{
    fill, outline, point, px, size, App, Bounds, ClipboardItem, FontStyle, Hsla, Keystroke, Pixels,
    SharedString, TextRun, Window,
};
use infiniterm_core::editor_theme::{EditorColors, SyntaxRule};
use infiniterm_core::files::{
    dir_list, draft_delete, draft_read, draft_write, file_mtime, file_read, file_write,
};
use infiniterm_core::grid::{Point, Size};
use infiniterm_core::ift::{open_plan, PathKind};
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
    /// The card's path and directory changed (a file picked from the tree).
    PathChanged {
        path: String,
        cwd: String,
    },
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
    spans: (u64, Vec<Span>),
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
    /// Shaped rows by (line, first char), keyed by what they were shaped from.
    shaped: HashMap<(usize, usize), (u64, gpui::ShapedLine)>,
    /// Pending events for the frame, drained by `take_event`.
    events: Vec<EditorEvent>,
    /// The line a link or `ift file:42` asked for, once the text is in.
    pub pending_line: Option<u64>,
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
            spans: (u64::MAX, vec![]),
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
            painted_focused: false,
            dirty: true,
            wheel_carry: crate::chrome::WheelCarry::default(),
            tree: None,
            tree_focused: false,
            sidebar_w: 0.,
            sidebar_top: false,
            selecting: false,
            rows_visible: 1,
            shaped: HashMap::new(),
            events: vec![],
            pending_line: None,
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
        self.dirty = true;
    }

    /// Writes the buffer to `path`, which the card now carries.
    pub fn save(&mut self, path: &str, now: f64) {
        if self.path.as_deref() != Some(path) {
            self.path = Some(path.to_string());
            self.cwd = parent_of(path);
            self.language = Language::for_path(path);
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
        // Centre it: the cursor row halfway down the visible rows.
        let target = self.buffer.line_of(self.buffer.cursor());
        self.scroll_line = target.saturating_sub(self.rows_visible / 2);
        self.dirty = true;
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
            self.buffer.select_range(a..b);
            self.ensure_cursor_visible();
        }
    }

    fn ensure_cursor_visible(&mut self) {
        let line = self.buffer.line_of(self.buffer.cursor());
        if line < self.scroll_line {
            self.scroll_line = line;
        } else {
            // Scroll until the cursor's line fits within the visible rows,
            // counting the rows wrapped lines take.
            let cols = self.cols_visible(self.world);
            loop {
                let mut rows = 0;
                for l in self.scroll_line..=line {
                    rows += self.rows_of(l, cols);
                }
                if rows <= self.rows_visible || self.scroll_line >= line {
                    break;
                }
                self.scroll_line += 1;
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
    fn open_from_tree(&mut self, path: String, now: f64) -> BodyAction {
        if self.is_dirty() && self.path.is_some() {
            return BodyAction::Open(open_plan(&path, PathKind::File, None));
        }
        let _ = draft_delete(&self.card_id);
        self.load(&path, false, now);
        self.tree_focused = false;
        self.focus = Focus::Buffer;
        self.events.push(EditorEvent::PathChanged {
            path: path.clone(),
            cwd: self.cwd.clone(),
        });
        BodyAction::None
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
                "c" => {
                    if let Some(t) = self.buffer.selected_text() {
                        cx.write_to_clipboard(ClipboardItem::new_string(t));
                    }
                }
                "x" => {
                    if let Some(t) = self.buffer.selected_text() {
                        cx.write_to_clipboard(ClipboardItem::new_string(t));
                        if !ro {
                            self.buffer.backspace(now);
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
                "left" => self.buffer.move_line_start(shift),
                "right" => self.buffer.move_line_end(shift),
                "up" => self.buffer.move_doc_start(shift),
                "down" => self.buffer.move_doc_end(shift),
                "backspace" => {
                    if !ro {
                        self.buffer.delete_to_line_start(now)
                    }
                }
                _ => return,
            }
        } else if m.alt {
            match key {
                "left" => self.buffer.move_word_left(shift),
                "right" => self.buffer.move_word_right(shift),
                "backspace" => {
                    if !ro {
                        self.buffer.delete_word_back(now)
                    }
                }
                "up" => self.buffer.move_up(shift),
                "down" => self.buffer.move_down(shift),
                _ => {
                    // Option+letter types the character macOS composed.
                    if let Some(ch) = k.key_char.as_deref().filter(|c| !ro && !c.is_empty()) {
                        for c in ch.chars() {
                            self.buffer.type_char(c, now);
                        }
                    } else {
                        return;
                    }
                }
            }
        } else if m.control {
            match key {
                "a" => self.buffer.move_line_start(shift),
                "e" => self.buffer.move_line_end(shift),
                _ => return,
            }
        } else {
            match key {
                "left" => self.buffer.move_left(shift),
                "right" => self.buffer.move_right(shift),
                "up" => self.buffer.move_up(shift),
                "down" => self.buffer.move_down(shift),
                "home" => self.buffer.move_line_start(shift),
                "end" => self.buffer.move_line_end(shift),
                "pageup" => self.buffer.move_page(self.rows_visible, false, shift),
                "pagedown" => self.buffer.move_page(self.rows_visible, true, shift),
                "backspace" => {
                    if !ro {
                        self.buffer.backspace(now)
                    }
                }
                "delete" => {
                    if !ro {
                        self.buffer.delete_forward(now)
                    }
                }
                "enter" => {
                    if !ro {
                        self.buffer.newline(now)
                    }
                }
                "tab" if shift => {
                    if !ro {
                        self.buffer.outdent(now)
                    }
                }
                "tab" => {
                    if !ro {
                        self.buffer.tab(now)
                    }
                }
                "escape" => {
                    if self.search.is_some() {
                        self.close_search();
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
                        for c in ch.chars() {
                            self.buffer.type_char(c, now);
                        }
                    } else {
                        return;
                    }
                }
            }
        }
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
        match field.key(k, paste.as_deref()) {
            Edit::Changed => {
                self.refresh_search();
                if self.focus == Focus::Query {
                    self.show_match();
                }
                self.dirty = true;
            }
            Edit::Handled => self.dirty = true,
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
        }
    }

    /// Keys with a possible action for the frame (a file opened beside).
    pub fn key_action(&mut self, k: &Keystroke, now: f64, cx: &mut App) -> BodyAction {
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

    // ----- paint -----

    fn spans_for(&mut self) -> &[Span] {
        if self.spans.0 != self.buffer.version {
            let spans = match self.language {
                Some(l) => self.highlighting.spans(l, &self.buffer.text()),
                None => vec![],
            };
            self.spans = (self.buffer.version, spans);
        }
        &self.spans.1
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
            let text = format!(
                "{}{}{}",
                TREE_INDENT.repeat(row.depth),
                marker,
                row.entry.name
            );
            let line = crate::text::shape(window, &text, font_size, &f, color);
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
            let text = if field.text.is_empty() && !active {
                String::new()
            } else {
                field.text.clone()
            };
            let mut runs_color = fg;
            if field.selected && !field.text.is_empty() {
                let tw = crate::text::shape(window, &text, font_size, &f, fg).width;
                window.paint_quad(fill(
                    Bounds::new(
                        point(
                            field_b.origin.x + px(FIELD_TEXT_PAD_PX as f32),
                            field_b.origin.y + px(SELECTION_BG_INSET_PX as f32),
                        ),
                        size(
                            tw,
                            field_b.size.height - px(SELECTION_BG_INSET_TOTAL_PX as f32),
                        ),
                    ),
                    sel_bg,
                ));
                runs_color = sel_fg;
            }
            let line = crate::text::shape(window, &text, font_size, &f, runs_color);
            let _ = line.paint(
                point(
                    field_b.origin.x + px(FIELD_TEXT_PAD_PX as f32),
                    y + px(FIELD_ROW_INSET_PX as f32),
                ),
                row_h - px(FIELD_ROW_INSET_TOTAL_PX as f32),
                window,
                cx,
            );
            if active && !field.selected {
                window.paint_quad(fill(
                    Bounds::new(
                        point(
                            field_b.origin.x + px(FIELD_TEXT_PAD_PX as f32) + line.width,
                            field_b.origin.y + px(CARET_TOP_INSET_PX as f32),
                        ),
                        size(
                            px(CARET_WIDTH_PX as f32),
                            field_b.size.height - px(CARET_HEIGHT_INSET_PX as f32),
                        ),
                    ),
                    fg,
                ));
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
        let legible = font_size >= px(crate::chrome::LEGIBLE_FONT_PX as f32);
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
        if self.search.is_some() && legible {
            let panel = Bounds::new(
                point(area.origin.x, area.origin.y - s(self.panel_h())),
                size(area.size.width, s(self.panel_h())),
            );
            self.paint_panel(panel, scale, focused, window, cx);
        }
        let rows_visible = ((t_size.h - PAD_Y * 2.) / self.line_h()).floor().max(1.) as usize;
        self.rows_visible = rows_visible;
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
        let cursor_col = self.buffer.col_of(cursor);
        let selection = self.buffer.selection();
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
        let text = self.buffer.text();
        let mut line_byte_starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_byte_starts.push(i + 1);
                if line_byte_starts.len() > last {
                    break;
                }
            }
        }
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
            if self.highlight_line && line_no == cursor_line && selection.is_none() {
                window.paint_quad(fill(
                    Bounds::new(point(origin.x + gutter_w, y), size(area.size.width, line_h)),
                    crate::chrome::with_alpha(gpui::rgb(0x808080).into(), ACTIVE_LINE_ALPHA),
                ));
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
            if let Some(sel) = &selection {
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
                let byte_start = line_byte_starts.get(line_no).copied().unwrap_or(0);
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
        // focused and the blink says so; hollow when the card is not focused.
        let cursor_row = vrows.iter().position(|r| {
            r.line == cursor_line
                && cursor_col >= r.a
                && (cursor_col < r.b
                    || (cursor_col == r.b && r.b == self.buffer.line(r.line).chars().count()))
        });
        if let (true, Some(row_i)) = (self.focus != Focus::Tree, cursor_row) {
            let col = cursor_col - vrows[row_i].a;
            let y = origin.y + line_h * row_i as f32;
            let rect = Bounds::new(
                point(text_x + cell_w * col as f32, y),
                size(
                    px((self.metrics.font_px * CURSOR_BLOCK_WIDTH_RATIO * scale) as f32)
                        .max(px(crate::chrome::HAIRLINE_PX as f32)),
                    line_h,
                ),
            );
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
        // Keep the shaping cache to the visible lines.
        self.shaped.retain(|(l, _), _| *l >= first && *l < last);
    }

    fn key(&mut self, k: &Keystroke, now: f64, cx: &mut App) -> BodyAction {
        self.key_action(k, now, cx)
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
                if modifiers.shift {
                    self.buffer.select_to(idx);
                } else {
                    self.buffer.set_cursor(idx);
                }
                self.selecting = true;
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
