//! A headless terminal that follows one session, so a viewer that joins can be
//! given what the card holds (#621): the scrollback and the screen, with their
//! colours, instead of the last bytes the pty wrote.
//!
//! Why not replay bytes: most of a ring is repaints (a spinner, a status line,
//! a full redraw), so 256 KiB of Claude Code output left a joining viewer with
//! 6 to 33 lines above its screen while the host's own terminal held hundreds.
//! The mirror is fed the same bytes the relay already sees, so it ends up in the
//! state the host's terminal is in, and `snapshot` writes that state out as
//! plain escape sequences. A viewer feeds them like any output, which is why
//! the wire needs no new frame and an old viewer keeps working.
//!
//! Pure: no socket, no gpui. Used by relay.rs. The host's own `Grid`
//! (infiniterm-term) is not shared on purpose: it lives on the ui thread and the
//! relay is fed from the daemon's reader threads.

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};

/// Lines kept above the screen. Smaller than the host's own scrollback: this is
/// a second copy per session, and a phone does not need ten thousand lines.
pub const MIRROR_HISTORY: usize = 3000;

/// The replies a program waits for (a cursor position, a colour) belong to the
/// host's terminal. The mirror answers nobody.
#[derive(Clone)]
struct Mute;

impl EventListener for Mute {
    fn send_event(&self, _: Event) {}
}

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

pub struct Mirror {
    term: Term<Mute>,
    processor: Processor,
}

impl Mirror {
    pub fn new(cols: u16, rows: u16) -> Mirror {
        let config = Config {
            scrolling_history: MIRROR_HISTORY,
            ..Default::default()
        };
        Mirror {
            term: Term::new(config, &size(cols, rows), Mute),
            processor: Processor::new(),
        }
    }

    pub fn advance(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(size(cols, rows));
    }

    /// The scrollback and the screen as output. Fed to a fresh terminal of the
    /// same size it rebuilds both: the lines above the screen scroll into the
    /// viewer's history as the text is written, the last `rows` lines are its
    /// screen, and the cursor and the modes a program set are put back after.
    ///
    /// Rows the program wrapped (the grid marks them) are written without a line
    /// break, so a viewer of another width wraps them itself. A program that
    /// draws its own line breaks (Claude Code) leaves no mark, and its rows stay
    /// as drawn.
    ///
    /// While a full-screen program has the alternate screen, the primary screen
    /// (with its scrollback) is written first, then the switch to the alternate
    /// screen and its content, so that when the program quits the viewer is
    /// back on the same primary screen the host shows. Only the primary screen
    /// has history. Needs `&mut self`: the primary screen is read by swapping to
    /// it and back, and the alternate screen is written back into the mirror
    /// after the swap (entering it clears it).
    pub fn snapshot(&mut self) -> Vec<u8> {
        let mode = *self.term.mode();
        let alt = mode.contains(TermMode::ALT_SCREEN);

        let mut out = Vec::new();
        // Hidden while it is painted, shown again at the end if the program had it on.
        out.extend_from_slice(b"\x1b[0m\x1b[?25l");
        if alt {
            let alt_screen = self.paint(0);
            self.term.swap_alt();
            let top = -(self.term.history_size() as i32);
            out.extend(self.paint(top));
            self.term.swap_alt();
            // The swap back cleared the alternate screen: put its content back.
            self.advance(b"\x1b[H");
            self.advance(&alt_screen);
            out.extend_from_slice(b"\x1b[?1049h\x1b[2J\x1b[H");
            out.extend(alt_screen);
        } else {
            let top = -(self.term.history_size() as i32);
            out.extend(self.paint(top));
        }
        for (flag, set) in [
            (TermMode::APP_CURSOR, "\x1b[?1h"),
            (TermMode::APP_KEYPAD, "\x1b="),
            (TermMode::BRACKETED_PASTE, "\x1b[?2004h"),
            (TermMode::MOUSE_REPORT_CLICK, "\x1b[?1000h"),
            (TermMode::MOUSE_DRAG, "\x1b[?1002h"),
            (TermMode::MOUSE_MOTION, "\x1b[?1003h"),
            (TermMode::SGR_MOUSE, "\x1b[?1006h"),
            (TermMode::FOCUS_IN_OUT, "\x1b[?1004h"),
            (TermMode::ALTERNATE_SCROLL, "\x1b[?1007h"),
            (TermMode::SHOW_CURSOR, "\x1b[?25h"),
        ] {
            if mode.contains(flag) {
                out.extend_from_slice(set.as_bytes());
            }
        }
        if !mode.contains(TermMode::LINE_WRAP) {
            out.extend_from_slice(b"\x1b[?7l");
        }
        out
    }

