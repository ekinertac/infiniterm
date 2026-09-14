//! Spike (2026-09-14): 25 terminals drawn by gpui while the zoom animates.
//!
//! The question the port hangs on for terminals: gpui shapes text at a
//! pixel font size, so a continuous zoom means every visible line is
//! reshaped at a new size each frame and the glyph atlas fills with new
//! rasterisations. Does that hold 60 fps with 25 cards, idle and flooding?
//! The Tauri app's numbers to beat, from CLAUDE.md: 25 idle cards zooming
//! at 60, 25 flooding at 45-60.
//!
//! Each pane is an `alacritty_terminal::Term` fed by its own PTY on
//! alacritty's event loop thread; the view locks each term once per frame
//! and paints rows as shaped lines onto a `canvas`. Zoom is the only
//! transform: font size, cell size and card origins are all multiplied.
//! Keys: `1` idle (`ls` output), `2` flood (`yes`), `space` pause the zoom.
//! Frame rate goes to stderr once a second and to the status line.
//!
//! Second shape, after the first one measured 1 fps under flood: the first
//! version used alacritty's own event loop, one thread per pane parsing
//! greedily under a FairMutex, and the renderer spent 40 to 400 ms per
//! frame waiting for those locks while shaping took 3 ms. This version is
//! the Tauri app's shape instead: reader threads only move bytes into a
//! bounded channel (the child blocks on the kernel PTY buffer when it
//! fills, the credit idea), and the main thread parses a bounded number
//! of bytes per frame before painting. No lock, no contention.
use alacritty_terminal::{
    event::{Event, EventListener, WindowSize},
    grid::Dimensions,
    term::{cell::Flags, Config, Term},
    tty,
    vte::ansi::{Color, NamedColor, Processor},
};
use gpui::{
    canvas, div, fill, font, prelude::*, px, rgb, size, Application, Bounds, Context, FocusHandle,
    Font, Hsla, KeyDownEvent, Render, SharedString, TextRun, TitlebarOptions, Window,
    WindowBounds, WindowOptions,
};
use std::{
    cell::RefCell,
    collections::VecDeque,
    fs::File,
    io::{Read, Write},
    rc::Rc,
    sync::mpsc::{sync_channel, Receiver},
    time::{Duration, Instant},
};

const PANES: usize = 25;
const GRID: usize = 5;
const COLS: usize = 80;
const ROWS: usize = 40;
const FONT_PX: f32 = 14.0;
const LINE_HEIGHT: f32 = 1.2;
const GAP: f32 = 24.0;
const FONT_FAMILY: &str = "Menlo";

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

struct Pane {
    term: Term<Listener>,
    processor: Processor,
    rx: Receiver<Vec<u8>>,
    writer: File,
    /// bytes received but not yet parsed, the per-pane queue the budget drains
    pending: VecDeque<Vec<u8>>,
    /// bytes parsed since start, for the log
    parsed: u64,
}

/// Bytes parsed per frame across all panes. The Tauri app's scheduler
/// floors at 32 KiB and tunes on the frame gap; a fixed budget is enough
/// here to see whether the shape holds.
const BUDGET_PER_FRAME: usize = 256 * 1024;

fn spawn_pane() -> Pane {
    let term = Term::new(Config::default(), &TermSize, Listener);
    let opts = tty::Options {
        shell: Some(tty::Shell::new("/bin/zsh".into(), vec!["-f".into()])),
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
    // Eight chunks of 64 KiB in flight, then the reader blocks on send and
    // the child blocks on the PTY: backpressure without a byte counter.
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
                // alacritty opens the master non-blocking; a poll is fine here
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    });
    Pane { term, processor: Processor::new(), rx, writer, pending: VecDeque::new(), parsed: 0 }
}

impl Pane {
    /// Parses up to `budget` bytes and returns how many it used.
    fn drain(&mut self, budget: usize) -> usize {
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
        self.parsed += used as u64;
        used
    }
}

