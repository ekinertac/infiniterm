//! A quiet terminal's scrollback, held as text and runs of style instead of
//! alacritty's cells.
//!
//! alacritty keeps every history row at full width, 24 bytes a cell, plus up
//! to a thousand empty rows it allocates ahead. Replaying 39 real sessions at
//! their real sizes (2026-11-24) cost 457 MB in the cards' terminals for
//! 93,712 rows, 32 MB held here, and nine in ten of those cards had printed
//! nothing for five minutes.
//!
//! ALL OR NOTHING, on purpose. `cool` moves the whole history out, `warm`
//! puts the whole history back, and the owner calls `warm` before ANYTHING
//! that could read or change history: output, a resize, a scroll up, a
//! search, a selection. No code ever runs on a terminal with half a history,
//! so reflow, `CSI 3 J`, the alternate screen and every reader are
//! alacritty's own, unchanged. Only the screen is readable while cold.
//!
//! The round trip is exact, cell by cell (the styles are whole `Cell`s, so a
//! hyperlink, an underline colour and zero-width characters come along).
//!
//! Pure: no clock, no gpui. The owner decides when a terminal is quiet. Used
//! by `infiniterm-term/src/grid.rs` (the card's own terminal) and
//! `backend/mirror.rs` (the join relay's copy of it). Constraints:
//! only for the primary screen (the alternate one has no history, and
//! alacritty hides the primary while it is up), at display offset 0.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::Color;

/// Styles remembered while storing, one per slot of a cheap hash. One look
/// per style change, not a search: output with a colour per cell took 114 ms
/// to store with a search of the last 32. A style that lost its slot is
/// stored again, which costs a few bytes.
const STYLE_SLOTS: usize = 256;
const NO_STYLE: u32 = u32::MAX;

/// The store must stay under this share of what the cells cost, or the
/// history is left alone: a different colour in every cell (a picture drawn
/// in half blocks) is bigger as runs than as cells.
const WORTH_DIVISOR: usize = 2;

/// A terminal's history while it is cold. Empty when it is warm.
#[derive(Default)]
pub struct ColdHistory {
    /// Every character of every row, oldest row first.
    text: String,
    /// Cells as the program left them, with a blank character.
    styles: Vec<Cell>,
    /// (style, cells) in reading order.
    runs: Vec<(u32, u16)>,
    /// Runs per row. A row's trailing default cells are not stored.
    rows: Vec<u16>,
}

impl ColdHistory {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Moves the whole history out of `term` and frees its rows, the ones
    /// alacritty keeps allocated ahead included. Does nothing, and says so,
    /// when there is nothing to move, it would not be worth it
    /// (`WORTH_DIVISOR`), or the terminal is not at rest on its primary
    /// screen. A refusal can cost as much as a success, so the owner asks
    /// once per change, not on every tick. `limit` is the terminal's
    /// configured scrollback.
    pub fn cool<T: EventListener>(&mut self, term: &mut Term<T>, limit: usize) -> bool {
        let history = term.history_size();
        if !self.is_empty()
            || history == 0
            || term.grid().display_offset() != 0
            || term.mode().contains(TermMode::ALT_SCREEN)
        {
            return false;
        }
        let cols = term.columns();
        // A row's run count and a run's length are 16 bits.
        if cols > u16::MAX as usize {
            return false;
        }
        let blank = Cell::default();
        let worth = history * cols * std::mem::size_of::<Cell>() / WORTH_DIVISOR;
        let mut slots = [NO_STYLE; STYLE_SLOTS];
        for i in (1..=history as i32).rev() {
            let row = &term.grid()[Line(-i)][..Column(cols)];
            let end = row.iter().rposition(|c| *c != blank).map_or(0, |p| p + 1);
            let mut runs = 0u16;
            let mut last: Option<u32> = None;
            for cell in &row[..end] {
                self.text.push(cell.c);
                let same = last.filter(|&s| same_style(&self.styles[s as usize], cell));
                match same {
                    Some(_) => self.runs.last_mut().expect("a run is open").1 += 1,
                    None => {
                        let s = self.style(cell, &mut slots);
                        self.runs.push((s, 1));
                        last = Some(s);
                        runs += 1;
                    }
                }
            }
            self.rows.push(runs);
            if self.bytes() > worth {
                *self = ColdHistory::default();
                return false;
            }
        }
        self.text.shrink_to_fit();
        self.styles.shrink_to_fit();
        self.runs.shrink_to_fit();
        self.rows.shrink_to_fit();
        let grid = term.grid_mut();
        // Down to no history, which drops the rows, then the limit back.
        grid.update_history(0);
        grid.truncate();
        grid.update_history(limit);
        true
    }

