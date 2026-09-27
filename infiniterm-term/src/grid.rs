//! The grid: alacritty's `Term` and VT parser wrapped as a byte sink that
//! hands out frames to paint. From `spikes/canvas/src/terminal.rs`.
//!
//! The reader thread moves bytes into the scheduler; the ui thread calls
//! `advance` with one budget's worth per frame and then `frame` to paint.
//! No lock, no contention, no `alacritty_terminal::event_loop`. Events the
//! terminal raises (a title, a bell, a reply the program is waiting for) are
//! collected here and drained by the body, which owns the pty writer.
//!
//! `Frame` is plain data: rows of runs with resolved colours, the cursor, a
//! mode summary. Everything gpui needs and nothing it does not, so the paint
//! is a loop and the crate stays free of the toolkit.
use crate::palette::Palette;
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::Direction;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionRange, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use alacritty_terminal::term::ClipboardType;
use alacritty_terminal::term::{viewport_to_point, Config, Term, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor};
use std::cell::RefCell;
use std::rc::Rc;

/// What the terminal asked for, drained per frame.
#[derive(Clone, Debug)]
pub enum TermEvent {
    /// The program wants these bytes on its stdin (a reply to a query).
    Write(String),
    Title(String),
    Bell,
    /// OSC 52: the program put text on the clipboard.
    Clipboard(String),
    /// OSC 52 with the "selection" buffer: what the shell's line editor has
    /// selected (our zsh integration reports it on every change, empty
    /// when the selection ends), so Cmd+C can copy it. Not the clipboard.
    LineSelection(String),
}

