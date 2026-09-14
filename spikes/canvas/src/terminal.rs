//! A terminal card's insides: a PTY, a reader thread, an alacritty `Term`,
//! and the painter that turns its grid into shaped lines.
//!
//! The shape the term-zoom spike settled on: the reader thread only moves
//! bytes into a bounded channel (the child blocks on the kernel PTY buffer
//! when it fills), and the UI thread parses a byte budget per frame before
//! painting. No lock, no contention; `alacritty_terminal::event_loop` is
//! not used. Keys are encoded here too, the small subset a shell needs.
use alacritty_terminal::{
    event::{Event, EventListener, WindowSize},
    grid::Dimensions,
    term::{cell::Flags, Config, Term},
    tty,
    vte::ansi::{Color, NamedColor, Processor},
};
use gpui::{fill, point, px, size, App, Bounds, Font, Hsla, Pixels, SharedString, TextRun, Window};
use std::{
    collections::VecDeque,
    fs::File,
    io::{Read, Write},
    sync::mpsc::{sync_channel, Receiver},
    time::Duration,
};

pub const COLS: usize = 80;
pub const ROWS: usize = 40;
pub const FONT_PX: f32 = 14.0;
pub const LINE_HEIGHT: f32 = 1.2;

#[derive(Clone)]
struct Listener;
impl EventListener for Listener {
    fn send_event(&self, _: Event) {}
}

struct TermSize;
impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        ROWS
    }
    fn screen_lines(&self) -> usize {
        ROWS
    }
    fn columns(&self) -> usize {
        COLS
    }
}

pub struct Terminal {
    term: Term<Listener>,
    processor: Processor,
    rx: Receiver<Vec<u8>>,
    writer: File,
    pending: VecDeque<Vec<u8>>,
}

impl Terminal {
    pub fn spawn(cwd: Option<&str>) -> Terminal {
        let term = Term::new(Config::default(), &TermSize, Listener);
        let opts = tty::Options {
            shell: Some(tty::Shell::new("/bin/zsh".into(), vec!["-l".into()])),
            working_directory: cwd.map(Into::into),
            ..Default::default()
        };
        let window_size = WindowSize {
            num_lines: ROWS as u16,
            num_cols: COLS as u16,
            cell_width: 8,
            cell_height: 17,
        };
        let pty = tty::new(&opts, window_size, 0).expect("pty");
        let mut reader = pty.file().try_clone().expect("clone");
        let writer = pty.file().try_clone().expect("clone");
        // The Pty struct owns the child; keep it alive for the run.
        std::mem::forget(pty);
        let (tx, rx) = sync_channel::<Vec<u8>>(8);
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        });
        Terminal { term, processor: Processor::new(), rx, writer, pending: VecDeque::new() }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
    }

    /// Parses up to `budget` bytes; returns how many it used.
    pub fn drain(&mut self, budget: usize) -> usize {
        while let Ok(chunk) = self.rx.try_recv() {
            self.pending.push_back(chunk);
        }
        let mut used = 0;
        while used < budget {
            let Some(mut chunk) = self.pending.pop_front() else { break };
            let take = chunk.len().min(budget - used);
            self.processor.advance(&mut self.term, &chunk[..take]);
            used += take;
            if take < chunk.len() {
                chunk.drain(..take);
                self.pending.push_front(chunk);
            }
        }
        used
    }

    /// A key as the bytes a shell expects. `None` for keys the spike does
    /// not encode; the real card has the whole table.
    pub fn encode_key(k: &gpui::Keystroke) -> Option<Vec<u8>> {
        let m = &k.modifiers;
        let seq = |s: &str| Some(s.as_bytes().to_vec());
        match k.key.as_str() {
            "enter" => seq("\r"),
            "backspace" => seq("\x7f"),
            "tab" => seq("\t"),
            "escape" => seq("\x1b"),
            "space" => seq(" "),
            // Alt+arrow is word movement, ESC b / ESC f, as in the Tauri app.
            "left" if m.alt => seq("\x1bb"),
            "right" if m.alt => seq("\x1bf"),
            "up" => seq("\x1b[A"),
            "down" => seq("\x1b[B"),
            "right" => seq("\x1b[C"),
            "left" => seq("\x1b[D"),
            "home" => seq("\x1b[H"),
            "end" => seq("\x1b[F"),
            "delete" => seq("\x1b[3~"),
            key if m.control && key.len() == 1 => {
                let c = key.as_bytes()[0].to_ascii_lowercase();
                if c.is_ascii_lowercase() {
                    Some(vec![c - b'a' + 1])
                } else {
                    None
                }
            }
            _ => k.key_char.as_ref().map(|s| s.as_bytes().to_vec()),
        }
    }

    /// Paints the grid into `bounds` at `scale`; `bounds` is the card's
    /// screen rect. Rows are one shaped line each, consecutive cells with
    /// equal colours sharing a run.
    pub fn paint(&self, bounds: Bounds<Pixels>, scale: f32, font: &Font, window: &mut Window, cx: &mut App) {
        let font_size = px(FONT_PX * scale);
        let line_h = px(FONT_PX * LINE_HEIGHT * scale);
        window.paint_quad(fill(bounds, palette(Color::Named(NamedColor::Background))));
        let content = self.term.renderable_content();
        let mut rows: Vec<(String, Vec<TextRun>)> =
            (0..ROWS).map(|_| (String::with_capacity(COLS), Vec::new())).collect();
        for cell in content.display_iter {
            let row = cell.point.line.0 as usize;
            if row >= ROWS || cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let (text, runs) = &mut rows[row];
            let (fg, bg) = if cell.flags.contains(Flags::INVERSE) { (cell.bg, cell.fg) } else { (cell.fg, cell.bg) };
            let color = palette(fg);
            let background = match bg {
                Color::Named(NamedColor::Background) => None,
                other => Some(palette(other)),
            };
            let len = cell.c.len_utf8();
            text.push(cell.c);
            match runs.last_mut() {
                Some(run) if run.color == color && run.background_color == background => run.len += len,
                _ => runs.push(TextRun {
                    len,
                    font: font.clone(),
                    color,
                    background_color: background,
                    underline: None,
                    strikethrough: None,
                }),
            }
        }
        // The cursor: a block in the cursor colour under the cell.
        let cursor = content.cursor.point;
        let cell_w = bounds.size.width / COLS as f32;
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.origin.x + cell_w * cursor.column.0 as f32, bounds.origin.y + line_h * cursor.line.0 as f32),
                size(cell_w, line_h),
            ),
            palette(Color::Named(NamedColor::Cursor)),
        ));
        for (r, (text, runs)) in rows.into_iter().enumerate() {
            if text.trim().is_empty() {
                continue;
            }
            let line = window.text_system().shape_line(SharedString::from(text), font_size, &runs, None);
            let origin = point(bounds.origin.x, bounds.origin.y + line_h * r as f32);
            let _ = line.paint(origin, line_h, window, cx);
        }
    }
}