/// Catppuccin Mocha, the app's default, so the spike looks like the app.
fn palette(color: Color) -> Hsla {
    let hex = match color {
        Color::Spec(rgb_) => ((rgb_.r as u32) << 16) | ((rgb_.g as u32) << 8) | rgb_.b as u32,
        Color::Indexed(i) if i < 16 => ANSI[i as usize],
        Color::Indexed(i) => {
            // 6x6x6 cube and the grey ramp, the usual xterm mapping
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
    rgb(hex).into()
}

const ANSI: [u32; 16] = [
    0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
    0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
];

struct Root {
    panes: Rc<RefCell<Vec<Pane>>>,
    focus: FocusHandle,
    font: Font,
    /// cell width at FONT_PX, measured once by shaping "M"
    cell_w: f32,
    started: Instant,
    zooming: bool,
    zoom_at_pause: f32,
    frames: u32,
    window_start: Instant,
    fps: f32,
    /// ms spent inside the paint closure for the last frame
    paint_ms: f32,
}

impl Root {
    fn zoom(&self) -> f32 {
        if !self.zooming {
            return self.zoom_at_pause;
        }
        let t = self.started.elapsed().as_secs_f32();
        // 0.35 .. 1.15, one round trip every 4 s: a pinch, not a slider
        0.75 + 0.4 * (t * std::f32::consts::TAU / 4.0).sin()
    }

    fn send_all(&self, bytes: &[u8]) {
        for p in self.panes.borrow_mut().iter_mut() {
            let _ = p.writer.write_all(bytes);
        }
    }
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // fps over one-second windows, printed so a log survives the run
        self.frames += 1;
        if self.window_start.elapsed() >= Duration::from_secs(1) {
            self.fps = self.frames as f32 / self.window_start.elapsed().as_secs_f32();
            eprintln!(
                "[spike] {:.0} fps  paint {:.1} ms  zoom {:.2}",
                self.fps,
                self.paint_ms,
                self.zoom()
            );
            self.frames = 0;
            self.window_start = Instant::now();
        }
        let zoom = self.zoom();
        let status = format!(
            "{:.0} fps  paint {:.1} ms  zoom {zoom:.2}  {} panes {COLS}x{ROWS}  [1 idle  2 flood  space pause]",
            self.fps, self.paint_ms, PANES
        );

        let panes = self.panes.clone();
        let font_ = self.font.clone();
        let cell_w = self.cell_w;
        let entity = cx.entity().downgrade();

        div()
            .size_full()
            .bg(rgb(0x11111b))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                match e.keystroke.key.as_str() {
                    "1" => this.send_all(b"\x03clear; ls -la /usr/lib | head -60\n"),
                    "2" => this.send_all(b"\x03yes\n"),
                    "space" => {
                        this.zoom_at_pause = this.zoom();
                        this.zooming = !this.zooming;
                    }
                    _ => return,
                }
                cx.notify();
            }))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        let started = Instant::now();
                        let mut panes = panes.borrow_mut();
                        // Parse first, round robin, one budget for the frame.
                        let per_pane = BUDGET_PER_FRAME / panes.len();
                        let mut parsed = 0;
                        for pane in panes.iter_mut() {
                            parsed += pane.drain(per_pane);
                        }
                        let parse_time = started.elapsed();
                        let mut shape_time = Duration::ZERO;
                        let font_size = px(FONT_PX * zoom);
                        let line_h = px(FONT_PX * LINE_HEIGHT * zoom);
                        let card_w = cell_w * COLS as f32 * zoom;
                        let card_h = FONT_PX * LINE_HEIGHT * ROWS as f32 * zoom;
                        let bg: Hsla = rgb(0x1e1e2e).into();
                        for (i, pane) in panes.iter().enumerate() {
                            let ox = bounds.origin.x + px((i % GRID) as f32 * (card_w + GAP * zoom));
                            let oy = bounds.origin.y + px((i / GRID) as f32 * (card_h + GAP * zoom));
                            window.paint_quad(fill(
                                Bounds::new(gpui::point(ox, oy), size(px(card_w), px(card_h))),
                                bg,
                            ));
                            let content = pane.term.renderable_content();
                            // One shaped line per row; consecutive cells with the
                            // same colours share a run, the way a renderer would.
                            let mut rows: Vec<(String, Vec<TextRun>)> =
                                (0..ROWS).map(|_| (String::with_capacity(COLS), Vec::new())).collect();
                            for cell in content.display_iter {
                                let row = cell.point.line.0 as usize;
                                if row >= ROWS || cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                                    continue;
                                }
                                let (text, runs) = &mut rows[row];
                                let (fg, bg_) = if cell.flags.contains(Flags::INVERSE) {
                                    (cell.bg, cell.fg)
                                } else {
                                    (cell.fg, cell.bg)
                                };
                                let color = palette(fg);
                                let background = match bg_ {
                                    Color::Named(NamedColor::Background) => None,
                                    other => Some(palette(other)),
                                };
                                let len = cell.c.len_utf8();
                                text.push(cell.c);
                                match runs.last_mut() {
                                    Some(run) if run.color == color && run.background_color == background => {
                                        run.len += len
                                    }
                                    _ => runs.push(TextRun {
                                        len,
                                        font: font_.clone(),
                                        color,
                                        background_color: background,
                                        underline: None,
                                        strikethrough: None,
                                    }),
                                }
                            }
                            let shape_started = Instant::now();
                            for (r, (text, runs)) in rows.into_iter().enumerate() {
                                if text.trim().is_empty() {
                                    continue;
                                }
                                let line = window.text_system().shape_line(
                                    SharedString::from(text),
                                    font_size,
                                    &runs,
                                    None,
                                );
                                let origin = gpui::point(ox, oy + line_h * r as f32);
                                let _ = line.paint(origin, line_h, window, cx);
                            }
                            shape_time += shape_started.elapsed();
                        }
                        if started.elapsed() > Duration::from_millis(30) || parsed > 0 && started.elapsed() > Duration::from_millis(12) {
                            eprintln!(
                                "[spike] frame {:.0} ms: parse {} KiB in {:.1} ms, shape+paint {:.1} ms",
                                started.elapsed().as_secs_f32() * 1000.0,
                                parsed / 1024,
                                parse_time.as_secs_f32() * 1000.0,
                                shape_time.as_secs_f32() * 1000.0
                            );
                        }
                        if let Some(root) = entity.upgrade() {
                            root.update(cx, |root, _| root.paint_ms = started.elapsed().as_secs_f32() * 1000.0);
                        }
                        window.request_animation_frame();
                    },
                )
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .px_2()
                    .py_1()
                    .bg(rgb(0x11111b))
                    .text_color(rgb(0xcdd6f4))
                    .text_size(px(12.0))
                    .child(status),
            )
    }
}