#[derive(Clone, Default)]
struct Listener(Rc<RefCell<Vec<TermEvent>>>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let mut out = self.0.borrow_mut();
        match event {
            Event::PtyWrite(s) => out.push(TermEvent::Write(s)),
            Event::Title(t) => out.push(TermEvent::Title(t)),
            Event::Bell => out.push(TermEvent::Bell),
            Event::ClipboardStore(ClipboardType::Selection, s) => {
                out.push(TermEvent::LineSelection(s))
            }
            Event::ClipboardStore(_, s) => out.push(TermEvent::Clipboard(s)),
            Event::ColorRequest(index, format) => {
                // Answered from the default palette; a theme change re-answers
                // on the next request, which is how xterm behaves too.
                if let Some(rgb) = Palette::default_rgb(index) {
                    out.push(TermEvent::Write(format(rgb)));
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// The second cell of a wide character, in `Row::text`: one char per cell.
pub const SPACER: char = '\u{200b}';

/// What a click starts: one click drags over cells, two take words, three
/// take lines. The same three as xterm.js and every Mac terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectKind {
    Cells,
    Words,
    Lines,
}

/// One run of cells sharing colours and flags.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub fg: [u8; 3],
    pub bg: Option<[u8; 3]>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
    pub dim: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub runs: Vec<Run>,
    /// The row as text, for links and selection: exactly one character per
    /// cell, which the bar painter, the backgrounds and the link underlines
    /// all count as columns.
    pub text: String,
    /// The zero-width characters alacritty keeps ON a cell (`Cell::
    /// zerowidth`), by column, sparse: U+FE0F asking for the colour emoji
    /// (⚠️ drew as a flat text ⚠ without it), the joiner in a ZWJ emoji
    /// (🏃‍♀️ drew as a runner and a text ♀), combining accents. Beside the
    /// text rather than in it, so a column is still a character everywhere
    /// else; only the shaper reads these (`terminal_body::shape_row`).
    pub zerowidth: Vec<(usize, String)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorKind {
    Block,
    Beam,
    Underline,
    #[default]
    Hidden,
}

/// A move of the visual-mode cursor (`Grid::visual_move`), named for what
/// it does rather than as alacritty's `ViMotion`, so the ui crate needs no
/// alacritty types. Mac keys and vim keys both land here (terminal_body.rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorMove {
    Up,
    Down,
    Left,
    Right,
    WordLeft,
    WordRight,
    WordEnd,
    LineStart,
    LineEnd,
    Top,
    Bottom,
    HalfPageUp,
    HalfPageDown,
}

/// What `v`, `V` and Ctrl+V start in visual mode: vim's three selections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualSelect {
    Cells,
    Lines,
    Block,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub rows: Vec<Row>,
    pub cursor: (usize, usize),
    pub cursor_kind: CursorKind,
    /// How far up the scrollback the view is; 0 at the bottom.
    pub display_offset: usize,
    pub cols: usize,
    /// Whether any row carries selection colours, so clearing the
    /// selection rebuilds them.
    pub selected: bool,
    /// Visual mode is on: `cursor` is the visual cursor, in viewport rows,
    /// and is drawn even while the view is up in the scrollback.
    pub visual: bool,
}

/// How many cells short of the width a row may end and still count as
/// wrapped by the program (`selection_text`): a word-wrap leaves at most
/// the word that did not fit, and twelve covers most English words.
const SOFT_WRAP_SLACK: usize = 12;
/// How many rows above and below a selection are looked at to find the
/// width the program wrapped at: a paragraph's worth.
const WRAP_NEIGHBOURHOOD: i32 = 20;
/// Below this width nothing is taken for a wrapped paragraph.
const WRAP_MIN_WIDTH: usize = 40;

pub struct Grid {
    term: Term<Listener>,
    processor: Processor,
    events: Listener,
    size: Size,
    /// Something outside alacritty's damage tracking changed (the palette,
    /// a resize): the next frame rebuilds every row.
    full_dirty: bool,
    /// A program asked `CSI ? u` and has not left since. See `kitty_keys`.
    kitty_asked: bool,
    /// Find in the terminal (`find`): every match in the scrollback and the
    /// one you are on. `None` when the bar is closed.
    search: Option<Search>,
    /// The last frame drew search colours, so the next rebuilds every row
    /// to clear them.
    search_painted: bool,
    /// Visual mode's selection came from `v` / `V` / Ctrl+V, vim's way, so
    /// plain moves extend it; one made with Shift+move is the Mac's, and a
    /// plain move drops it.
    visual_sticky: bool,
}

/// What `find` found, in order from the top of the scrollback.
struct Search {
    matches: Vec<Match>,
    current: usize,
}

/// Matches counted and highlighted at most: past this a query is too
/// common to be worth stepping through, and collecting them is the cost.
pub const MAX_FIND_MATCHES: usize = 1000;

/// A query as a literal pattern: find is for text, not regular
/// expressions, so `a.b` finds `a.b`.
fn literal(query: &str) -> String {
    let mut out = String::with_capacity(query.len() * 2);
    for c in query.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl Grid {
    pub fn new(cols: usize, rows: usize, scrollback: usize) -> Grid {
        let events = Listener::default();
        let config = Config {
            scrolling_history: scrollback,
            // The kitty keyboard protocol. Off by default in alacritty, and
            // off meant the query (CSI ? u) went unanswered, so a program
            // that asks before pushing never pushed. Claude Code asks, and
            // without an answer Shift+Enter reached it as a bare CR and
            // sent the prompt instead of breaking the line. With it on,
            // alacritty answers, tracks the push and the pop, and
            // `disambiguate_keys` tells the encoder what to send.
            kitty_keyboard: true,
            ..Default::default()
        };
        let size = Size {
            cols: cols.max(2),
            rows: rows.max(1),
        };
        Grid {
            term: Term::new(config, &size, events.clone()),
            processor: Processor::new(),
            events,
            size,
            full_dirty: true,
            kitty_asked: false,
            search: None,
            search_painted: false,
            visual_sticky: false,
        }
    }

    /// Visual mode (Cmd+Shift+C, 2026-09-27): a cursor over the output and
    /// the scrollback, moved by keys, selecting as it goes, for copying
    /// without the mouse. alacritty's vi mode underneath: its cursor, its
    /// motions and its selection, which is what tmux's copy mode and
    /// WezTerm's are too. Nothing reaches the program while it is on.
    pub fn visual(&self) -> bool {
        self.term.mode().contains(TermMode::VI)
    }

    /// Starts at the terminal's own cursor, or the top-left of the view
    /// when that is off screen (alacritty's rule).
    pub fn visual_enter(&mut self) {
        if !self.visual() {
            self.term.toggle_vi_mode();
        }
        self.term.selection = None;
        self.visual_sticky = false;
        self.full_dirty = true;
    }

    /// Escape, and only Escape (Ekin: leave deliberately, a copy does not):
    /// the selection goes and the view returns to the prompt.
    pub fn visual_leave(&mut self) {
        if self.visual() {
            self.term.toggle_vi_mode();
        }
        self.term.selection = None;
        self.visual_sticky = false;
        self.term.scroll_display(Scroll::Bottom);
        self.full_dirty = true;
    }

    /// Moves the visual cursor, keeping it in view. `extend` (Shift held)
    /// grows the selection from where the cursor was, starting one if
    /// there is none; a plain move drops a Shift selection and extends a
    /// `v` one.
    pub fn visual_move(&mut self, m: CursorMove, extend: bool) {
        use alacritty_terminal::vi_mode::ViMotion as V;
        if !self.visual() {
            return;
        }
        if extend || self.visual_sticky {
            if !self.has_selection() {
                self.select_at_cursor(SelectionType::Simple);
            }
        } else {
            self.term.selection = None;
        }
        let half = (self.size.rows / 2).max(1) as i32;
        match m {
            CursorMove::Top => {
                let p = Point::new(self.term.topmost_line(), Column(0));
                self.term.vi_goto_point(p);
            }
            CursorMove::Bottom => {
                let p = Point::new(self.term.bottommost_line(), Column(0));
                self.term.vi_goto_point(p);
            }
            CursorMove::HalfPageUp | CursorMove::HalfPageDown => {
                let lines = if m == CursorMove::HalfPageUp {
                    half
                } else {
                    -half
                };
                let p = self.term.vi_mode_cursor.scroll(&self.term, lines).point;
                self.term.vi_goto_point(p);
            }
            _ => {
                self.term.vi_motion(match m {
                    CursorMove::Up => V::Up,
                    CursorMove::Down => V::Down,
                    CursorMove::Left => V::Left,
                    CursorMove::Right => V::Right,
                    CursorMove::WordLeft => V::SemanticLeft,
                    CursorMove::WordRight => V::SemanticRight,
                    CursorMove::WordEnd => V::SemanticRightEnd,
                    CursorMove::LineStart => V::First,
                    _ => V::Last,
                });
                let p = self.term.vi_mode_cursor.point;
                self.term.scroll_to_point(p);
            }
        }
        self.full_dirty = true;
    }

    /// `v`, `V`, Ctrl+V: vim's toggle. The same kind again ends the
    /// selection; another kind converts it; none starts one at the cursor.
    pub fn visual_select(&mut self, kind: VisualSelect) {
        let ty = match kind {
            VisualSelect::Cells => SelectionType::Simple,
            VisualSelect::Lines => SelectionType::Lines,
            VisualSelect::Block => SelectionType::Block,
        };
        match self.term.selection.as_mut() {
            Some(sel) if self.visual_sticky && sel.ty == ty => {
                self.term.selection = None;
                self.visual_sticky = false;
            }
            Some(sel) => {
                sel.ty = ty;
                self.visual_sticky = true;
            }
            None => {
                self.select_at_cursor(ty);
                self.visual_sticky = true;
            }
        }
        self.full_dirty = true;
    }

    /// A selection covering the cell under the visual cursor. `include_all`
    /// makes it non-empty, without which alacritty never extends it.
    fn select_at_cursor(&mut self, ty: SelectionType) {
        let mut sel = Selection::new(ty, self.term.vi_mode_cursor.point, Side::Left);
        sel.include_all();
        self.term.selection = Some(sel);
    }

    /// Searches the whole scrollback for `query` (smart case: a lowercase
    /// query ignores case, alacritty's rule) and lands on the NEWEST match,
    /// nearest the bottom, since what you look for in a terminal has
    /// usually just scrolled past. Returns (matches, the current one as
    /// 1-based); (0, 0) when nothing matches, and an empty query clears.
    pub fn find(&mut self, query: &str) -> (usize, usize) {
        self.full_dirty = true;
        if query.is_empty() {
            self.search = None;
            return (0, 0);
        }
        let Ok(mut regex) = RegexSearch::new(&literal(query)) else {
            self.search = None;
            return (0, 0);
        };
        let start = Point::new(self.term.topmost_line(), Column(0));
        let end = Point::new(self.term.bottommost_line(), self.term.last_column());
        let matches: Vec<Match> =
            RegexIter::new(start, end, Direction::Right, &self.term, &mut regex)
                .take(MAX_FIND_MATCHES)
                .collect();
        if matches.is_empty() {
            self.search = None;
            return (0, 0);
        }
        let current = matches.len() - 1;
        self.search = Some(Search { matches, current });
        self.show_current()
    }

    /// Steps to the next match: `older` goes up the scrollback, wrapping.
    pub fn find_step(&mut self, older: bool) -> (usize, usize) {
        let Some(s) = self.search.as_mut() else {
            return (0, 0);
        };
        let n = s.matches.len();
        s.current = if older {
            (s.current + n - 1) % n
        } else {
            (s.current + 1) % n
        };
        self.full_dirty = true;
        self.show_current()
    }

    /// The bar closed: the colours go, and the match you were on stays
    /// SELECTED where it is, so Cmd+C copies it (Ekin, 2026-09-26: Escape
    /// used to clear it and jump to the bottom, and there was no way to
    /// copy what you had found). The view stays on it; typing scrolls down
    /// as it always does.
    pub fn find_clear(&mut self) {
        if let Some(s) = self.search.take() {
            let m = &s.matches[s.current];
            let mut sel = Selection::new(SelectionType::Simple, *m.start(), Side::Left);
            sel.update(*m.end(), Side::Right);
            self.term.selection = Some(sel);
            self.full_dirty = true;
        }
    }

    /// The text of the match you are on, for Cmd+C while the bar is open.
    pub fn find_current_text(&self) -> Option<String> {
        let s = self.search.as_ref()?;
        let m = &s.matches[s.current];
        Some(self.term.bounds_to_string(*m.start(), *m.end()))
    }

    /// Scrolls the current match into view; (count, 1-based current).
    fn show_current(&mut self) -> (usize, usize) {
        let Some(s) = self.search.as_ref() else {
            return (0, 0);
        };
        let (n, i, at) = (s.matches.len(), s.current, *s.matches[s.current].start());
        self.term.scroll_to_point(at);
        (n, i + 1)
    }

    pub fn cols(&self) -> usize {
        self.size.cols
    }

    pub fn rows(&self) -> usize {
        self.size.rows
    }

    /// Parses `bytes`; the caller budgets how many per frame.
    pub fn advance(&mut self, bytes: &[u8]) {
        let pasting = self.bracketed_paste();
        self.advance_bytes(bytes);
        if pasting && !self.bracketed_paste() {
            self.kitty_asked = false;
        }
    }

    fn advance_bytes(&mut self, bytes: &[u8]) {
        let history = self.term.history_size();
        self.processor.advance(&mut self.term, bytes);
        // Output pushes lines into history, which moves every match up by
        // as many lines; without this the highlights drifted off their
        // text while a program kept printing. Once the scrollback is full
        // its size stops changing and this cannot tell; the highlights may
        // then drift until the next search.
        if let Some(s) = self.search.as_mut() {
            let grew = self.term.history_size().saturating_sub(history) as i32;
            if grew > 0 {
                let top = self.term.topmost_line();
                let current = s.matches[s.current].clone();
                let shift = |m: &Match| {
                    let (mut a, mut b) = (*m.start(), *m.end());
                    a.line -= grew;
                    b.line -= grew;
                    a..=b
                };
                s.matches = s
                    .matches
                    .iter()
                    .map(shift)
                    .filter(|m| m.start().line >= top)
                    .collect();
                let moved = shift(&current);
                s.current = s.matches.iter().position(|m| *m == moved).unwrap_or(0);
                if s.matches.is_empty() {
                    self.search = None;
                }
                self.full_dirty = true;
            }
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) -> bool {
        let size = Size {
            cols: cols.max(2),
            rows: rows.max(1),
        };
        if size.cols == self.size.cols && size.rows == self.size.rows {
            return false;
        }
        self.size = size;
        self.term.resize(size);
        self.full_dirty = true;
        true
    }

    pub fn take_events(&mut self) -> Vec<TermEvent> {
        let events = std::mem::take(&mut *self.events.0.borrow_mut());
        // alacritty's answer to `CSI ? u` passes through here on its way to
        // the pty. The question is the program's opt-in in practice; see
        // `kitty_keys`.
        if events.iter().any(
            |e| matches!(e, TermEvent::Write(s) if s.starts_with("\x1b[?") && s.ends_with('u')),
        ) {
            self.kitty_asked = true;
        }
        events
    }

    /// A viewport cell as a grid point, which is what a selection is made of:
    /// the grid point stays on its text when the view scrolls, a viewport
    /// cell does not.
    fn point_at(&self, col: usize, row: usize) -> Point {
        let offset = self.term.grid().display_offset();
        viewport_to_point(
            offset,
            Point::new(
                row.min(self.size.rows - 1),
                Column(col.min(self.size.cols - 1)),
            ),
        )
    }

    pub fn start_selection(&mut self, col: usize, row: usize, kind: SelectKind) {
        let ty = match kind {
            SelectKind::Cells => SelectionType::Simple,
            SelectKind::Words => SelectionType::Semantic,
            SelectKind::Lines => SelectionType::Lines,
        };
        self.term.selection = Some(Selection::new(ty, self.point_at(col, row), Side::Left));
    }

    /// The drag end. `right` when the pointer is in the right half of the
    /// cell, so a drag that ends on a character takes it.
    pub fn update_selection(&mut self, col: usize, row: usize, right: bool) {
        let point = self.point_at(col, row);
        if let Some(sel) = self.term.selection.as_mut() {
            sel.update(point, if right { Side::Right } else { Side::Left });
        }
    }

    /// The selection as text, `None` when nothing is selected. Trailing
    /// spaces of each line are dropped, as alacritty and xterm do.
    ///
    /// A row break that a TUI made is joined: Claude Code (Ink) wraps its
    /// own paragraphs and writes every screen row with a hard newline, so
    /// the grid holds no soft-wrap flag and a copy across the wrap came out
    /// broken mid-sentence. Where the row before the break is filled to
    /// within `SOFT_WRAP_SLACK` cells of the width (a word-wrap leaves at
    /// most a word) and the row after continues at the same indent, the
    /// two are one line and the break becomes a space. A short row keeps
    /// its newline, so code and lists keep their lines.
    pub fn selection_text(&self) -> Option<String> {
        let raw = self.term.selection_to_string().filter(|s| !s.is_empty())?;
        let range = self.term.selection.as_ref()?.to_range(&self.term)?;
        if range.is_block || range.start.line == range.end.line {
            return Some(raw);
        }
        let pieces: Vec<&str> = raw.split('\n').collect();
        let lines: Vec<Line> = (range.start.line.0..=range.end.line.0).map(Line).collect();
        if pieces.len() != lines.len() {
            return Some(raw);
        }
        let cols = self.size.cols;
        let extent = |line: Line| -> Option<(usize, usize)> {
            let row = &self.term.grid()[line];
            let first = (0..cols).find(|&c| row[Column(c)].c != ' ')?;
            let last = (0..cols).rev().find(|&c| row[Column(c)].c != ' ')?;
            Some((first, last))
        };
        // The width the program wrapped at is not the grid's: Claude Code
        // wraps at the width it had when it drew, which on a widened or
        // maximised card is far short of the grid (measured: 73-character
        // rows on a 150-column card). The yardstick is the widest row in
        // the selection's neighbourhood, which is that wrap width.
        let grid = self.term.grid();
        let top = grid
            .topmost_line()
            .0
            .max(range.start.line.0 - WRAP_NEIGHBOURHOOD);
        let bottom = grid
            .bottommost_line()
            .0
            .min(range.end.line.0 + WRAP_NEIGHBOURHOOD);
        let widths: Vec<usize> = (top..=bottom)
            .filter_map(|l| extent(Line(l)).map(|(_, last)| last + 1))
            .collect();
        let wrap_width = widths.iter().copied().max().unwrap_or(cols);
        // Nothing wraps prose at less than forty columns; rows that short
        // are a listing, a shell's output, and their breaks are real.
        if wrap_width < WRAP_MIN_WIDTH {
            return Some(raw);
        }
        let slack = SOFT_WRAP_SLACK.min(wrap_width / 4);
        // Wrapping leaves several rows at the width; one long row among
        // short ones ("hello world" over "second") is just the longest
        // line, and its break is real.
        let full_rows = widths
            .iter()
            .filter(|w| **w >= wrap_width.saturating_sub(slack))
            .count();
        if full_rows < 2 {
            return Some(raw);
        }
        let mut out = String::with_capacity(raw.len());
        let mut joined_into = false;
        for (i, piece) in pieces.iter().enumerate() {
            // A row joined onto the one before drops its wrap indent.
            out.push_str(if joined_into {
                piece.trim_start()
            } else {
                piece
            });
            if i + 1 == pieces.len() {
                break;
            }
            joined_into = match (extent(lines[i]), extent(lines[i + 1])) {
                (Some((a_first, a_last)), Some((b_first, _))) => {
                    a_last + 1 >= wrap_width.saturating_sub(slack) && b_first == a_first
                }
                _ => false,
            };
            out.push(if joined_into { ' ' } else { '\n' });
        }
        Some(out)
    }

    pub fn has_selection(&self) -> bool {
        self.term.selection.as_ref().is_some_and(|s| !s.is_empty())
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    /// The palette changed: every resolved colour is stale.
    pub fn set_palette_changed(&mut self) {
        self.full_dirty = true;
    }

    /// Scrolls the view by `lines` (positive is up into history).
    pub fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    /// Whether a program has asked for the mouse (a TUI); then clicks and
    /// wheels are reported to it rather than acted on here.
    pub fn wants_mouse(&self) -> bool {
        self.term.mode().intersects(TermMode::MOUSE_MODE)
    }

    /// The alternate screen with no mouse mode: a wheel becomes arrow keys
    /// (`less`, `man`), which is what every terminal does.
    pub fn alternate_scroll(&self) -> bool {
        let m = self.term.mode();
        m.contains(TermMode::ALT_SCREEN)
            && m.contains(TermMode::ALTERNATE_SCROLL)
            && !m.intersects(TermMode::MOUSE_MODE)
    }

    pub fn sgr_mouse(&self) -> bool {
        self.term.mode().contains(TermMode::SGR_MOUSE)
    }

    pub fn mouse_drag(&self) -> bool {
        self.term.mode().contains(TermMode::MOUSE_DRAG)
    }

    pub fn app_cursor(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    /// Whether the program in this pane speaks the kitty keyboard protocol,
    /// for `keys::encode_with`.
    ///
    /// True while a flag is pushed, as the protocol says, OR while a program
    /// has merely ASKED (`CSI ? u`) and not yet gone away. The second half
    /// is the one that matters: Claude Code asks, is answered, and then
    /// pushes nothing, expecting the terminal to send `CSI 13;2u` for
    /// Shift+Enter on its own, which is what kitty's legacy mode does and
    /// what the other terminals it calls "native" do. A shell never asks.
    ///
    /// "Gone away" is bracketed paste turning off. Claude turns it off on
    /// exit; zsh turns it off on every Enter, so a Claude that crashed
    /// without saying goodbye leaves the shell one wrong Shift+Enter at
    /// most before it heals.
    /// Seeds the flag for a pane whose program announced itself before we
    /// were watching: an adopted session, whose startup is out of the ring.
    /// Only ever turns it ON. The live rules still turn it off when the
    /// program leaves, so a wrong seed costs one keystroke and heals.
    /// The program that asked is gone (its session died with a reboot and
    /// a new shell is starting): Shift+Enter must not send `CSI u` to zsh.
    pub fn forget_kitty_keys(&mut self) {
        self.kitty_asked = false;
    }

    pub fn assume_kitty_keys(&mut self) {
        self.kitty_asked = true;
    }

    pub fn kitty_keys(&self) -> bool {
        self.term.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES) || self.kitty_asked
    }

    pub fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    /// Clears the screen and the scrollback, keeping the cursor line: the
    /// reference's `card.clear`, iTerm2's clear.
    pub fn clear(&mut self) {
        // ESC c would reset modes a running program relies on; this is the
        // scrollback-and-screen clear alacritty binds to Cmd+K.
        self.processor
            .advance(&mut self.term, b"\x1b[H\x1b[2J\x1b[3J");
    }

    /// Everything back to a fresh emulator at the same size: screen,
    /// scrollback and every mode (ESC c). For a re-adopt, whose replay is
    /// the whole ring again and must not land on top of the copy the grid
    /// already holds.
    pub fn reset(&mut self) {
        self.processor.advance(&mut self.term, b"\x1bc\x1b[3J");
        self.kitty_asked = false;
        self.full_dirty = true;
    }

    /// The visible rows with resolved colours, built from scratch. `palette`
    /// decides what every named and indexed colour paints as; a cell's own
    /// RGB stays its own. Tests and one-off callers; the body keeps a
    /// `Frame` and uses `update_frame`.
    pub fn frame(&mut self, palette: &Palette) -> Frame {
        let mut frame = Frame::default();
        self.update_frame(palette, &mut frame);
        frame
    }

    /// Brings `frame` up to date, rebuilding only the rows alacritty marks
    /// damaged since the last call: a prompt redraw touches one row, and
    /// twenty-five idle cards under a zoom touch none. A scroll, a resize, a
    /// selection or a palette change rebuilds everything. Builds from
    /// scratch when `frame` is another size. Returns the rows rebuilt, so
    /// the caller's own per-row work (links) can skip the rest too.
    pub fn update_frame(&mut self, palette: &Palette, frame: &mut Frame) -> Vec<usize> {
        let cols = self.size.cols;
        let rows = self.size.rows;
        let offset = self.term.grid().display_offset();
        let (selection, cursor_shape, cursor_point) = {
            let content = self.term.renderable_content();
            (
                content.selection,
                content.cursor.shape,
                content.cursor.point,
            )
        };
        let fresh = frame.rows.len() != rows || frame.cols != cols;
        let searching = self.search.is_some();
        let full = fresh
            || self.full_dirty
            || selection.is_some()
            || frame.selected
            || searching
            || self.search_painted;
        self.full_dirty = false;
        self.search_painted = searching;
        let damaged: Vec<usize> = if full {
            (0..rows).collect()
        } else {
            match self.term.damage() {
                TermDamage::Full => (0..rows).collect(),
                TermDamage::Partial(lines) => lines.map(|l| l.line).filter(|l| *l < rows).collect(),
            }
        };
        self.term.reset_damage();
        if fresh {
            frame.rows = (0..rows)
                .map(|_| Row {
                    runs: Vec::new(),
                    text: String::with_capacity(cols),
                    zerowidth: Vec::new(),
                })
                .collect();
        }
        frame.selected = selection.is_some();
        for &line in &damaged {
            let mut row = std::mem::take(&mut frame.rows[line]);
            self.build_row(line, offset, selection.as_ref(), palette, &mut row);
            frame.rows[line] = row;
        }
        frame.cursor_kind = match cursor_shape {
            CursorShape::Block => CursorKind::Block,
            CursorShape::Beam => CursorKind::Beam,
            CursorShape::Underline => CursorKind::Underline,
            CursorShape::HollowBlock => CursorKind::Block,
            CursorShape::Hidden => CursorKind::Hidden,
        };
        let Point {
            line: Line(line),
            column: Column(col),
        } = cursor_point;
        frame.visual = self.visual();
        frame.cursor = if frame.visual {
            // The visual cursor can be up in the history: grid line to
            // viewport row. The terminal's own cursor is drawn only at the
            // bottom, where the two agree.
            frame.cursor_kind = CursorKind::Block;
            (col, (line + offset as i32).max(0) as usize)
        } else {
            (col, line.max(0) as usize)
        };
        frame.display_offset = offset;
        frame.cols = cols;
        damaged
    }

    /// One viewport row as runs, into `out` (its buffers reused).
    fn build_row(
        &self,
        line: usize,
        offset: usize,
        selection: Option<&SelectionRange>,
        palette: &Palette,
        out: &mut Row,
    ) {
        out.runs.clear();
        out.text.clear();
        out.zerowidth.clear();
        let grid_line = Line(line as i32 - offset as i32);
        let row = &self.term.grid()[grid_line];
        // The matches touching this row, found once: checking every cell
        // against every match was cells times matches a frame.
        let (row_matches, current_match): (Vec<&Match>, Option<&Match>) = match &self.search {
            Some(s) => (
                s.matches
                    .iter()
                    .filter(|m| m.start().line <= grid_line && grid_line <= m.end().line)
                    .collect(),
                Some(&s.matches[s.current]),
            ),
            None => (vec![], None),
        };
        for col in 0..self.size.cols {
            let cell = &row[Column(col)];
            let flags = cell.flags;
            let point = Point::new(grid_line, Column(col));
            let selected = selection.is_some_and(|s| s.contains(point));
            // Find: every match, and the current one, which wears the
            // selection pair like a selection would.
            let hit = |m: &&Match| m.start() <= &point && &point <= m.end();
            let current = current_match.as_ref().is_some_and(hit);
            let found = current || row_matches.iter().any(hit);
            // The common cell is a blank on the default ground: it joins the
            // run before it, or starts one in the default colour, and
            // nothing is resolved. Most of a screen is this, and a flood of
            // short lines is nearly all of it.
            if cell.c == ' '
                && flags.is_empty()
                && !selected
                && !found
                && matches!(cell.bg, Color::Named(NamedColor::Background))
            {
                out.text.push(' ');
                match out.runs.last_mut() {
                    Some(last) if last.bg.is_none() && !last.underline && !last.strikeout => {
                        last.text.push(' ')
                    }
                    _ => out.runs.push(Run {
                        text: " ".to_string(),
                        fg: palette.foreground,
                        bg: None,
                        bold: false,
                        italic: false,
                        underline: false,
                        strikeout: false,
                        dim: false,
                    }),
                }
                continue;
            }
            let (mut fg, mut bg) = (cell.fg, cell.bg);
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let bold = flags.intersects(Flags::BOLD);
            let mut fg_rgb = palette.resolve(fg, bold);
            let mut bg_rgb = match bg {
                Color::Named(NamedColor::Background) if !flags.contains(Flags::INVERSE) => None,
                other => Some(palette.resolve(other, false)),
            };
            // Selected cells take the app's selection pair, not the theme's:
            // the reference pushes the same pair into every xterm, because a
            // theme's own selection colour is chosen against a prompt and
            // vanishes on a page of anything.
            if selected || current {
                fg_rgb = palette.selection_text;
                bg_rgb = Some(palette.selection);
            } else if found {
                // The theme's yellow, as every terminal marks a match, with
                // the ground as ink so it reads on any theme.
                fg_rgb = palette.background;
                bg_rgb = Some(palette.ansi[3]);
            }
            let run = Run {
                text: String::new(),
                fg: fg_rgb,
                bg: bg_rgb,
                bold,
                italic: flags.intersects(Flags::ITALIC),
                underline: flags.intersects(Flags::ALL_UNDERLINES),
                strikeout: flags.contains(Flags::STRIKEOUT),
                dim: flags.intersects(Flags::DIM),
            };
            // A wide character's second cell keeps its column with a
            // zero-width space, so the cell after it lands where it should;
            // the painter shapes nothing for it.
            let ch = if flags.contains(Flags::WIDE_CHAR_SPACER) {
                SPACER
            } else if flags.contains(Flags::HIDDEN) {
                ' '
            } else {
                cell.c
            };
            if ch == cell.c {
                if let Some(zw) = cell.zerowidth().filter(|zw| !zw.is_empty()) {
                    out.zerowidth.push((col, zw.iter().collect()));
                }
            }
            out.text.push(ch);
            match out.runs.last_mut() {
                Some(last)
                    if last.fg == run.fg
                        && last.bg == run.bg
                        && last.bold == run.bold
                        && last.italic == run.italic
                        && last.underline == run.underline
                        && last.strikeout == run.strikeout
                        && last.dim == run.dim =>
                {
                    last.text.push(ch)
                }
                _ => {
                    let mut run = run;
                    run.text.push(ch);
                    out.runs.push(run);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

    // The shell's selection comes as OSC 52 "s" and is not the clipboard.
    #[test]
    fn an_osc52_selection_is_the_line_selection_not_the_clipboard() {
        let mut g = Grid::new(20, 2, 100);
        g.advance(b"\x1b]52;s;bG8=\x07\x1b]52;c;aGk=\x07");
        let got: Vec<String> = g
            .take_events()
            .into_iter()
            .map(|e| format!("{e:?}"))
            .collect();
        assert_eq!(got, ["LineSelection(\"lo\")", "Clipboard(\"hi\")"]);
    }

    // Visual mode: the cursor walks the output, Shift selects the Mac's
    // way, and Escape puts everything back.
    #[test]
    fn visual_mode_moves_selects_and_leaves() {
        let mut g = Grid::new(20, 3, 100);
        g.advance(b"alpha beta\r\ngamma\r\n$ ");
        g.visual_enter();
        assert!(g.visual());
        let pal = Palette::default_palette();
        assert_eq!(
            g.frame(&pal).cursor,
            (2, 2),
            "starts at the prompt's cursor"
        );
        g.visual_move(CursorMove::Top, false);
        assert_eq!(g.frame(&pal).cursor, (0, 0));
        g.visual_move(CursorMove::WordRight, true);
        g.visual_move(CursorMove::Left, true);
        g.visual_move(CursorMove::Left, true);
        assert_eq!(g.selection_text().as_deref(), Some("alpha"));
        // A plain move drops a Shift selection.
        g.visual_move(CursorMove::Down, false);
        assert!(!g.has_selection());
        g.visual_leave();
        assert!(!g.visual());
        assert!(!g.has_selection());
    }

    // `v` makes the selection follow plain moves; `V` takes whole lines;
    // the same key again ends it.
    #[test]
    fn visual_v_selections_follow_plain_moves() {
        let mut g = Grid::new(20, 3, 100);
        g.advance(b"one\r\ntwo\r\n$ ");
        g.visual_enter();
        g.visual_move(CursorMove::Top, false);
        g.visual_select(VisualSelect::Lines);
        g.visual_move(CursorMove::Down, false);
        assert_eq!(
            g.selection_text().as_deref(),
            Some("one\ntwo\n"),
            "whole lines, as vim yanks them"
        );
        g.visual_select(VisualSelect::Lines);
        assert!(!g.has_selection());
    }

    // The cursor can go up into the scrollback and the view follows it.
    #[test]
    fn visual_top_scrolls_the_history_into_view() {
        let mut g = Grid::new(10, 2, 100);
        g.advance(b"1\r\n2\r\n3\r\n4\r\n5");
        g.visual_enter();
        g.visual_move(CursorMove::Top, false);
        let f = g.frame(&Palette::default_palette());
        assert!(f.display_offset > 0);
        assert_eq!(f.cursor, (0, 0));
        assert!(f.visual);
        g.visual_leave();
        assert_eq!(g.frame(&Palette::default_palette()).display_offset, 0);
    }

    fn fed(text: &str) -> Grid {
        let mut g = Grid::new(20, 4, 100);
        g.advance(text.replace('\n', "\r\n").as_bytes());
        g
    }

    // Every match in the scrollback is counted, the newest is current (it
    // is at the bottom), a lowercase query ignores case, and stepping up
    // wraps round.
    #[test]
    fn find_counts_every_match_and_starts_at_the_newest() {
        let mut g = fed("error one\nok\nError two\nok\nok\nok\nerror three\nok");
        assert_eq!(g.find("error"), (3, 3));
        assert_eq!(g.find_step(true), (3, 2));
        assert_eq!(g.find_step(true), (3, 1));
        assert_eq!(g.find_step(true), (3, 3), "wraps");
        assert_eq!(g.find("Error"), (1, 1), "a capital makes it exact");
        assert_eq!(g.find("nothing"), (0, 0));
        assert_eq!(g.find(""), (0, 0));
    }

    // Output after the search moves the text up; the highlights follow it.
    #[test]
    fn matches_follow_their_text_as_output_scrolls() {
        let palette = crate::palette::Palette::default_palette();
        let mut g = fed("target\n");
        assert_eq!(g.find("target"), (1, 1));
        g.advance(b"a\r\nb\r\nc\r\nd\r\ne\r\n");
        let mut frame = Frame::default();
        g.update_frame(&palette, &mut frame);
        let s = g.search.as_ref().unwrap();
        let line = s.matches[0].start().line;
        let text: String = (0..6).map(|c| g.term.grid()[line][Column(c)].c).collect();
        assert_eq!(text, "target");
    }

    // Find is for text: punctuation is literal, not a pattern.
    #[test]
    fn find_takes_the_query_literally() {
        let mut g = fed("a.b axb (x) [y] $z ~/c-d #1 &");
        assert_eq!(g.find("a.b").0, 1);
        assert_eq!(g.find("(x)").0, 1);
        assert_eq!(g.find("[y]").0, 1);
        assert_eq!(g.find("$z").0, 1);
        assert_eq!(g.find("~/c-d").0, 1);
        assert_eq!(g.find("#1").0, 1);
        assert_eq!(g.find("&").0, 1);
    }

    // The current match wears the selection pair and the others the theme's
    // yellow; closing the bar clears them.
    #[test]
    fn matches_are_coloured_and_the_colours_go_with_the_bar() {
        let palette = crate::palette::Palette::default_palette();
        let mut g = fed("find me\nand me");
        let mut frame = Frame::default();
        g.find("me");
        g.update_frame(&palette, &mut frame);
        let bgs = |frame: &Frame, row: usize| -> Vec<Option<[u8; 3]>> {
            frame.rows[row].runs.iter().map(|r| r.bg).collect()
        };
        assert!(
            bgs(&frame, 1).contains(&Some(palette.selection)),
            "the newest is current"
        );
        assert!(
            bgs(&frame, 0).contains(&Some(palette.ansi[3])),
            "the other is yellow"
        );
        assert_eq!(g.find_current_text().as_deref(), Some("me"));
        g.find_clear();
        g.update_frame(&palette, &mut frame);
        assert!(!bgs(&frame, 0).contains(&Some(palette.ansi[3])));
        // What you were on stays selected, for Cmd+C.
        assert_eq!(g.selection_text().as_deref(), Some("me"));
    }

    use super::*;

    fn text(frame: &Frame) -> Vec<String> {
        frame
            .rows
            .iter()
            .map(|r| r.text.trim_end().to_string())
            .collect()
    }

    // ⚠️ is a text symbol plus U+FE0F, 🏃‍♀️ a runner, a joiner, ♀ and
    // U+FE0F. alacritty keeps the zero-width ones ON a cell; they must
    // reach the row beside its text, on the right columns, or the painter
    // draws a flat ⚠ and a runner next to a text ♀.
    #[test]
    fn zero_width_characters_ride_beside_the_text_on_their_cells() {
        let mut g = Grid::new(20, 2, 100);
        g.advance("\u{26a0}\u{fe0f} x\r\n\u{1f3c3}\u{200d}\u{2640}\u{fe0f}!".as_bytes());
        let f = g.frame(&Palette::default_palette());
        // The text is still one character per cell.
        assert!(
            f.rows[0].text.starts_with("\u{26a0} x"),
            "{:?}",
            f.rows[0].text
        );
        assert_eq!(f.rows[0].zerowidth, vec![(0, "\u{fe0f}".to_string())]);
        let runner = &f.rows[1];
        assert!(
            runner
                .text
                .starts_with(&format!("\u{1f3c3}{SPACER}\u{2640}!")),
            "{:?}",
            runner.text
        );
        assert_eq!(
            runner.zerowidth,
            vec![(0, "\u{200d}".to_string()), (2, "\u{fe0f}".to_string())]
        );
        // A plain row carries none.
        g.advance(b"\x1b[2J\x1b[Hplain");
        assert!(g.frame(&Palette::default_palette()).rows[0]
            .zerowidth
            .is_empty());
    }

    #[test]
    fn parses_output_into_rows_and_runs() {
        let mut g = Grid::new(20, 3, 100);
        g.advance(b"hello \x1b[31mred\x1b[0m\r\nworld");
        let f = g.frame(&Palette::default_palette());
        assert_eq!(text(&f)[..2], ["hello red", "world"]);
        let first = &f.rows[0].runs;
        assert_eq!(first[0].text, "hello ");
        assert_eq!(first[1].text.trim_end(), "red"); // the blanks after it join the run
        assert_ne!(first[0].fg, first[1].fg);
        assert_eq!(f.cursor, (5, 1));
    }

    #[test]
    fn resize_reflows_and_reports_the_change() {
        let mut g = Grid::new(10, 2, 100);
        assert!(g.resize(40, 10));
        assert!(!g.resize(40, 10));
        assert_eq!((g.cols(), g.rows()), (40, 10));
    }

    #[test]
    fn a_query_reply_comes_back_as_a_write_event() {
        let mut g = Grid::new(10, 2, 100);
        g.advance(b"\x1b[6n"); // cursor position report
        let events = g.take_events();
        assert!(matches!(events.first(), Some(TermEvent::Write(s)) if s.starts_with("\x1b[")));
        assert!(g.take_events().is_empty());
    }

    // The kitty keyboard protocol, as Claude Code drives it: it asks with
    // CSI ? u, and only a terminal that ANSWERS gets the push that follows.
    // Unanswered, it stayed on legacy keys and Shift+Enter arrived as a bare
    // CR, identical to Enter, and sent the prompt instead of breaking the
    // line. Every terminal Claude lists as "native" answers this query.
    #[test]
    fn the_kitty_keyboard_query_is_answered() {
        let mut g = Grid::new(10, 2, 100);
        g.advance(b"\x1b[?u");
        let events = g.take_events();
        assert!(
            matches!(events.first(), Some(TermEvent::Write(s)) if s == "\x1b[?0u"),
            "no flags pushed yet, so the answer is ?0u: {events:?}"
        );
    }

    // Claude Code asks and never pushes, so the question itself has to be
    // the opt-in. It ends when the program does: bracketed paste going off
    // is how both Claude (on exit) and zsh (on every Enter) say so.
    #[test]
    fn asking_about_kitty_keys_turns_them_on_until_the_program_leaves() {
        let mut g = Grid::new(10, 2, 100);
        assert!(!g.kitty_keys(), "a shell that never asked gets legacy keys");
        g.advance(b"\x1b[?2004h\x1b[?u");
        let _ = g.take_events(); // the answer is where the asking is noticed
        assert!(g.kitty_keys(), "asked, so Shift+Enter is CSI u from here");
        g.advance(b"echo still running");
        assert!(g.kitty_keys(), "output alone does not end it");
        g.advance(b"\x1b[?2004l");
        assert!(
            !g.kitty_keys(),
            "bracketed paste off is the program leaving"
        );
    }

    // The protocol's own path still counts: a program that pushes a flag
    // gets CSI u whether or not it asked first.
    #[test]
    fn a_pushed_flag_counts_too_and_a_pop_ends_it() {
        let mut g = Grid::new(10, 2, 100);
        g.advance(b"\x1b[>1u");
        assert!(g.kitty_keys());
        g.advance(b"\x1b[<u");
        assert!(!g.kitty_keys());
    }

    // Pi's cursor, exactly as it sends it: an inverse-video space alone on
    // the input line. The row's text is blank, and the run must still
    // carry a background or the painter has nothing to draw the block with.
    #[test]
    fn an_inverse_space_on_an_empty_row_has_a_background() {
        let mut g = Grid::new(20, 3, 100);
        g.advance(b"\r\n\x1b[7m \x1b[0m");
        let f = g.frame(&Palette::default_palette());
        let row = &f.rows[1];
        assert!(
            row.text.trim().is_empty(),
            "nothing but the space: {:?}",
            row.text
        );
        let block = row.runs.iter().find(|r| r.bg.is_some());
        assert!(
            block.is_some(),
            "the inverse space has a background: {:?}",
            row.runs
        );
        assert_eq!(block.unwrap().text, " ");
    }

    #[test]
    fn clear_empties_the_screen() {
        let mut g = Grid::new(10, 3, 100);
        g.advance(b"one\r\ntwo\r\nthree");
        g.clear();
        assert!(text(&g.frame(&Palette::default_palette()))
            .iter()
            .all(|r| r.is_empty()));
    }

    #[test]
    fn scrolling_up_shows_the_history_in_order() {
        let mut g = Grid::new(10, 2, 100);
        g.advance(b"one\r\ntwo\r\nthree\r\nfour");
        g.scroll(2);
        let f = g.frame(&Palette::default_palette());
        assert_eq!(f.display_offset, 2);
        assert_eq!(text(&f), ["one", "two"]);
        g.scroll_to_bottom();
        assert_eq!(
            text(&g.frame(&Palette::default_palette())),
            ["three", "four"]
        );
    }

    #[test]
    fn a_drag_selects_cells_and_reads_back_as_text() {
        let mut g = Grid::new(20, 3, 100);
        g.advance(b"hello world\r\nsecond");
        assert!(!g.has_selection());
        g.start_selection(6, 0, SelectKind::Cells);
        g.update_selection(2, 1, true);
        assert!(g.has_selection());
        assert_eq!(g.selection_text().as_deref(), Some("world\nsec"));
        let p = Palette::default_palette();
        let f = g.frame(&p);
        let selected: Vec<&Run> = f.rows[0]
            .runs
            .iter()
            .filter(|r| r.bg == Some(p.selection))
            .collect();
        assert_eq!(selected.len(), 1);
        // The tail of the first row is selected through to the wrap.
        assert_eq!(selected[0].text.trim_end(), "world");
        assert_eq!(selected[0].fg, p.selection_text);
        g.clear_selection();
        assert!(g.selection_text().is_none());
    }

    #[test]
    fn two_clicks_take_the_word_and_three_the_line() {
        let mut g = Grid::new(20, 3, 100);
        g.advance(b"hello world here");
        g.start_selection(7, 0, SelectKind::Words);
        assert_eq!(g.selection_text().as_deref(), Some("world"));
        g.start_selection(7, 0, SelectKind::Lines);
        // A line selection carries its newline, as a copied line should.
        assert_eq!(g.selection_text().as_deref(), Some("hello world here\n"));
    }

    #[test]
    fn a_selection_follows_its_text_when_the_view_scrolls() {
        let mut g = Grid::new(10, 2, 100);
        g.advance(b"one\r\ntwo\r\nthree");
        g.start_selection(0, 1, SelectKind::Words); // "three"
        g.scroll(1);
        assert_eq!(g.selection_text().as_deref(), Some("three"));
        let p = Palette::default_palette();
        let f = g.frame(&p);
        // Scrolled up, the view shows one, two and the selection is off-screen.
        assert_eq!(text(&f), ["one", "two"]);
        assert!(!f
            .rows
            .iter()
            .any(|r| r.runs.iter().any(|r| r.bg == Some(p.selection))));
        g.scroll_to_bottom();
        let f = g.frame(&p);
        assert!(f.rows[1].runs.iter().any(|r| r.bg == Some(p.selection)));
        assert!(!f.rows[0].runs.iter().any(|r| r.bg == Some(p.selection)));
    }

    #[test]
    fn update_rebuilds_only_the_damaged_rows() {
        let p = Palette::default_palette();
        let mut g = Grid::new(10, 3, 100);
        g.advance(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");
        let mut f = g.frame(&p);
        // Sentinels show which rows the next update leaves alone.
        for r in &mut f.rows {
            r.text = "KEPT".into();
        }
        g.advance(b"\x1b[1;1HONE"); // overwrite row 0 in place
        let rebuilt = g.update_frame(&p, &mut f);
        assert!(rebuilt.contains(&0) && !rebuilt.contains(&1));
        assert_eq!(f.rows[0].text.trim_end(), "ONEee"); // "three" overwritten in place
        assert_eq!(f.rows[1].text, "KEPT");
        // The cursor row (row 0, where it now sits) and the row it left
        // (row 2) are damaged by alacritty's cursor rule; row 1 is not.
        g.scroll(1);
        g.update_frame(&p, &mut f);
        assert!(
            f.rows.iter().all(|r| r.text != "KEPT"),
            "a scroll rebuilds all"
        );
        for r in &mut f.rows {
            r.text = "KEPT".into();
        }
        g.set_palette_changed();
        g.update_frame(&p, &mut f);
        assert!(
            f.rows.iter().all(|r| r.text != "KEPT"),
            "a palette change rebuilds all"
        );
    }

    #[test]
    fn mouse_mode_is_visible() {
        let mut g = Grid::new(10, 3, 100);
        assert!(!g.wants_mouse());
        g.advance(b"\x1b[?1000h\x1b[?1006h");
        assert!(g.wants_mouse());
    }
}
