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
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionRange, SelectionType};
use alacritty_terminal::term::cell::Flags;
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
        }
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
        self.processor.advance(&mut self.term, bytes);
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
        let full = fresh || self.full_dirty || selection.is_some() || frame.selected;
        self.full_dirty = false;
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
        frame.cursor = (col, line.max(0) as usize);
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
        for col in 0..self.size.cols {
            let cell = &row[Column(col)];
            let flags = cell.flags;
            let point = Point::new(grid_line, Column(col));
            let selected = selection.is_some_and(|s| s.contains(point));
            // The common cell is a blank on the default ground: it joins the
            // run before it, or starts one in the default colour, and
            // nothing is resolved. Most of a screen is this, and a flood of
            // short lines is nearly all of it.
            if cell.c == ' '
                && flags.is_empty()
                && !selected
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
            if selected {
                fg_rgb = palette.selection_text;
                bg_rgb = Some(palette.selection);
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
