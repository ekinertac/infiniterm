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
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
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

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub runs: Vec<Run>,
    /// The row as text, for links and selection.
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorKind {
    Block,
    Beam,
    Underline,
    Hidden,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub rows: Vec<Row>,
    pub cursor: (usize, usize),
    pub cursor_kind: CursorKind,
    /// How far up the scrollback the view is; 0 at the bottom.
    pub display_offset: usize,
    pub cols: usize,
}

pub struct Grid {
    term: Term<Listener>,
    processor: Processor,
    events: Listener,
    size: Size,
}

impl Grid {
    pub fn new(cols: usize, rows: usize, scrollback: usize) -> Grid {
        let events = Listener::default();
        let config = Config {
            scrolling_history: scrollback,
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
        true
    }

    pub fn take_events(&mut self) -> Vec<TermEvent> {
        std::mem::take(&mut *self.events.0.borrow_mut())
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

    /// The visible rows with resolved colours. `palette` decides what every
    /// named and indexed colour paints as; a cell's own RGB stays its own.
    pub fn frame(&self, palette: &Palette) -> Frame {
        let content = self.term.renderable_content();
        let cols = self.size.cols;
        let rows = self.size.rows;
        let mut out: Vec<Row> = (0..rows)
            .map(|_| Row {
                runs: Vec::new(),
                text: String::with_capacity(cols),
            })
            .collect();
        for cell in content.display_iter {
            let row = cell.point.line.0 as usize;
            if row >= rows {
                continue;
            }
            let flags = cell.flags;
            let (mut fg, mut bg) = (cell.fg, cell.bg);
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let bold = flags.intersects(Flags::BOLD);
            let fg_rgb = palette.resolve(fg, bold);
            let bg_rgb = match bg {
                Color::Named(NamedColor::Background) if !flags.contains(Flags::INVERSE) => None,
                other => Some(palette.resolve(other, false)),
            };
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
            let r = &mut out[row];
            r.text.push(ch);
            match r.runs.last_mut() {
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
                    r.runs.push(run);
                }
            }
        }
        let cursor_kind = if self.term.mode().contains(TermMode::SHOW_CURSOR) {
            match content.cursor.shape {
                CursorShape::Block => CursorKind::Block,
                CursorShape::Beam => CursorKind::Beam,
                CursorShape::Underline => CursorKind::Underline,
                CursorShape::HollowBlock => CursorKind::Block,
                CursorShape::Hidden => CursorKind::Hidden,
            }
        } else {
            CursorKind::Hidden
        };
        let Point {
            line: Line(line),
            column: Column(col),
        } = content.cursor.point;
        Frame {
            rows: out,
            cursor: (col, line.max(0) as usize),
            cursor_kind,
            display_offset: content.display_offset,
            cols,
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

    #[test]
    fn parses_output_into_rows_and_runs() {
        let mut g = Grid::new(20, 3, 100);
        g.advance(b"hello \x1b[31mred\x1b[0m\r\nworld");
        let f = g.frame(&Palette::default_palette());
        assert_eq!(text(&f)[..2], ["hello red", "world"]);
        let first = &f.rows[0].runs;
        assert_eq!(first[0].text, "hello ");
        assert_eq!(first[1].text, "red");
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
    fn mouse_mode_is_visible() {
        let mut g = Grid::new(10, 3, 100);
        assert!(!g.wants_mouse());
        g.advance(b"\x1b[?1000h\x1b[?1006h");
        assert!(g.wants_mouse());
    }
}