/// Catppuccin Mocha, the app's default.
pub fn palette(color: Color) -> Hsla {
    let hex = match color {
        Color::Spec(c) => ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32,
        Color::Indexed(i) if i < 16 => ANSI[i as usize],
        Color::Indexed(i) => {
            let i = i as u32;
            if i < 232 {
                let i = i - 16;
                let (r, g, b) = (i / 36, (i / 6) % 6, i % 6);
                let c = |v: u32| if v == 0 { 0 } else { 55 + v * 40 };
                (c(r) << 16) | (c(g) << 8) | c(b)
            } else {
                let v = 8 + (i - 232) * 10;
                (v << 16) | (v << 8) | v
            }
        }
        Color::Named(n) => match n {
            NamedColor::Foreground | NamedColor::BrightForeground => 0xcdd6f4,
            NamedColor::DimForeground => 0x9399b2,
            NamedColor::Background => 0x1e1e2e,
            NamedColor::Cursor => 0xf5e0dc,
            n => {
                let i = n as usize;
                if i < 16 { ANSI[i] } else { ANSI[i - NamedColor::DimBlack as usize] }
            }
        },
    };
    gpui::rgb(hex).into()
}

const ANSI: [u32; 16] = [
    0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
    0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
];

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Keystroke, Modifiers};

    fn key(k: &str, ch: Option<&str>, m: Modifiers) -> Keystroke {
        Keystroke { modifiers: m, key: k.into(), key_char: ch.map(Into::into) }
    }

    #[test]
    fn printable_keys_are_their_text() {
        assert_eq!(Terminal::encode_key(&key("a", Some("a"), Modifiers::default())), Some(b"a".to_vec()));
        assert_eq!(Terminal::encode_key(&key("enter", None, Modifiers::default())), Some(b"\r".to_vec()));
    }

    #[test]
    fn control_letters_are_control_bytes() {
        let m = Modifiers { control: true, ..Default::default() };
        assert_eq!(Terminal::encode_key(&key("c", None, m)), Some(vec![3]));
        assert_eq!(Terminal::encode_key(&key("d", None, m)), Some(vec![4]));
    }

    #[test]
    fn alt_arrows_are_word_movement() {
        let m = Modifiers { alt: true, ..Default::default() };
        assert_eq!(Terminal::encode_key(&key("left", None, m)), Some(b"\x1bb".to_vec()));
        assert_eq!(Terminal::encode_key(&key("left", None, Modifiers::default())), Some(b"\x1b[D".to_vec()));
    }
}