fn main() {
    Application::new().run(|cx| {
        let bounds = Bounds::centered(None, size(px(1900.0), px(1150.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions { title: Some("term-zoom".into()), ..Default::default() }),
                ..Default::default()
            },
            |window, cx| {
                let font_ = font(FONT_FAMILY);
                let probe = window.text_system().shape_line(
                    "M".into(),
                    px(FONT_PX),
                    &[TextRun {
                        len: 1,
                        font: font_.clone(),
                        color: gpui::black(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                );
                let cell_w: f32 = probe.width.into();
                eprintln!("[spike] cell width {cell_w} px at {FONT_PX} px");
                let panes = Rc::new(RefCell::new((0..PANES).map(|_| spawn_pane()).collect::<Vec<Pane>>()));
                let root = cx.new(|cx| Root {
                    panes,
                    focus: cx.focus_handle(),
                    font: font_,
                    cell_w,
                    started: Instant::now(),
                    zooming: true,
                    zoom_at_pause: 1.0,
                    frames: 0,
                    window_start: Instant::now(),
                    fps: 0.0,
                    paint_ms: 0.0,
                });
                // Something to look at before a key is pressed.
                root.update(cx, |root, _| root.send_all(b"clear; ls -la /usr/lib | head -60\n"));
                root
            },
        )
        .expect("window");
        cx.activate(true);
    });
}