    /// The rows of the screen that is showing, from line `top` (negative: into
    /// its history) to its last row, then the cursor.
    fn paint(&self, top: i32) -> Vec<u8> {
        let term = &self.term;
        let grid = term.grid();
        let cols = term.columns();
        let screen = term.screen_lines() as i32;
        let mut out = Vec::new();
        let mut style = Style::default();
        for line in top..screen {
            let row = &grid[Line(line)];
            let wrapped = write_row(&mut out, &mut style, &row[Column(0)..Column(cols)]);
            if !wrapped && line < screen - 1 {
                style.reset(&mut out);
                out.extend_from_slice(b"\r\n");
            }
        }
        style.reset(&mut out);
        let cursor = grid.cursor.point;
        out.extend_from_slice(
            format!("\x1b[{};{}H", cursor.line.0 + 1, cursor.column.0 + 1).as_bytes(),
        );
        out
    }
}

fn size(cols: u16, rows: u16) -> Size {
    Size {
        cols: (cols as usize).max(2),
        rows: (rows as usize).max(1),
    }
}

/// The style the output is currently in, so a code is written only on a change.
#[derive(PartialEq)]
struct Style {
    on: bool,
    fg: Option<Color>,
    bg: Option<Color>,
    flags: Flags,
}

impl Default for Style {
    fn default() -> Style {
        Style {
            on: false,
            fg: None,
            bg: None,
            flags: Flags::empty(),
        }
    }
}

const SHOWN_FLAGS: Flags = Flags::INVERSE
    .union(Flags::BOLD)
    .union(Flags::ITALIC)
    .union(Flags::DIM)
    .union(Flags::HIDDEN)
    .union(Flags::STRIKEOUT)
    .union(Flags::ALL_UNDERLINES);

impl Style {
    fn reset(&mut self, out: &mut Vec<u8>) {
        if self.on {
            out.extend_from_slice(b"\x1b[0m");
            *self = Style::default();
        }
    }

    fn set(&mut self, out: &mut Vec<u8>, cell: &Cell) {
        let flags = cell.flags & SHOWN_FLAGS;
        let fg = (cell.fg != Color::Named(NamedColor::Foreground)).then_some(cell.fg);
        let bg = (cell.bg != Color::Named(NamedColor::Background)).then_some(cell.bg);
        if self.fg == fg && self.bg == bg && self.flags == flags {
            return;
        }
        // Every change starts from a reset: simpler than undoing single attributes.
        let mut codes = String::from("0");
        for (flag, code) in [
            (Flags::BOLD, "1"),
            (Flags::DIM, "2"),
            (Flags::ITALIC, "3"),
            (Flags::UNDERLINE, "4"),
            (Flags::DOUBLE_UNDERLINE, "21"),
            (Flags::UNDERCURL, "4:3"),
            (Flags::INVERSE, "7"),
            (Flags::HIDDEN, "8"),
            (Flags::STRIKEOUT, "9"),
        ] {
            if flags.contains(flag) {
                codes.push(';');
                codes.push_str(code);
            }
        }
        if let Some(c) = fg {
            push_color(&mut codes, c, false);
        }
        if let Some(c) = bg {
            push_color(&mut codes, c, true);
        }
        out.extend_from_slice(format!("\x1b[{codes}m").as_bytes());
        *self = Style {
            on: true,
            fg,
            bg,
            flags,
        };
    }
}

fn push_color(codes: &mut String, color: Color, background: bool) {
    let base = if background { 40 } else { 30 };
    let named = |n: NamedColor| -> Option<u8> {
        use NamedColor::*;
        Some(match n {
            Black | DimBlack => 0,
            Red | DimRed => 1,
            Green | DimGreen => 2,
            Yellow | DimYellow => 3,
            Blue | DimBlue => 4,
            Magenta | DimMagenta => 5,
            Cyan | DimCyan => 6,
            White | DimWhite => 7,
            BrightBlack => 8,
            BrightRed => 9,
            BrightGreen => 10,
            BrightYellow => 11,
            BrightBlue => 12,
            BrightMagenta => 13,
            BrightCyan => 14,
            BrightWhite => 15,
            // The default colours, the cursor and the rest.
            _ => return None,
        })
    };
    let code = match color {
        Color::Named(n) => match named(n) {
            Some(i) if i < 8 => format!(";{}", base + i as u32),
            Some(i) => format!(";{}", base + 60 + (i - 8) as u32),
            None => return,
        },
        Color::Indexed(i) => format!(";{};5;{i}", base + 8),
        Color::Spec(rgb) => format!(";{};2;{};{};{}", base + 8, rgb.r, rgb.g, rgb.b),
    };
    codes.push_str(&code);
}

