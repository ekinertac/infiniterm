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
    fill, outline, point, px, size, App, Bounds, ClipboardItem, Corners, Font, FontStyle, Hsla,
    ImageAssetLoader, Keystroke, Pixels, Resource, SharedString, TextRun, Window,
};
use infiniterm_core::complete::{provider_for, Offer, Provider};
use infiniterm_core::editor_theme::{EditorColors, SyntaxRule};
use infiniterm_core::files::{
    dir_list, draft_delete, draft_read, draft_write, file_mtime, file_read, file_write,
};
use infiniterm_core::git::{git_show_head, repo_of};
use infiniterm_core::grid::{Point, Size};
use infiniterm_editor::buffer::Buffer;
use infiniterm_editor::diff::{gutter_marks, GutterMark};
use infiniterm_editor::explorer::{Entry, Tree, TreeAction};
use infiniterm_editor::highlight::{Highlighting, Span};
use infiniterm_editor::jumps::Jumps;
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
/// The status bar under the text, in lines: a line of text and some air,
/// the height VS Code and Zed give theirs.
const STATUS_HEIGHT_LINES: f64 = 1.4;
/// Spaces between the status bar's fields on the right.
const STATUS_FIELD_GAP: &str = "   ";
/// A blink is slower than the terminal's: CodeMirror's 1200 ms period.
const BLINK_MS: f64 = 600.;
const DISK_POLL_MS: f64 = 2000.;
/// A click this many lines from the caret is a leap worth a Ctrl+- back;
/// a click within the screenful is just placing the caret.
const JUMP_CLICK_LINES: usize = 20;
const DRAFT_MS: f64 = 500.;

/// Digits reserved in the gutter before it grows past three: line numbers
/// up to 999 fit without a mid-file resize.
/// Half the width of the tree divider's grab zone, in world px each side of the line.
const DIVIDER_GRAB: f64 = 4.;
/// The divider's width in screen px while it is hovered or held.
const DIVIDER_ACTIVE_PX: f32 = 3.;
const MIN_GUTTER_DIGITS: usize = 3;
/// Breathing room between the gutter's line numbers and the text that
/// follows; also the margin a line number is right-aligned by.
const GUTTER_EXTRA_PAD_PX: f64 = 16.;
/// The offset from the gutter's right edge to its separator line: the same
/// margin as the padding reserved beyond the line numbers.
const GUTTER_SEPARATOR_OFFSET_PX: f64 = 8.;
/// The git gutter's mark: a thin bar in the pad between the line numbers
/// and the separator, which has room to spare (`GUTTER_EXTRA_PAD_PX` is
/// wider than this plus `GUTTER_SEPARATOR_OFFSET_PX`), so it costs no
/// extra gutter width.
const GUTTER_MARK_W_PX: f64 = 3.;
const GUTTER_MARK_INSET_PX: f64 = 2.;
const GUTTER_MARK_ADDED: &str = "#4caf50";
const GUTTER_MARK_MODIFIED: &str = "#e3b341";
const GUTTER_MARK_DELETED: &str = "#f85149";
/// The deleted-lines marker's height: a notch, not a full row.
const GUTTER_MARK_DELETED_H_PX: f64 = 3.;
/// The completion popup shows this many rows and scrolls past them.
const COMPLETION_ROWS: usize = 8;
/// The completion popup is not offered in a text longer than this: the
/// provider is given the whole text on every key.
const COMPLETION_MAX_CHARS: usize = 512 * 1024;
/// Its width in cells: wide enough for a long key and its default, never
/// wider than this.
const COMPLETION_MAX_COLS: usize = 72;
const COMPLETION_MIN_COLS: usize = 56;
/// The selected row's wash, as an alpha of the selection colour.
const COMPLETION_SELECTED_ALPHA: f32 = 0.45;
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
    /// The tree's divider was dragged to this many world px from the card's
    /// edge: `editors.rs` clamps it into `Card.sidebar` (#258).
    Sidebar(f64),
}