    /// Puts the history back exactly as `cool` found it. Cheap when warm.
    pub fn warm<T: EventListener>(&mut self, term: &mut Term<T>, limit: usize) {
        if self.is_empty() {
            return;
        }
        let cold = std::mem::take(self);
        let rows = cold.rows.len();
        let grid = term.grid_mut();
        // alacritty's only public way to grow history: to the limit, so the
        // limit is the size wanted for a moment.
        grid.update_history(rows);
        grid.initialize_all();
        grid.update_history(limit);
        let mut text = cold.text.chars();
        let mut runs = cold.runs.iter();
        for (i, n) in cold.rows.iter().enumerate() {
            let row = &mut grid[Line(i as i32 - rows as i32)];
            let mut col = 0;
            for &(style, cells) in runs.by_ref().take(*n as usize) {
                for _ in 0..cells {
                    let cell = &mut row[Column(col)];
                    cell.clone_from(&cold.styles[style as usize]);
                    cell.c = text.next().expect("a character per stored cell");
                    col += 1;
                }
            }
        }
    }

    /// The index of `cell`'s style, stored if its slot holds another.
    fn style(&mut self, cell: &Cell, slots: &mut [u32; STYLE_SLOTS]) -> u32 {
        let slot = &mut slots[style_slot(cell)];
        if *slot != NO_STYLE && same_style(&self.styles[*slot as usize], cell) {
            return *slot;
        }
        let mut style = cell.clone();
        style.c = ' ';
        self.styles.push(style);
        *slot = (self.styles.len() - 1) as u32;
        *slot
    }

    /// What the store holds so far (its own vectors, not what a hyperlink or
    /// a zero-width character points at: those the cells share).
    fn bytes(&self) -> usize {
        self.text.len()
            + std::mem::size_of_val(&self.styles[..])
            + std::mem::size_of_val(&self.runs[..])
            + std::mem::size_of_val(&self.rows[..])
    }
}

/// A slot from what usually tells styles apart. Cells whose link or
/// underline colour differs share one, which only costs a style stored again.
fn style_slot(cell: &Cell) -> usize {
    let color = |c: Color| match c {
        Color::Named(n) => n as u32,
        Color::Indexed(i) => 1 << 24 | i as u32,
        Color::Spec(rgb) => 2 << 24 | (rgb.r as u32) << 16 | (rgb.g as u32) << 8 | rgb.b as u32,
    };
    // Fibonacci hashing (2^32 over the golden ratio): the top bits mix all.
    let h = color(cell.fg)
        .wrapping_mul(0x9E37_79B1)
        .wrapping_add(color(cell.bg).wrapping_mul(0x85EB_CA77))
        .wrapping_add((cell.flags.bits() as u32).wrapping_mul(0xC2B2_AE3D));
    (h >> 24) as usize % STYLE_SLOTS
}