/// A cell that shows nothing: a space in the default colours and no attribute
/// that paints a blank (inverse, underline, strike-through).
fn is_plain_blank(cell: &Cell) -> bool {
    cell.c == ' '
        && cell.bg == Color::Named(NamedColor::Background)
        && !cell
            .flags
            .intersects(Flags::INVERSE | Flags::STRIKEOUT | Flags::ALL_UNDERLINES)
        && cell.zerowidth().is_none()
}

/// Writes one row. Returns whether the program wrapped it onto the next one,
/// in which case the row is written in full and the caller adds no line break.
fn write_row(out: &mut Vec<u8>, style: &mut Style, cells: &[Cell]) -> bool {
    let wrapped = cells
        .last()
        .is_some_and(|c| c.flags.contains(Flags::WRAPLINE));
    let end = if wrapped {
        cells.len()
    } else {
        cells
            .iter()
            .rposition(|c| !is_plain_blank(c))
            .map_or(0, |i| i + 1)
    };
    let mut buf = [0u8; 4];
    for cell in &cells[..end] {
        // The second half of a wide character is drawn by its first half.
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        style.set(out, cell);
        let c = if cell.c == '\0' { ' ' } else { cell.c };
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        for z in cell.zerowidth().into_iter().flatten() {
            out.extend_from_slice(z.encode_utf8(&mut buf).as_bytes());
        }
    }
    wrapped
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every line, history first, as (text, per-cell colours and flags) so a
    /// rebuilt mirror can be compared with the one it came from.
    fn dump(m: &Mirror) -> Vec<Vec<(char, Color, Color, Flags)>> {
        let term = &m.term;
        let top = -(term.history_size() as i32);
        (top..term.screen_lines() as i32)
            .map(|l| {
                (0..term.columns())
                    .map(|c| {
                        let cell = &term.grid()[Line(l)][Column(c)];
                        (
                            cell.c,
                            cell.fg,
                            cell.bg,
                            cell.flags & (SHOWN_FLAGS | Flags::WIDE_CHAR),
                        )
                    })
                    .collect()
            })
            .collect()
    }

    fn rebuilt(m: &mut Mirror, cols: u16, rows: u16) -> Mirror {
        let mut fresh = Mirror::new(cols, rows);
        fresh.advance(&m.snapshot());
        fresh
    }

    fn text(m: &Mirror) -> String {
        dump(m)
            .iter()
            .map(|l| {
                l.iter()
                    .map(|c| c.0)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn scrollback_and_screen_come_back_line_for_line() {
        let mut m = Mirror::new(40, 10);
        for i in 0..100 {
            m.advance(format!("line {i}\r\n").as_bytes());
        }
        let r = rebuilt(&mut m, 40, 10);
        assert_eq!(text(&r), text(&m));
        assert!(text(&r).starts_with("line 0\n"), "the oldest line is kept");
    }

    #[test]
    fn colours_and_attributes_survive() {
        let mut m = Mirror::new(40, 5);
        m.advance(b"\x1b[1;31mred bold\x1b[0m \x1b[38;5;208morange\x1b[0m \x1b[48;2;1;2;3;4mon rgb\x1b[0m\r\n");
        m.advance(b"\x1b[7minverse\x1b[0m \x1b[94mbright\x1b[0m\r\n");
        assert_eq!(dump(&rebuilt(&mut m, 40, 5)), dump(&m));
    }

    #[test]
    fn a_wrapped_line_is_rebuilt_as_one_line_and_wraps_again_on_a_narrower_viewer() {
        let mut m = Mirror::new(20, 5);
        m.advance(b"abcdefghij abcdefghij abcdefghij\r\nnext\r\n");
        assert_eq!(dump(&rebuilt(&mut m, 20, 5)), dump(&m));
        // A viewer of another width wraps the same text itself.
        let narrow = rebuilt(&mut m, 10, 8);
        assert!(text(&narrow).contains("abcdefghij\n abcdefghi\nj abcdefgh\nij"));
    }

    #[test]
    fn wide_characters_and_cursor_position_survive() {
        let mut m = Mirror::new(20, 5);
        m.advance("日本語 ok\r\nsecond".as_bytes());
        m.advance(b"\x1b[2;4H");
        let r = rebuilt(&mut m, 20, 5);
        assert_eq!(dump(&r), dump(&m));
        assert_eq!(r.term.grid().cursor.point, m.term.grid().cursor.point);
    }

    #[test]
    fn the_modes_a_program_set_are_put_back() {
        let mut m = Mirror::new(20, 5);
        m.advance(b"\x1b[?2004h\x1b[?1h\x1b[?25l");
        let r = rebuilt(&mut m, 20, 5);
        let mode = *r.term.mode();
        assert!(mode.contains(TermMode::BRACKETED_PASTE));
        assert!(mode.contains(TermMode::APP_CURSOR));
        assert!(!mode.contains(TermMode::SHOW_CURSOR));
    }

    #[test]
    fn a_viewer_that_joins_during_vim_is_back_on_the_primary_screen_when_vim_quits() {
        let mut m = Mirror::new(20, 4);
        for i in 0..10 {
            m.advance(format!("history {i}\r\n").as_bytes());
        }
        m.advance(b"$ vim\r\n\x1b[?1049h\x1b[H\x1b[2Jfull screen\x1b[3;5Hcursor here");
        let before = dump(&m);

        let mut viewer = rebuilt(&mut m, 20, 4);
        assert!(viewer.term.mode().contains(TermMode::ALT_SCREEN));
        assert_eq!(dump(&viewer), before, "the alternate screen is the same");
        assert_eq!(viewer.term.grid().cursor.point, m.term.grid().cursor.point);

        // Taking the snapshot must not have disturbed the host's own mirror.
        assert_eq!(dump(&m), before);
        assert_eq!(m.snapshot(), m.snapshot(), "and it can be taken again");

        // vim quits: the viewer shows the shell the host shows, history and all.
        m.advance(b"\x1b[?1049l");
        viewer.advance(b"\x1b[?1049l");
        assert!(!viewer.term.mode().contains(TermMode::ALT_SCREEN));
        assert_eq!(dump(&viewer), dump(&m));
        assert!(text(&viewer).contains("history 0"));
        assert!(text(&viewer).contains("$ vim"));
        assert_eq!(viewer.term.grid().cursor.point, m.term.grid().cursor.point);
    }

    #[test]
    fn a_program_that_queries_gets_no_answer_from_the_mirror() {
        // Cursor position report: the host's terminal answers it, not this one.
        let mut m = Mirror::new(20, 4);
        m.advance(b"\x1b[6n\x1b]11;?\x07");
        assert!(text(&m).trim().is_empty());
    }

    /// Not a test: the cost of a mirror, for the PR (#621). Run with
    /// `cargo test -p infiniterm-core mirror_cost -- --ignored --nocapture`.
    /// The dev profile optimises dependencies (Cargo.toml), and the parser is
    /// the cost, so the numbers are close to a release build's.
    #[test]
    #[ignore]
    fn mirror_cost() {
        fn rss_mb() -> f64 {
            let out = std::process::Command::new("ps")
                .args(["-o", "rss=", "-p", &std::process::id().to_string()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse::<f64>()
                .unwrap()
                / 1024.0
        }
        // A screenful of output as a busy card writes it: text, colours, line breaks.
        let chunk: String = (0..200)
            .map(|i| format!("\x1b[3{}m{:>4} the quick brown fox jumps over the lazy dog, again and again and again and again and again and again and again and again and again and again and again and again and again\x1b[0m\r\n", i % 8, i))
            .collect();

        let base = rss_mb();
        let mut idle: Vec<Mirror> = (0..20).map(|_| Mirror::new(200, 50)).collect();
        let after_idle = rss_mb();
        println!("20 fresh mirrors (200x50): +{:.1} MB", after_idle - base);

        // Fill each to a full scrollback: 3000 lines of history plus the screen.
        for m in idle.iter_mut() {
            for _ in 0..(MIRROR_HISTORY / 200 + 2) {
                m.advance(chunk.as_bytes());
            }
        }
        let after_full = rss_mb();
        println!(
            "20 mirrors with a full scrollback ({} lines, 200 columns): +{:.1} MB, {:.1} MB each",
            MIRROR_HISTORY,
            after_full - after_idle,
            (after_full - after_idle) / 20.0
        );

        // Parse speed: a few MB, as `cat` of a big file.
        let mut m = Mirror::new(200, 50);
        let mut fed = 0usize;
        let start = std::time::Instant::now();
        while fed < 16 * 1024 * 1024 {
            m.advance(chunk.as_bytes());
            fed += chunk.len();
        }
        let secs = start.elapsed().as_secs_f64();
        println!(
            "mirror parse: {:.1} MB in {:.2} s = {:.0} MB/s",
            fed as f64 / 1048576.0,
            secs,
            fed as f64 / 1048576.0 / secs
        );

        let start = std::time::Instant::now();
        let snapshot = m.snapshot();
        println!(
            "snapshot of a full scrollback: {:.1} KB in {:.0} ms",
            snapshot.len() as f64 / 1024.0,
            start.elapsed().as_secs_f64() * 1000.0
        );
    }

    #[test]
    fn a_resize_keeps_the_text() {
        let mut m = Mirror::new(40, 10);
        m.advance(b"hello\r\nworld\r\n");
        m.resize(20, 6);
        assert!(text(&m).contains("hello\nworld"));
        let r = rebuilt(&mut m, 20, 6);
        assert_eq!(text(&r), text(&m));
    }
}