/// The completion popup: what the provider offered and the row picked.
#[derive(Clone)]
struct CompletionPopup {
    offer: Offer,
    selected: usize,
    /// Up or Down moved the row: only then does it stay on its item while
    /// the list narrows; otherwise the best match is always the top row.
    moved: bool,
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
    /// HEAD's text for the current path (`refresh_git_head`); `None`
    /// outside a git repo, or for a file HEAD does not have.
    git_head: Option<String>,
    last_git_check: f64,
    /// The git gutter's marks, cached the same way `spans` is: the
    /// buffer's version and the load count, so switching files does not
    /// paint the previous one's marks for one frame.
    gutter: (u64, u64, Vec<GutterMark>, Vec<usize>),
    /// Where the caret was before it last leapt: Ctrl+- and Ctrl+Shift+-.
    jumps: Jumps,
    /// The file's completion provider (`complete::completer_for`) and the
    /// popup it feeds. `completion_closed_at` is the buffer version at which
    /// Escape or an accept closed it: it stays closed until the text changes.
    completer: Option<Box<dyn Provider>>,
    completion: Option<CompletionPopup>,
    completion_closed_at: Option<u64>,
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
    /// App chrome typography for the editor's own status bar. Buffer text,
    /// search and completion remain on terminal typography.
    pub status_font: Font,
    pub status_font_px: f64,
    pub status_cell_w: f64,
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
    /// The pointer is on the tree's divider, or it is being dragged: the line
    /// thickens and the cursor says it moves.
    divider_hover: bool,
    divider_drag: bool,
    /// The tree row a right-click menu was opened on: lit while the menu is up
    /// and the row its commands act on (#281).
    menu_row: Option<usize>,
    selecting: bool,
    /// The file on disk had Windows line endings, for the status bar.
    crlf: bool,
    /// Selected text in the status bar, as (anchor, end) columns of the
    /// bar's line (`status_line`); `selecting_status` while it is dragged.
    status_sel: Option<(usize, usize)>,
    selecting_status: bool,
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
            git_head: None,
            last_git_check: 0.,
            gutter: (u64::MAX, 0, vec![], vec![]),
            jumps: Jumps::default(),
            completer: None,
            completion: None,
            completion_closed_at: None,
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
            status_font: metrics.font(),
            status_font_px: metrics.font_px,
            status_cell_w: metrics.cell_w,
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
            divider_hover: false,
            divider_drag: false,
            menu_row: None,
            selecting: false,
            crlf: false,
            status_sel: None,
            selecting_status: false,
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
        self.completer = provider_for(path);
        self.completion = None;
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
                self.crlf = text.contains("\r\n");
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
        self.refresh_git_head();
        self.dirty = true;
        self.blink_epoch = now;
    }

    /// Untitled: the draft is all there is.
    pub fn load_untitled(&mut self) {
        if let Ok(Some(draft)) = draft_read(&self.card_id) {
            self.buffer = Buffer::new(&draft);
        }
        self.loads += 1;
        self.git_head = None;
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
            self.completer = provider_for(path);
            self.completion = None;
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
        // The gutter's own poll: HEAD moves on a commit, checkout or
        // stash elsewhere, not on every keystroke, so it is not worth a
        // shell-out that often either. Same cadence as the disk poll, its
        // own timer so the two do not have to land on the same frame.
        if now - self.last_git_check >= DISK_POLL_MS {
            self.last_git_check = now;
            self.refresh_git_head();
            self.dirty = true;
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

    /// The caret is about to leap: remember where it is.
    fn note_jump(&mut self) {
        self.jumps.record(self.buffer.cursor());
    }

    /// Ctrl+- (`forward` false) and Ctrl+Shift+-: back to where the caret
    /// last leapt from, and again forward.
    fn jump_history(&mut self, forward: bool) {
        let here = self.buffer.cursor();
        let to = if forward {
            self.jumps.forward(here)
        } else {
            self.jumps.back(here)
        };
        if let Some(to) = to {
            self.buffer.collapse_to_primary();
            self.buffer.set_cursor(to.min(self.buffer.len_chars()));
            self.centre_cursor();
            self.centre_pending = true;
        }
    }

    pub fn go_to_line(&mut self, line: usize) {
        self.note_jump();
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
            self.note_jump();
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

    /// What a keybinding's `when` can ask of this editor (#269).
    pub fn context_flags(&self) -> infiniterm_core::model::UiContext {
        infiniterm_core::model::UiContext {
            suggest_widget_visible: self.completion.is_some(),
            find_widget_visible: self.search.is_some(),
            editor_has_selection: self.buffer.selection().is_some(),
            ..Default::default()
        }
    }

    /// Whether a single Escape still has something to close or drop: the
    /// completion popup, the find panel, extra cursors, a selection. While it
    /// does, Escape does that and the card stays locked; when it does not,
    /// Escape unlocks the card (`input.rs`), like closing a popup with Esc
    /// and leaving with the next one in any editor (#265).
    pub fn escape_has_work(&self) -> bool {
        self.completion.is_some()
            || self.search.is_some()
            || self.buffer.cursor_count() > 1
            || self.buffer.selection().is_some()
    }

    /// Whether a point in the text area (below any strip) is on the tree
    /// rather than the text, for the lock: clicking the tree must not lock.
    pub fn is_on_tree(&self, local: Point) -> bool {
        // The divider's grab zone counts: grabbing it must not lock the card.
        self.tree.is_some() && self.across(local) < self.sidebar_w + DIVIDER_GRAB
    }

    /// The coordinate across the divider: x for a tree beside the text, y for
    /// one above it.
    fn across(&self, local: Point) -> f64 {
        if self.sidebar_top {
            local.y
        } else {
            local.x
        }
    }

    /// Whether a point is on the line between the tree and the text.
    pub fn on_divider(&self, local: Point) -> bool {
        self.tree.is_some() && (self.across(local) - self.sidebar_w).abs() <= DIVIDER_GRAB
    }

    /// The tree row under a point of the body (below any strip), or `None` off
    /// the tree, on its divider or past its last row.
    pub fn tree_row_at(&self, local: Point) -> Option<usize> {
        let tree = self.tree.as_ref()?;
        if self.across(local) >= self.sidebar_w - DIVIDER_GRAB {
            return None;
        }
        let row = ((local.y - PAD_Y) / self.line_h()).floor();
        if row < 0. {
            return None;
        }
        let index = row as usize + tree.scroll;
        (index < tree.rows().len()).then_some(index)
    }

    /// Lights a tree row (the one a menu is open on) or clears it.
    pub fn set_menu_row(&mut self, row: Option<usize>) {
        if self.menu_row != row {
            self.menu_row = row;
            self.dirty = true;
        }
    }

    /// The entry a tree menu command acts on: the row the menu was opened on,
    /// else the tree's cursor row.
    pub fn menu_entry(&self) -> Option<infiniterm_editor::explorer::Entry> {
        let tree = self.tree.as_ref()?;
        tree.rows()
            .get(self.menu_row.unwrap_or(tree.cursor))
            .map(|r| r.entry.clone())
    }

    /// The tree's root folder, for a relative path.
    pub fn tree_root(&self) -> Option<&str> {
        self.tree.as_ref().map(|t| t.root.as_str())
    }

    /// The menu's Open: a file in a tab, a folder opened or closed.
    pub fn tree_open_entry(&mut self) {
        let Some(entry) = self.menu_entry() else {
            return;
        };
        if entry.is_dir {
            let index = self.menu_row.or(self.tree.as_ref().map(|t| t.cursor));
            if let (Some(i), Some(tree)) = (index, self.tree.as_mut()) {
                tree.toggle(i, &mut list_dir);
                self.dirty = true;
            }
        } else {
            self.open_from_tree(entry.path, crate::now_ms());
        }
    }

    /// The cursor for the divider while it is under the pointer or held.
    pub fn divider_cursor(&self) -> Option<gpui::CursorStyle> {
        let held = self.divider_hover || self.divider_drag;
        let style = if self.sidebar_top {
            gpui::CursorStyle::ResizeUpDown
        } else {
            gpui::CursorStyle::ResizeLeftRight
        };
        held.then_some(style)
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
        if self.image.is_none() {
            h -= self.status_h();
        }
        (Point { x, y }, Size { w, h })
    }

    fn status_h(&self) -> f64 {
        self.status_font_px * self.metrics.line_height * STATUS_HEIGHT_LINES
    }

    /// The facts the status bar reports (`infiniterm_editor::status`).
    fn status(&self) -> infiniterm_editor::status::Status {
        let cursor = self.buffer.cursor();
        let selected = self
            .buffer
            .all_selections()
            .iter()
            .filter_map(|(c, a)| a.map(|a| a.abs_diff(*c)))
            .sum();
        infiniterm_editor::status::Status {
            path: self.path.clone(),
            line: self.buffer.line_of(cursor) + 1,
            col: self.buffer.col_of(cursor) + 1,
            language: self.language,
            selected,
            cursors: self.buffer.cursor_count(),
            indent: infiniterm_editor::status::Indent::Spaces2,
            crlf: self.crlf,
            read_only: self.read_only,
        }
    }

    /// The bar as one line of `cols` monospace cells: the path on the left,
    /// the fields on the right, the path cut from its front with an ellipsis
    /// when both do not fit. One line so a selection is a range of columns.
    fn status_line(&self, cols: usize) -> Vec<char> {
        let s = self.status();
        let home = std::env::var("HOME").unwrap_or_default();
        let left: Vec<char> = infiniterm_editor::status::left(&s, home.trim_end_matches('/'))
            .chars()
            .collect();
        let right: Vec<char> = infiniterm_editor::status::right(&s)
            .join(STATUS_FIELD_GAP)
            .chars()
            .collect();
        let room = cols.saturating_sub(right.len() + STATUS_FIELD_GAP.len());
        let left: Vec<char> = if left.len() > room {
            let keep = room.saturating_sub(1);
            std::iter::once('…')
                .chain(left[left.len() - keep..].iter().copied())
                .take(room)
                .collect()
        } else {
            left
        };
        let mut line = left;
        let pad = cols.saturating_sub(line.len() + right.len());
        line.extend(std::iter::repeat_n(' ', pad));
        line.extend(right);
        line
    }

    fn status_cols(&self, world: Size) -> usize {
        let (_, size) = self.text_area(world);
        ((size.w - PAD_X * 2.) / self.status_cell_w).floor().max(0.) as usize
    }

    /// The column of the bar under `local`, or `None` when it is not on the bar.
    fn status_col_at(&self, local: Point, world: Size) -> Option<usize> {
        if self.image.is_some() {
            return None;
        }
        let (o, size) = self.text_area(world);
        let top = o.y + size.h;
        if local.y < top || local.y > top + self.status_h() || local.x < o.x {
            return None;
        }
        Some(
            ((local.x - o.x - PAD_X) / self.status_cell_w)
                .floor()
                .max(0.) as usize,
        )
    }

    /// The selected text of the bar, trimmed, when there is any.
    fn status_selection(&self) -> Option<String> {
        let (a, b) = self.status_sel?;
        let (a, b) = (a.min(b), a.max(b));
        let line = self.status_line(self.status_cols(self.world));
        let text: String = line.get(a..b.min(line.len()))?.iter().collect();
        let text = text.trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    /// The completion popup under the caret (above it when there is no
    /// room), with the selected item's description in a last row.
    fn paint_completion(
        &self,
        area: Bounds<Pixels>,
        scale: f64,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (Some(popup), Some(caret)) = (self.completion.as_ref(), self.painted_caret) else {
            return;
        };
        let font_size = px((self.metrics.font_px * scale) as f32);
        let line_h = px((self.line_h() * scale) as f32);
        let cell_w = px((self.metrics.cell_w * scale) as f32);
        let base = self.metrics.font();
        let fg = hex(&self.colors.foreground);
        let dim = hex(&self.colors.gutter);
        let items = &popup.offer.items;
        let rows = items.len().min(COMPLETION_ROWS);
        let first = (popup.selected + 1).saturating_sub(rows);
        let want = items
            .iter()
            .map(|c| c.label.chars().count() + 2 + c.detail.chars().count().min(24))
            .max()
            .unwrap_or(0)
            + 2;
        let cols = want.clamp(COMPLETION_MIN_COLS, COMPLETION_MAX_COLS);
        let width = cell_w * cols as f32;
        let selected = &items[popup.selected.min(items.len() - 1)];
        let has_doc = !selected.doc.is_empty();
        let height = line_h * (rows + usize::from(has_doc)) as f32;
        // Left edge at the start of the text being replaced.
        let mut x = caret.origin.x - cell_w * popup.offer.typed as f32;
        let right = area.origin.x + area.size.width;
        if x + width > right {
            x = right - width;
        }
        x = x.max(area.origin.x);
        let mut y = caret.origin.y + line_h;
        if y + height > area.origin.y + area.size.height {
            y = (caret.origin.y - height).max(area.origin.y);
        }
        let panel = Bounds::new(point(x, y), size(width, height));
        window.paint_quad(fill(panel, hex(&self.colors.background)));
        window.paint_quad(
            outline(
                panel,
                crate::chrome::with_alpha(dim, 0.6),
                gpui::BorderStyle::Solid,
            )
            .border_widths(px(crate::chrome::HAIRLINE_PX as f32)),
        );
        let pad = cell_w;
        let text_cols = cols.saturating_sub(2);
        for (i, item) in items.iter().enumerate().skip(first).take(rows) {
            let row_y = y + line_h * (i - first) as f32;
            if i == popup.selected {
                window.paint_quad(fill(
                    Bounds::new(point(x, row_y), size(width, line_h)),
                    crate::chrome::with_alpha(
                        hex(&self.colors.selection),
                        COMPLETION_SELECTED_ALPHA,
                    ),
                ));
            }
            let label = crate::text::shape(window, &item.label, font_size, &base, fg);
            let _ = label.paint(point(x + pad, row_y), line_h, window, cx);
            let room = text_cols.saturating_sub(item.label.chars().count() + 2);
            let detail: String = item.detail.chars().take(room.min(24)).collect();
            if !detail.is_empty() {
                let d = crate::text::shape(window, &detail, font_size, &base, dim);
                let at = x + pad + cell_w * (item.label.chars().count() + 2) as f32;
                let _ = d.paint(point(at, row_y), line_h, window, cx);
            }
        }
        if has_doc {
            let row_y = y + line_h * rows as f32;
            let text = if selected.doc.chars().count() > text_cols {
                let cut: String = selected
                    .doc
                    .chars()
                    .take(text_cols.saturating_sub(1))
                    .collect();
                format!("{}…", cut.trim_end())
            } else {
                selected.doc.clone()
            };
            let d = crate::text::shape(window, &text, font_size, &base, dim);
            let _ = d.paint(point(x + pad, row_y), line_h, window, cx);
        }
    }

    fn paint_status(&self, bounds: Bounds<Pixels>, scale: f64, window: &mut Window, _cx: &mut App) {
        let s = |v: f64| px((v * scale) as f32);
        let (o, area) = self.text_area(self.world);
        let bar = Bounds::new(
            point(bounds.origin.x + s(o.x), bounds.origin.y + s(o.y + area.h)),
            size(s(area.w), s(self.status_h())),
        );
        // A hairline over it in the gutter's colour: the bar is chrome, not text.
        window.paint_quad(fill(
            Bounds::new(
                bar.origin,
                size(
                    bar.size.width,
                    px((scale as f32).max(crate::chrome::HAIRLINE_PX as f32)),
                ),
            ),
            hex(&self.colors.gutter).opacity(0.35),
        ));
        let line = self.status_line(self.status_cols(self.world));
        let text: String = line.iter().collect();
        let font_size = px((self.status_font_px * scale) as f32);
        let line_h = px((self.status_font_px * self.metrics.line_height * scale) as f32);
        let cell_w = s(self.status_cell_w);
        let x0 = bar.origin.x + s(PAD_X);
        let y0 = bar.origin.y + (bar.size.height - line_h) / 2.;
        if let Some((a, b)) = self.status_sel {
            let (a, b) = (a.min(b), a.max(b).min(line.len()));
            if b > a {
                window.paint_quad(fill(
                    Bounds::new(
                        point(x0 + cell_w * a as f32, y0),
                        size(cell_w * (b - a) as f32, line_h),
                    ),
                    hex(&self.colors.selection),
                ));
            }
        }
        let run = TextRun {
            len: text.len(),
            font: self.status_font.clone(),
            color: hex(&self.colors.gutter),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let slots = fixed_cell_slots(&text);
        let shaped =
            window
                .text_system()
                .shape_line(SharedString::from(text), font_size, &[run], None);
        // A system UI font is proportional, while this bar deliberately
        // lays out in selectable cells. Shape once, then anchor each
        // character in its own cell as the terminal painter does.
        let mut first_x: Vec<Option<Pixels>> = vec![None; line.len()];
        let padding_top = (line_h - shaped.ascent - shaped.descent) / 2.;
        let baseline = y0 + padding_top + shaped.ascent;
        for run in &shaped.runs {
            for glyph in &run.glyphs {
                let Some(&slot) = slots.get(glyph.index) else {
                    continue;
                };
                let anchor = *first_x[slot].get_or_insert(glyph.position.x);
                let at = point(
                    x0 + cell_w * slot as f32 + (glyph.position.x - anchor),
                    baseline + glyph.position.y,
                );
                let _ = if glyph.is_emoji {
                    window.paint_emoji(at, run.font_id, glyph.id, font_size)
                } else {
                    window.paint_glyph(
                        at,
                        run.font_id,
                        glyph.id,
                        font_size,
                        hex(&self.colors.gutter),
                    )
                };
            }
        }
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

    /// Recomputes the completion popup from the text before the caret.
    /// Only for one plain caret in the text; the selected row stays on the
    /// same item while the list narrows around it.
    fn update_completion(&mut self) {
        let Some(provider) = self.completer.as_ref() else {
            self.completion = None;
            return;
        };
        if self.buffer.cursor_count() != 1
            || self.buffer.selection().is_some()
            || self.focus != Focus::Buffer
            || self.completion_closed_at == Some(self.buffer.version)
            || self.buffer.len_chars() > COMPLETION_MAX_CHARS
        {
            self.completion = None;
            return;
        }
        // shortcut: the whole text is copied for every key. Fine for a
        // config file; the cap above is where that stops being true.
        let text = self.buffer.text();
        let cursor = self.buffer.cursor();
        let offer = provider.complete(&text, cursor);
        // A popup is for a word being typed, or a quote that opens a key. After
        // a newline, a space or a comma it offered every key again, so every
        // Enter or space opened one, and the next Enter took a completion
        // instead of making a line (Ekin could not write in settings.json).
        let after_word = cursor > 0
            && text[..text
                .char_indices()
                .nth(cursor)
                .map_or(text.len(), |(i, _)| i)]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '"' | '-'));
        let offer = offer.filter(|_| after_word);
        let kept = self
            .completion
            .as_ref()
            .filter(|p| p.moved)
            .and_then(|p| p.offer.items.get(p.selected))
            .map(|c| c.label.clone());
        let moved = kept.is_some();
        self.completion = offer.map(|offer| {
            let selected = kept
                .and_then(|label| offer.items.iter().position(|c| c.label == label))
                .unwrap_or(0);
            CompletionPopup {
                offer,
                selected,
                moved,
            }
        });
    }

    /// A key for the open popup: Up and Down move, Tab accepts, Enter accepts
    /// a row Up or Down chose (else it closes the popup and stays a newline),
    /// Escape closes. True when the popup took it; nothing is taken with a
    /// modifier held or with no popup open.
    fn complete_key(&mut self, k: &Keystroke, now: f64) -> bool {
        let m = &k.modifiers;
        let Some(popup) = self.completion.as_mut() else {
            return false;
        };
        if m.platform || m.control || m.alt || m.shift {
            return false;
        }
        let n = popup.offer.items.len();
        match k.key.as_str() {
            "down" => {
                popup.selected = (popup.selected + 1) % n;
                popup.moved = true;
            }
            "up" => {
                popup.selected = (popup.selected + n - 1) % n;
                popup.moved = true;
            }
            "tab" => self.accept_completion(now),
            // Enter takes the row only after Up or Down chose it; otherwise it
            // is the newline it always was, and the popup closes (#247).
            "enter" if popup.moved => self.accept_completion(now),
            "enter" => {
                self.completion = None;
                return false;
            }
            "escape" => {
                self.completion = None;
                self.completion_closed_at = Some(self.buffer.version);
            }
            _ => return false,
        }
        true
    }

    /// Replaces what was typed of the key with the selected item.
    fn accept_completion(&mut self, now: f64) {
        let Some(popup) = self.completion.take() else {
            return;
        };
        let Some(item) = popup.offer.items.get(popup.selected) else {
            return;
        };
        let cursor = self.buffer.cursor();
        let from = cursor.saturating_sub(popup.offer.typed);
        self.buffer.replace_range(from..cursor, &item.insert, now);
        self.completion_closed_at = Some(self.buffer.version);
    }

    fn key_buffer(&mut self, k: &Keystroke, now: f64, cx: &mut App) {
        // An open completion popup takes Up, Down, Tab, Enter and Escape.
        if self.complete_key(k, now) {
            self.now_dirty(now);
            return;
        }
        let version_before = self.buffer.version;
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
                    // Text selected in the status bar is the copy, when there is some.
                    let t = self
                        .status_selection()
                        .or_else(|| self.buffer.selected_text())
                        .or_else(|| {
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
                "up" => {
                    self.note_jump();
                    self.buffer.for_each_cursor(|b| b.move_doc_start(shift))
                }
                "down" => {
                    self.note_jump();
                    self.buffer.for_each_cursor(|b| b.move_doc_end(shift))
                }
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
                // Ctrl+- / Ctrl+Shift+-: jump back / forward. By the physical
                // key, since `key` is the shifted glyph on some layouts.
                _ if crate::keycode::last_code() == Some("Minus") => {
                    self.jump_history(crate::keycode::last_shift())
                }
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
                        self.note_jump();
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
        // The popup opens on typing, not on arriving at a spot; once open,
        // every key (an arrow too) re-asks, so it narrows or closes.
        if self.buffer.version != version_before || self.completion.is_some() {
            self.update_completion();
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

    /// Re-reads HEAD's text for this path over git: at load and on the
    /// disk poll's cadence, never per keystroke (a shell-out). `None`
    /// outside a repo, or for a picture, where there is no text to gutter.
    fn refresh_git_head(&mut self) {
        self.git_head = self
            .path
            .as_deref()
            .filter(|_| self.image.is_none())
            .and_then(|path| {
                let repo = repo_of(std::path::Path::new(path)).ok()?;
                let repo = repo.to_string_lossy();
                let rel = path.strip_prefix(&format!("{}/", repo.trim_end_matches('/')))?;
                git_show_head(&repo, rel).ok()
            });
    }

    /// The gutter's marks for the buffer's current text against the HEAD
    /// last fetched, cached like `spans_for`. Empty without a git HEAD to
    /// diff against (no repo, or an untitled buffer).
    fn gutter_for(&mut self) -> (&[GutterMark], &[usize]) {
        if (self.gutter.0, self.gutter.1) != (self.buffer.version, self.loads) {
            let (marks, deleted) = match &self.git_head {
                Some(head) => gutter_marks(head, &self.buffer.text()),
                None => (vec![], vec![]),
            };
            self.gutter = (self.buffer.version, self.loads, marks, deleted);
        }
        (&self.gutter.2, &self.gutter.3)
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
            // The row a right-click menu is open on: a stronger wash and a
            // hairline, so it is clear which file the menu is about.
            if self.menu_row == Some(i) {
                window.paint_quad(fill(row_b, crate::chrome::with_alpha(sel_bg, 0.75)));
                window.paint_quad(
                    gpui::outline(
                        row_b,
                        crate::chrome::with_alpha(fg, 0.55),
                        gpui::BorderStyle::Solid,
                    )
                    .border_widths(px(crate::chrome::HAIRLINE_PX as f32)),
                );
            }
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
        // A hairline between the tree and the text, thicker and stronger while
        // the pointer is on it or it is held, so it reads as something to grab.
        let active = self.divider_hover || self.divider_drag;
        let hairline = if active {
            px(DIVIDER_ACTIVE_PX)
        } else {
            px(crate::chrome::HAIRLINE_PX as f32)
        };
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
            crate::chrome::with_alpha(
                hex(&self.colors.gutter),
                if active {
                    0.9
                } else {
                    crate::chrome::HAIRLINE_ALPHA
                },
            ),
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
        window.paint_quad(crate::chrome::card_body_quad(cx, bounds, bg));
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
        // The text, gutter and carets are cut at the text area's edge: with the
        // file tree beside it, a line scrolled right would be drawn over the tree.
        window.with_content_mask(Some(gpui::ContentMask { bounds: area }), |window| {
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
            let (gutter_marks, gutter_deleted): (Vec<GutterMark>, Vec<usize>) = {
                let (m, d) = self.gutter_for();
                (m.to_vec(), d.to_vec())
            };
            // The line numbers and git marks, outside the text's clip below, so
            // scrolled text never runs under them.
            for (row_i, vrow) in vrows.iter().enumerate() {
                let line_no = vrow.line;
                let y = origin.y + line_h * row_i as f32;
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
                if vrow.a == 0 {
                    let mark_x =
                        origin.x + gutter_w - s(GUTTER_EXTRA_PAD_PX) + s(GUTTER_MARK_INSET_PX);
                    if let Some(mark) = gutter_marks
                        .get(line_no)
                        .filter(|m| **m != GutterMark::Clean)
                    {
                        let color = hex(match mark {
                            GutterMark::Added => GUTTER_MARK_ADDED,
                            GutterMark::Modified => GUTTER_MARK_MODIFIED,
                            GutterMark::Clean => unreachable!(),
                        });
                        window.paint_quad(fill(
                            Bounds::new(point(mark_x, y), size(s(GUTTER_MARK_W_PX), line_h)),
                            color,
                        ));
                    }
                    let deleted_before_this = gutter_deleted.contains(&(line_no + 1))
                        || (line_no == 0 && gutter_deleted.contains(&0));
                    if deleted_before_this {
                        window.paint_quad(fill(
                            Bounds::new(
                                point(mark_x, y),
                                size(s(GUTTER_MARK_W_PX) * 2., s(GUTTER_MARK_DELETED_H_PX)),
                            ),
                            hex(GUTTER_MARK_DELETED),
                        ));
                    }
                }
            }
            // The text, selection and carets are cut at the gutter's right edge:
            // scrolled right, a line starts left of the text area.
            let text_mask = Bounds::new(
                point(origin.x + gutter_w, area.origin.y),
                size(area.size.width - s(PAD_X) - gutter_w, area.size.height),
            );
            window.with_content_mask(Some(gpui::ContentMask { bounds: text_mask }), |window| {
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
                            Bounds::new(
                                point(origin.x + gutter_w, y),
                                size(area.size.width, line_h),
                            ),
                            crate::chrome::with_alpha(
                                gpui::rgb(0x808080).into(),
                                ACTIVE_LINE_ALPHA,
                            ),
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
                        window
                            .paint_quad(fill(chip, crate::chrome::with_alpha(fg, FOLD_CHIP_ALPHA)));
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
                        let chars: Vec<char> =
                            line_text.chars().skip(vrow.a).take(row_len).collect();
                        for (i, ch) in chars.iter().enumerate() {
                            match (ch.is_whitespace(), start) {
                                (false, None) => start = Some(i),
                                (true, Some(s)) => {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(text_x + cell_w * s as f32, by),
                                            size(cell_w * (i - s) as f32, bar_h),
                                        ),
                                        crate::chrome::with_alpha(
                                            fg,
                                            crate::chrome::TEXTURE_BAR_ALPHA,
                                        ),
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
                        let push =
                            |runs: &mut Vec<TextRun>, len: usize, color: Hsla, italic: bool| {
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
                                let (ba, bb) =
                                    (char_to_byte(&line_text, a), char_to_byte(&line_text, b));
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
                                    || (col == r.b
                                        && r.b == self.buffer.line(r.line).chars().count()))
                        }) else {
                            continue;
                        };
                        let vcol = col - vrows[row_i].a;
                        let y = origin.y + line_h * row_i as f32;
                        let rect = Bounds::new(
                            point(text_x + cell_w * vcol as f32, y),
                            size(
                                px((self.metrics.font_px * CURSOR_BLOCK_WIDTH_RATIO * scale)
                                    as f32)
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
                                .border_widths(px(
                                    (scale as f32).max(crate::chrome::HAIRLINE_PX as f32)
                                )),
                            );
                        }
                    }
                }
            });
            // Keep the shaping cache to the visible lines.
            self.shaped.retain(|(l, _), _| *l >= first && *l < last);
            if legible {
                self.paint_completion(area, scale, window, cx);
            }
        });
        if legible {
            // Lines, not rows: a wrapped line takes several rows, so the thumb
            // is a little long on prose (shortcut; exact needs the whole
            // file's wrap, which is only computed for the visible rows).
            crate::scrollbar::paint(
                window,
                area,
                self.buffer.line_count(),
                self.rows_visible,
                self.scroll_line,
                hex(&self.colors.gutter),
            );
            self.paint_status(bounds, scale, window, cx);
        }
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
        // The tree's divider: a press holds it, the moves resize the tree.
        if self.on_divider(local) {
            self.divider_drag = true;
            self.divider_hover = true;
            self.dirty = true;
            return BodyAction::None;
        }
        let world = self.world;
        // The status bar: its text selects like any text, a double-click
        // takes one field whole (the path, "Ln 3, Col 9"), Cmd+C copies it.
        if let Some(col) = self.status_col_at(local, world) {
            self.status_sel = if clicks >= 2 {
                let line = self.status_line(self.status_cols(world));
                Some(field_around(&line, col))
            } else {
                Some((col, col))
            };
            self.selecting_status = clicks < 2;
            self.dirty = true;
            return BodyAction::None;
        }
        if self.status_sel.take().is_some() {
            self.dirty = true;
        }
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
        self.completion = None;
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
                    let far = self
                        .buffer
                        .line_of(idx)
                        .abs_diff(self.buffer.line_of(self.buffer.cursor()));
                    if far >= JUMP_CLICK_LINES {
                        self.note_jump();
                    }
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
        self.selecting_status = false;
        if self.divider_drag {
            self.divider_drag = false;
            self.dirty = true;
        }
    }

    fn mouse_leave(&mut self) {
        if self.divider_hover && !self.divider_drag {
            self.divider_hover = false;
            self.dirty = true;
        }
    }

    fn mouse_move(&mut self, local: Point, _modifiers: &gpui::Modifiers) {
        if self.divider_drag {
            self.events.push(EditorEvent::Sidebar(self.across(local)));
            self.dirty = true;
            return;
        }
        let over = self.on_divider(local);
        if over != self.divider_hover {
            self.divider_hover = over;
            self.dirty = true;
        }
        if self.selecting_status {
            let (o, _) = self.text_area(self.world);
            let col = ((local.x - o.x - PAD_X) / self.status_cell_w)
                .floor()
                .max(0.) as usize;
            if let Some((a, _)) = self.status_sel {
                self.status_sel = Some((a, col));
                self.dirty = true;
            }
            return;
        }
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
        self.selecting || self.selecting_status || self.divider_drag
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

fn fixed_cell_slots(text: &str) -> Vec<usize> {
    let mut slots = vec![0; text.len()];
    for (slot, (start, ch)) in text.char_indices().enumerate() {
        slots[start..start + ch.len_utf8()].fill(slot);
    }
    slots
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

/// The field of the status bar around `col`: the run of text between two
/// gaps of two or more spaces, so a double-click takes "Ln 3, Col 9" or the
/// whole path, never one word of it.
fn field_around(line: &[char], col: usize) -> (usize, usize) {
    if line.is_empty() {
        return (0, 0);
    }
    let col = col.min(line.len() - 1);
    let gap = |i: usize| {
        line[i] == ' ' && (line.get(i + 1) == Some(&' ') || (i > 0 && line[i - 1] == ' '))
    };
    if gap(col) {
        return (col, col);
    }
    let mut a = col;
    while a > 0 && !gap(a - 1) {
        a -= 1;
    }
    let mut b = col;
    while b < line.len() && !gap(b) {
        b += 1;
    }
    (a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_double_click_on_the_status_bar_takes_one_field_whole() {
        let line: Vec<char> = "~/Code/a b.rs      Ln 3, Col 9   UTF-8".chars().collect();
        let field = |col| {
            let (a, b) = field_around(&line, col);
            line[a..b].iter().collect::<String>()
        };
        assert_eq!(
            field(2),
            "~/Code/a b.rs",
            "one space inside a field does not split it"
        );
        assert_eq!(field(22), "Ln 3, Col 9");
        assert_eq!(field(36), "UTF-8");
        assert_eq!(field_around(&line, 15), (15, 15), "a gap selects nothing");
    }

    #[test]
    fn fixed_cell_slots_map_every_byte_to_its_character() {
        assert_eq!(fixed_cell_slots("aé中"), [0, 1, 1, 2, 2, 2]);
    }

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

    // A locked editor unlocks on an Escape that has nothing else to do (#265):
    // while a popup, the find panel, extra cursors or a selection is there,
    // Escape closes or drops that first, as in every editor.
    #[test]
    fn escape_has_work_until_the_popup_the_find_panel_and_the_selection_are_gone() {
        let mut b = body();
        b.buffer = Buffer::new("hello world");
        assert!(!b.escape_has_work(), "a plain caret: Escape leaves");
        b.buffer.select_all();
        assert!(b.escape_has_work(), "a selection is dropped first");
        b.buffer.set_cursor(0);
        assert!(!b.escape_has_work());
        b.buffer.add_cursor_at(6);
        assert!(b.escape_has_work(), "several cursors collapse first");
        b.buffer.collapse_to_primary();
        assert!(!b.escape_has_work());
        b.open_search(false);
        assert!(b.escape_has_work(), "the find panel closes first");
        b.close_search();
        assert!(!b.escape_has_work());
        // the completion popup
        b.completer = settings_provider();
        b.buffer = Buffer::new("{\n  \"ui.fit");
        b.buffer.move_doc_end(false);
        b.update_completion();
        assert!(
            b.completion.is_some() && b.escape_has_work(),
            "the popup closes first"
        );
        assert!(b.complete_key(&key("escape"), 0.));
        assert!(!b.escape_has_work());
    }

    // The tree's divider can be dragged (#258): a press on the line holds it,
    // the moves ask for the width through `EditorEvent::Sidebar`, the release
    // lets go, and hovering it says so (the resize cursor, a thicker line). A
    // press on it is not a click into the text, so it must not lock the card.
    #[test]
    fn the_tree_divider_is_grabbed_dragged_and_released() {
        let mut b = body();
        b.show_tree("/");
        b.sidebar_w = 200.;
        b.sidebar_top = false;
        let mods = gpui::Modifiers::default();
        let at = |x: f64| Point { x, y: 50. };
        assert!(b.on_divider(at(200.)) && b.on_divider(at(203.)) && b.on_divider(at(197.)));
        assert!(!b.on_divider(at(100.)) && !b.on_divider(at(300.)));
        assert!(
            b.is_on_tree(at(203.)),
            "the grab zone does not lock the card"
        );
        assert!(!b.is_on_tree(at(300.)));
        // hover
        b.mouse_move(at(201.), &mods);
        assert_eq!(b.divider_cursor(), Some(gpui::CursorStyle::ResizeLeftRight));
        b.mouse_leave();
        assert_eq!(b.divider_cursor(), None);
        // a press holds it; moves ask for the width, wherever they go
        b.mouse_down(at(200.), gpui::MouseButton::Left, &mods, 1);
        assert!(b.captures_drag());
        b.mouse_move(at(260.), &mods);
        b.mouse_move(at(180.), &mods);
        let events = b.take_events();
        assert_eq!(
            events,
            vec![EditorEvent::Sidebar(260.), EditorEvent::Sidebar(180.)]
        );
        b.mouse_leave();
        assert!(
            b.divider_cursor().is_some(),
            "held, it keeps its cursor off the card"
        );
        b.mouse_up(at(180.), gpui::MouseButton::Left, &mods);
        assert!(!b.captures_drag());
        // a tree above the text drags vertically
        b.sidebar_top = true;
        assert_eq!(b.divider_cursor(), Some(gpui::CursorStyle::ResizeUpDown));
        assert!(b.on_divider(Point { x: 10., y: 199. }));
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

    fn key(name: &str) -> Keystroke {
        Keystroke::parse(name).unwrap()
    }

    fn settings_provider() -> Option<Box<dyn Provider>> {
        let dir = infiniterm_core::paths::config_dir();
        provider_for(&dir.join("settings.json").to_string_lossy())
    }

    #[test]
    fn the_settings_popup_narrows_moves_accepts_and_stays_closed_after_escape() {
        let mut b = body();
        b.completer = settings_provider();
        b.buffer = Buffer::new("{\n  \"ui.");
        b.buffer.move_doc_end(false);
        b.update_completion();
        let n = b
            .completion
            .as_ref()
            .expect("opens after ui.")
            .offer
            .items
            .len();
        assert!(n > 3);
        // Down moves the row, wrapping at the end; the list narrows as you type.
        assert!(b.complete_key(&key("down"), 0.));
        assert_eq!(b.completion.as_ref().unwrap().selected, 1);
        assert!(b.complete_key(&key("up"), 0.));
        assert!(b.complete_key(&key("up"), 0.));
        assert_eq!(b.completion.as_ref().unwrap().selected, n - 1);
        // Moved by hand, the row stays on its item while the list narrows.
        b.buffer.type_char('f', 1.);
        b.update_completion();
        assert!(b.completion.as_ref().unwrap().moved);
        b.buffer.type_char('i', 2.);
        b.buffer.type_char('t', 3.);
        b.buffer.type_char('P', 4.);
        b.update_completion();
        assert_eq!(b.completion.as_ref().unwrap().offer.items.len(), 1);
        // Tab inserts the whole key in place of what was typed, quote kept.
        assert!(b.complete_key(&key("tab"), 5.));
        assert_eq!(b.buffer.text(), "{\n  \"ui.fitPadding");
        assert!(b.completion.is_none());
        // It stays closed until the text changes.
        b.update_completion();
        assert!(b.completion.is_none());
    }

    // Ekin could not write in settings.json (#247): every Enter opened a popup
    // and the next Enter took a completion. A popup is for a word being typed;
    // a newline, a space or a comma opens none, and Enter takes a row only
    // after Up or Down chose it.
    #[test]
    fn a_newline_opens_no_popup_and_enter_is_a_newline_unless_a_row_was_chosen() {
        let mut b = body();
        b.completer = settings_provider();
        b.buffer = Buffer::new("{\n  \"ui.fit");
        b.buffer.move_doc_end(false);
        b.update_completion();
        assert!(b.completion.is_some(), "typing a key opens it");
        // Enter, untouched row: not taken, the popup closes, the key falls through
        assert!(!b.complete_key(&key("enter"), 0.));
        assert!(b.completion.is_none());
        // after a newline, a comma or a space there is nothing to complete
        for tail in ["\n  ", ",\n", " "] {
            let mut b = body();
            b.completer = settings_provider();
            b.buffer = Buffer::new(&format!("{{\n  \"ui.fitPadding\": 1{tail}"));
            b.buffer.move_doc_end(false);
            b.update_completion();
            assert!(b.completion.is_none(), "no popup after {tail:?}");
        }
        // a row chosen with Down is taken by Enter
        let mut b = body();
        b.completer = settings_provider();
        b.buffer = Buffer::new("{\n  \"ui.fit");
        b.buffer.move_doc_end(false);
        b.update_completion();
        assert!(b.complete_key(&key("down"), 0.));
        assert!(b.complete_key(&key("enter"), 1.));
        assert!(b.completion.is_none());
        assert!(
            b.buffer.text().contains("\"ui.fit") && b.buffer.text().len() > "{\n  \"ui.fit".len()
        );
    }

    #[test]
    fn the_best_match_is_the_top_row_until_the_user_moves_it() {
        let mut b = body();
        b.completer = settings_provider();
        b.buffer = Buffer::new("{\"fitp");
        b.buffer.move_doc_end(false);
        b.update_completion();
        let top = b.completion.as_ref().unwrap().offer.items[0].label.clone();
        assert_eq!(top, "ui.fitPadding");
        assert_eq!(b.completion.as_ref().unwrap().selected, 0);
        b.buffer.type_char('a', 1.);
        b.update_completion();
        let p = b.completion.as_ref().unwrap();
        assert_eq!((p.selected, p.moved), (0, false));
    }

    #[test]
    fn escape_closes_the_popup_until_the_next_edit_and_other_keys_pass_through() {
        let mut b = body();
        b.completer = settings_provider();
        b.buffer = Buffer::new("{\"ui.");
        b.buffer.move_doc_end(false);
        b.update_completion();
        assert!(b.completion.is_some());
        // A letter, or a key with a modifier, is not the popup's.
        assert!(!b.complete_key(&key("a"), 0.));
        assert!(!b.complete_key(&key("cmd-down"), 0.));
        assert!(b.complete_key(&key("escape"), 0.));
        assert!(b.completion.is_none());
        b.update_completion();
        assert!(b.completion.is_none(), "closed on purpose");
        b.buffer.type_char('f', 1.);
        b.update_completion();
        assert!(b.completion.is_some(), "an edit opens it again");
        // No popup, no keys taken.
        b.completion = None;
        assert!(!b.complete_key(&key("enter"), 2.));
        // A file with no provider never opens one.
        b.completer = None;
        b.update_completion();
        assert!(b.completion.is_none());
    }

    #[test]
    fn a_go_to_line_can_be_jumped_back_from_and_forward_again() {
        let mut b = body();
        let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
        b.buffer = Buffer::new(&text);
        b.rows_visible = 40;
        b.buffer.go_to_line(5);
        let start = b.buffer.cursor();
        b.go_to_line(150);
        let far = b.buffer.cursor();
        assert_ne!(start, far);
        b.jump_history(false);
        assert_eq!(b.buffer.cursor(), start);
        b.jump_history(true);
        assert_eq!(b.buffer.cursor(), far);
        // Nothing further forward: the caret stays.
        b.jump_history(true);
        assert_eq!(b.buffer.cursor(), far);
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