/// Everything about two cells but the character.
fn same_style(a: &Cell, b: &Cell) -> bool {
    a.fg == b.fg && a.bg == b.bg && a.flags == b.flags && a.extra == b.extra
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::Event;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::Processor;

    const LIMIT: usize = 500;

    #[derive(Clone)]
    struct Mute;
    impl EventListener for Mute {
        fn send_event(&self, _: Event) {}
    }

    #[derive(Clone, Copy)]
    struct Size(usize, usize);
    impl Dimensions for Size {
        fn total_lines(&self) -> usize {
            self.0
        }
        fn screen_lines(&self) -> usize {
            self.0
        }
        fn columns(&self) -> usize {
            self.1
        }
    }

    struct T {
        term: Term<Mute>,
        p: Processor,
        cold: ColdHistory,
    }

    impl T {
        fn new(rows: usize, cols: usize) -> T {
            let config = Config {
                scrolling_history: LIMIT,
                ..Default::default()
            };
            T {
                term: Term::new(config, &Size(rows, cols), Mute),
                p: Processor::new(),
                cold: ColdHistory::default(),
            }
        }
        fn feed(&mut self, bytes: &[u8]) {
            self.cold.warm(&mut self.term, LIMIT);
            self.p.advance(&mut self.term, bytes);
        }
        fn resize(&mut self, rows: usize, cols: usize) {
            self.cold.warm(&mut self.term, LIMIT);
            self.term.resize(Size(rows, cols));
        }
        fn rest(&mut self) -> bool {
            self.cold.cool(&mut self.term, LIMIT)
        }
        /// Every cell, history first, and everything else that is state.
        fn dump(&mut self) -> (Vec<Vec<Cell>>, usize, String) {
            self.cold.warm(&mut self.term, LIMIT);
            let t = &self.term;
            let rows = (-(t.history_size() as i32)..t.screen_lines() as i32)
                .map(|l| t.grid()[Line(l)][..Column(t.columns())].to_vec())
                .collect();
            (
                rows,
                t.history_size(),
                format!(
                    "{:?} {:?} {}",
                    t.grid().cursor.point,
                    t.mode(),
                    t.grid().display_offset()
                ),
            )
        }
    }

    /// Output with everything a cell can hold.
    fn varied(lines: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for i in 0..lines {
            let line = match i % 9 {
                0 => format!(
                    "\x1b[3{}mline {i}\x1b[0m plain \x1b[1;4mbold\x1b[0m\r\n",
                    i % 8
                ),
                1 => "wide \u{4f60}\u{597d} emoji \u{1f980} e\u{301}\u{302}\r\n".into(),
                2 => format!(
                    "\x1b]8;id=x{i};https://a.example/{i}\x1b\\link\x1b]8;;\x1b\\ after\r\n"
                ),
                3 => "\x1b[4:3m\x1b[58;2;1;2;3mcurly\x1b[0m\r\n".into(),
                4 => format!("{}\r\n", "wrapped ".repeat(20)),
                5 => "\x1b[48;2;10;20;30m\x1b[K\x1b[0m\r\n".into(),
                6 => "\r\n".into(),
                7 => "a\tb\tc   \x1b[7m inverse \x1b[0m  \r\n".into(),
                _ => format!("\x1b[38;5;{}mindexed\x1b[0m\r\n", i % 256),
            };
            out.extend(line.into_bytes());
        }
        out
    }

    #[test]
    fn cooling_and_warming_gives_back_every_cell() {
        let mut stock = T::new(10, 40);
        let mut cold = T::new(10, 40);
        stock.feed(&varied(200));
        cold.feed(&varied(200));
        assert!(cold.rest());
        assert_eq!(cold.term.history_size(), 0, "the rows left alacritty");
        assert_eq!(cold.dump(), stock.dump());
    }

    #[test]
    fn a_full_history_survives() {
        let mut stock = T::new(5, 30);
        let mut cold = T::new(5, 30);
        stock.feed(&varied(2 * LIMIT));
        cold.feed(&varied(2 * LIMIT));
        assert!(cold.rest());
        assert_eq!(cold.dump(), stock.dump());
        // Still capped, and still the newest rows, after more output.
        for t in [&mut stock, &mut cold] {
            t.feed(&varied(40));
        }
        assert_eq!(stock.term.history_size(), LIMIT);
        assert_eq!(cold.dump(), stock.dump());
    }

    #[test]
    fn everything_after_a_rest_is_as_if_it_never_rested() {
        type Step = fn(&mut T);
        let steps: &[(&str, Step)] = &[
            ("output", |t| t.feed(&varied(30))),
            ("wider", |t| t.resize(10, 70)),
            ("narrower", |t| t.resize(10, 17)),
            ("taller", |t| t.resize(30, 40)),
            ("shorter", |t| t.resize(3, 40)),
            ("clear saved lines", |t| t.feed(b"\x1b[3J")),
            ("clear like `clear`, then a prompt", |t| {
                t.feed(b"\x1b[H\x1b[2J\x1b[3J$ ")
            }),
            ("reset", |t| t.feed(b"\x1bc")),
            ("the alternate screen, resized, and back", |t| {
                t.feed(b"\x1b[?1049h\x1b[2Jvim");
                t.resize(14, 55);
                t.feed(b"\x1b[?1049l");
            }),
            ("a pen left on", |t| t.feed(b"\x1b[31mred")),
            ("scroll region", |t| {
                t.feed(b"\x1b[2;5r\x1b[5;1H\n\n\n\x1b[r")
            }),
        ];
        for (a, first) in steps {
            for (b, second) in steps {
                let mut stock = T::new(10, 40);
                let mut cold = T::new(10, 40);
                for t in [&mut stock, &mut cold] {
                    t.feed(&varied(120));
                }
                cold.rest();
                first(&mut stock);
                first(&mut cold);
                cold.rest();
                second(&mut stock);
                second(&mut cold);
                cold.rest();
                assert_eq!(cold.dump(), stock.dump(), "{a}, then {b}");
            }
        }
    }

    #[test]
    fn a_history_bigger_as_runs_than_as_cells_is_left_alone() {
        let picture = || -> Vec<u8> {
            (0..200usize)
                .flat_map(|r| {
                    let cells = (0..40usize).map(move |c| {
                        format!("\x1b[38;2;{c};{r};{};48;2;{r};{c};9m\u{2580}", r * c % 256)
                    });
                    cells
                        .chain(["\x1b[0m\r\n".into()])
                        .flat_map(String::into_bytes)
                        .collect::<Vec<u8>>()
                })
                .collect()
        };
        let mut t = T::new(10, 40);
        let mut stock = T::new(10, 40);
        t.feed(&picture());
        stock.feed(&picture());
        assert!(!t.rest(), "not worth it");
        assert!(t.cold.is_empty() && t.term.history_size() > 0);
        assert_eq!(t.dump(), stock.dump());
        // Styles by turns are: eight colours, one cell each.
        let mut t = T::new(10, 40);
        for r in 0..200 {
            let line: String = (0..40)
                .map(|c| format!("\x1b[3{}mx", (r + c) % 8))
                .collect();
            t.feed(line.as_bytes());
            t.feed(b"\r\n");
        }
        assert!(t.rest());
        assert!(t.cold.styles.len() < 100, "a style per slot, not per cell");
    }

    #[test]
    fn the_screen_is_untouched_while_cold() {
        let mut t = T::new(6, 40);
        t.feed(&varied(50));
        let screen = |t: &T| -> Vec<Vec<Cell>> {
            (0..6)
                .map(|l| t.term.grid()[Line(l)][..Column(40)].to_vec())
                .collect()
        };
        let before = screen(&t);
        assert!(t.rest());
        assert_eq!(screen(&t), before);
    }

    #[test]
    fn nothing_to_move_is_left_alone() {
        let mut t = T::new(10, 40);
        assert!(!t.rest(), "no history");
        t.feed(&varied(50));
        t.feed(b"\x1b[?1049h");
        assert!(!t.rest(), "the alternate screen hides the history");
        t.feed(b"\x1b[?1049l");
        t.term
            .scroll_display(alacritty_terminal::grid::Scroll::Delta(3));
        assert!(!t.rest(), "scrolled up, so it is being read");
        t.term
            .scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        assert!(t.rest());
        assert!(!t.rest(), "already cold");
    }
}
