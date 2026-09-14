//! Spike (2026-09-15): the canvas. Twelve terminal cards and three browser
//! cards in one world, a viewport over it, the Tauri app's zoom commands on
//! the Tauri app's keys.
//!
//! What the three earlier spikes proved, combined: terminals painted as
//! shaped lines at the zoomed font size (term-zoom), browsers painted from
//! CEF's off-screen frames (cef-frame), the moat applied so Google signs
//! in. New here: a viewport (viewport.rs, ported with its tests), animated
//! fits, hit testing and input routed to the card under the cursor with the
//! zoom undone, and card chrome sized in screen pixels.
//!
//! Keys, Cmd first like the app: `Cmd+=` / `Cmd+-` zoom, `Cmd+0` actual
//! size on the focused card, `Cmd+1` fit the focused card, `Cmd+2` fit all,
//! `Cmd+scroll` zooms at the cursor, `Cmd+drag` or middle-drag pans. A bare
//! click focuses the card under it; everything else typed goes to the
//! focused card, `Cmd+V/C/X/A/Z` included.
mod browser;
mod chrome_moat;
mod terminal;
mod viewport;

use browser::{BrowserCard, VIEW_H, VIEW_W};
use cef::{args::Args, *};
use gpui::{
    canvas, div, fill, font, outline, point, prelude::*, px, rgb, size, AsyncApp, BorderStyle, Bounds, Context,
    FocusHandle, Font, Hsla, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Render, ScrollDelta, ScrollWheelEvent, SharedString, TextRun, TitlebarOptions, WindowBounds, WindowOptions,
};
use std::time::{Duration, Instant};
use terminal::{Terminal, COLS, FONT_PX, LINE_HEIGHT, ROWS};
use viewport::{
    anchored_viewport, bounding_rect, centre_on, fit_frame, fit_rect, viewport_centre, world_pos_of, Point, Rect,
    Size, Viewport, FIT_DURATION_MS, ZOOM_DURATION_MS,
};

const GAP: f32 = 32.0;
const BUDGET_PER_FRAME: usize = 256 * 1024;
/// Cmd+drag becomes a pan only after this much movement, so a Cmd+click
/// still reaches the page as a click.
const DRAG_SLOP: f32 = 4.0;
const BORDER_PX: f32 = 2.0;

const PAGES: [(&str, &str); 3] = [
    ("hacker news", "https://news.ycombinator.com"),
    ("wikipedia", "https://en.wikipedia.org/wiki/Terminal_emulator"),
    ("zed", "https://github.com/zed-industries/zed"),
];
const TERMINALS: usize = 12;

enum Body {
    Terminal(Terminal),
    Browser(BrowserCard),
}

struct Card {
    name: String,
    rect: Rect,
    body: Body,
}

struct Anim {
    from: Viewport,
    to: Viewport,
    started: Instant,
    duration_ms: f32,
}

enum Pan {
    /// Cmd+left is down and has not moved past the slop yet.
    Pending(Point),
    Dragging(Point),
}

struct Canvas {
    cards: Vec<Card>,
    viewport: Viewport,
    anim: Option<Anim>,
    focused: Option<usize>,
    pan: Option<Pan>,
    view_size: Size,
    focus: FocusHandle,
    font: Font,
    cell_w: f32,
    frames: u32,
    window_start: Instant,
    fps: f32,
}

impl Canvas {
    fn screen_rect(&self, rect: Rect) -> Bounds<Pixels> {
        let vp = self.viewport;
        Bounds::new(
            point(px((rect.x - vp.x) * vp.scale), px((rect.y - vp.y) * vp.scale)),
            size(px(rect.w * vp.scale), px(rect.h * vp.scale)),
        )
    }

    fn card_at(&self, screen: Point) -> Option<usize> {
        let world = world_pos_of(screen, self.viewport);
        self.cards.iter().position(|c| {
            world.x >= c.rect.x && world.x < c.rect.x + c.rect.w && world.y >= c.rect.y && world.y < c.rect.y + c.rect.h
        })
    }

    /// A screen point as page/cell coordinates inside a card.
    fn local(&self, i: usize, screen: Point) -> Point {
        let world = world_pos_of(screen, self.viewport);
        Point { x: world.x - self.cards[i].rect.x, y: world.y - self.cards[i].rect.y }
    }

    fn animate_to(&mut self, to: Viewport, duration_ms: f32) {
        self.anim = Some(Anim { from: self.viewport, to, started: Instant::now(), duration_ms });
    }

    fn step_animation(&mut self) {
        if let Some(a) = &self.anim {
            let t = a.started.elapsed().as_secs_f32() * 1000.0 / a.duration_ms;
            if t >= 1.0 {
                self.viewport = a.to;
                self.anim = None;
            } else {
                self.viewport = fit_frame(a.from, a.to, self.view_size, t);
            }
        }
    }

    fn zoom_by(&mut self, factor: f32) {
        let centre = viewport_centre(self.viewport, self.view_size);
        let to = centre_on(centre, self.viewport.scale * factor, self.view_size);
        self.animate_to(to, ZOOM_DURATION_MS);
    }

    fn fit_focused(&mut self) {
        if let Some(i) = self.focused {
            let to = fit_rect(self.cards[i].rect, self.view_size);
            self.animate_to(to, FIT_DURATION_MS);
        }
    }

    fn fit_all(&mut self) {
        let rects: Vec<Rect> = self.cards.iter().map(|c| c.rect).collect();
        if let Some(b) = bounding_rect(&rects) {
            let to = fit_rect(b, self.view_size);
            self.animate_to(to, FIT_DURATION_MS);
        }
    }

    fn actual_size(&mut self) {
        let point = match self.focused {
            Some(i) => {
                let r = self.cards[i].rect;
                Point { x: r.x + r.w / 2.0, y: r.y + r.h / 2.0 }
            }
            None => viewport_centre(self.viewport, self.view_size),
        };
        let to = centre_on(point, 1.0, self.view_size);
        self.animate_to(to, ZOOM_DURATION_MS);
    }

    fn set_focus(&mut self, i: Option<usize>) {
        if self.focused == i {
            return;
        }
        if let Some(Body::Browser(b)) = self.focused.map(|f| &self.cards[f].body) {
            b.focus(false);
        }
        self.focused = i;
        if let Some(Body::Browser(b)) = i.map(|f| &self.cards[f].body) {
            b.focus(true);
        }
    }

    fn key(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.platform {
            match k.key.as_str() {
                "=" | "+" => self.zoom_by(1.2),
                "-" => self.zoom_by(1.0 / 1.2),
                "0" => self.actual_size(),
                "1" => self.fit_focused(),
                "2" => self.fit_all(),
                other => {
                    let Some(i) = self.focused else { return };
                    let text = cx.read_from_clipboard().and_then(|c| c.text());
                    match &mut self.cards[i].body {
                        Body::Browser(b) => {
                            b.edit_chord(other);
                        }
                        Body::Terminal(t) => {
                            if other == "v" {
                                if let Some(text) = text {
                                    t.write(text.as_bytes());
                                }
                            }
                        }
                    }
                }
            }
            return;
        }
        let Some(i) = self.focused else { return };
        match &mut self.cards[i].body {
            Body::Browser(b) => b.key(k),
            Body::Terminal(t) => {
                if let Some(bytes) = Terminal::encode_key(k) {
                    t.write(&bytes);
                }
            }
        }
    }

    fn mouse_down(&mut self, e: &MouseDownEvent) {
        let p = Point { x: f32::from(e.position.x), y: f32::from(e.position.y) };
        match e.button {
            MouseButton::Middle => {
                self.pan = Some(Pan::Dragging(p));
                return;
            }
            MouseButton::Left if e.modifiers.platform => {
                self.pan = Some(Pan::Pending(p));
            }
            MouseButton::Left => {}
            _ => return,
        }
        let hit = self.card_at(p);
        self.set_focus(hit);
        if let Some(i) = hit {
            let l = self.local(i, p);
            if let Body::Browser(b) = &self.cards[i].body {
                b.mouse_button(l.x, l.y, &e.modifiers, false, e.click_count);
            }
        }
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent) {
        let p = Point { x: f32::from(e.position.x), y: f32::from(e.position.y) };
        match self.pan {
            Some(Pan::Pending(start)) => {
                if (p.x - start.x).abs() > DRAG_SLOP || (p.y - start.y).abs() > DRAG_SLOP {
                    self.pan = Some(Pan::Dragging(start));
                }
                return;
            }
            Some(Pan::Dragging(last)) => {
                self.viewport.x -= (p.x - last.x) / self.viewport.scale;
                self.viewport.y -= (p.y - last.y) / self.viewport.scale;
                self.pan = Some(Pan::Dragging(p));
                self.anim = None;
                return;
            }
            None => {}
        }
        if let Some(i) = self.card_at(p) {
            let l = self.local(i, p);
            if let Body::Browser(b) = &self.cards[i].body {
                b.mouse_move(l.x, l.y, &e.modifiers);
            }
        }
    }

    fn mouse_up(&mut self, e: &MouseUpEvent) {
        let p = Point { x: f32::from(e.position.x), y: f32::from(e.position.y) };
        let was_dragging = matches!(self.pan, Some(Pan::Dragging(_)));
        self.pan = None;
        if was_dragging {
            return;
        }
        if let Some(i) = self.card_at(p) {
            let l = self.local(i, p);
            if let Body::Browser(b) = &self.cards[i].body {
                b.mouse_button(l.x, l.y, &e.modifiers, true, e.click_count);
            }
        }
    }

    fn wheel(&mut self, e: &ScrollWheelEvent) {
        let p = Point { x: f32::from(e.position.x), y: f32::from(e.position.y) };
        let (dx, dy) = match e.delta {
            ScrollDelta::Pixels(d) => (f32::from(d.x), f32::from(d.y)),
            ScrollDelta::Lines(l) => (l.x * 20.0, l.y * 20.0),
        };
        if e.modifiers.platform {
            // Zoom at the cursor: the world point under it stays under it.
            let world = world_pos_of(p, self.viewport);
            let factor = (1.0 + dy / 200.0).clamp(0.5, 2.0);
            let scale = (self.viewport.scale * factor).clamp(viewport::MIN_SCALE, viewport::MAX_SCALE);
            self.viewport = anchored_viewport(world, p, scale);
            self.anim = None;
            return;
        }
        if let Some(i) = self.card_at(p) {
            let l = self.local(i, p);
            if let Body::Browser(b) = &self.cards[i].body {
                b.wheel(l.x, l.y, &e.modifiers, dx / self.viewport.scale, dy / self.viewport.scale);
            }
        }
    }
}

impl Render for Canvas {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frames += 1;
        if self.window_start.elapsed() >= Duration::from_secs(1) {
            self.fps = self.frames as f32 / self.window_start.elapsed().as_secs_f32();
            eprintln!("[spike] {:.0} fps  zoom {:.2}", self.fps, self.viewport.scale);
            self.frames = 0;
            self.window_start = Instant::now();
        }
        let focused_name = self.focused.map(|i| self.cards[i].name.clone()).unwrap_or_else(|| "none".into());
        let status = format!(
            "{:.0} fps  zoom {:.2}  focused: {focused_name}   [Cmd+= / Cmd+-  Cmd+0 actual  Cmd+1 fit card  Cmd+2 fit all  Cmd+scroll  Cmd+drag]",
            self.fps, self.viewport.scale
        );
        let entity = cx.entity();

        div()
            .size_full()
            .bg(rgb(0x11111b))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| this.key(e, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, window, _| {
                window.focus(&this.focus);
                this.mouse_down(e)
            }))
            .on_mouse_down(MouseButton::Middle, cx.listener(|this, e: &MouseDownEvent, _, _| this.mouse_down(e)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, e: &MouseUpEvent, _, _| this.mouse_up(e)))
            .on_mouse_up(MouseButton::Middle, cx.listener(|this, e: &MouseUpEvent, _, _| this.mouse_up(e)))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, _| this.mouse_move(e)))
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, _| this.wheel(e)))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        entity.update(cx, |this, cx| {
                            this.view_size = Size { w: f32::from(bounds.size.width), h: f32::from(bounds.size.height) };
                            this.step_animation();
                            // Parse first, one budget for the frame.
                            let terminals = this.cards.iter().filter(|c| matches!(c.body, Body::Terminal(_))).count().max(1);
                            let per = BUDGET_PER_FRAME / terminals;
                            for card in this.cards.iter_mut() {
                                if let Body::Terminal(t) = &mut card.body {
                                    t.drain(per);
                                }
                            }
                            let scale = this.viewport.scale;
                            let accent: Hsla = rgb(0x89b4fa).into();
                            let dim: Hsla = rgb(0x45475a).into();
                            let label_bg: Hsla = rgb(0x313244).into();
                            let label_fg: Hsla = rgb(0xcdd6f4).into();
                            let view = Bounds::new(point(px(0.0), px(0.0)), bounds.size);
                            for i in 0..this.cards.len() {
                                let rect = this.screen_rect(this.cards[i].rect);
                                if !view.intersects(&rect) {
                                    continue;
                                }
                                match &this.cards[i].body {
                                    Body::Terminal(t) => t.paint(rect, scale, &this.font, window, cx),
                                    Body::Browser(b) => {
                                        let frame = b.shared.borrow().frame.clone();
                                        match frame {
                                            Some(f) => {
                                                let _ = window.paint_image(rect, Default::default(), f, 0, false);
                                            }
                                            None => window.paint_quad(fill(rect, label_bg)),
                                        }
                                    }
                                }
                                // Chrome in SCREEN pixels: a 2px border is 2px at every zoom.
                                let color = if this.focused == Some(i) { accent } else { dim };
                                window.paint_quad(outline(rect, color, BorderStyle::Solid).border_widths(px(BORDER_PX)));
                                let label = window.text_system().shape_line(
                                    SharedString::from(this.cards[i].name.clone()),
                                    px(12.0),
                                    &[TextRun {
                                        len: this.cards[i].name.len(),
                                        font: this.font.clone(),
                                        color: label_fg,
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                    }],
                                    None,
                                );
                                let label_bounds = Bounds::new(
                                    point(rect.origin.x + px(BORDER_PX), rect.origin.y + px(BORDER_PX)),
                                    size(label.width + px(12.0), px(20.0)),
                                );
                                window.paint_quad(fill(label_bounds, label_bg));
                                let _ = label.paint(point(label_bounds.origin.x + px(6.0), label_bounds.origin.y), px(20.0), window, cx);
                            }
                        });
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
    let _loader = {
        let loader = library_loader::LibraryLoader::new(&std::env::current_exe().unwrap(), false);
        assert!(loader.load(), "CEF framework not found beside the executable; run from the bundle");
        loader
    };
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = Args::new();
    let mut app = browser::AppBuilder::new();
    let ret = execute_process(Some(args.as_main_args()), Some(&mut app), std::ptr::null_mut());
    if ret >= 0 {
        std::process::exit(ret);
    }
    let profile = browser::spike_dir().join("profile");
    let settings = Settings {
        windowless_rendering_enabled: 1,
        external_message_pump: 1,
        cache_path: profile.to_string_lossy().as_ref().into(),
        ..Default::default()
    };

    gpui::Application::new().run(move |cx: &mut gpui::App| {
        browser::cef_app_protocol::install();
        assert_eq!(initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), std::ptr::null_mut()), 1);

        let bounds = Bounds::centered(None, size(px(1900.0), px(1150.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions { title: Some("canvas".into()), ..Default::default() }),
                ..Default::default()
            },
            |window, cx| {
                let font_ = font("Menlo");
                let probe = window.text_system().shape_line(
                    "M".into(),
                    px(FONT_PX),
                    &[TextRun { len: 1, font: font_.clone(), color: gpui::black(), background_color: None, underline: None, strikethrough: None }],
                    None,
                );
                let cell_w: f32 = probe.width.into();
                let scale_factor = window.scale_factor();

                // Layout: the browsers in the first row, the terminals in a
                // 4-wide grid below. Positions are world units.
                let mut cards = Vec::new();
                let mut x = 0.0;
                for (name, url) in PAGES {
                    cards.push(Card {
                        name: name.into(),
                        rect: Rect { x, y: 0.0, w: VIEW_W as f32, h: VIEW_H as f32 },
                        body: Body::Browser(BrowserCard::open(url, scale_factor)),
                    });
                    x += VIEW_W as f32 + GAP;
                }
                let term_w = cell_w * COLS as f32;
                let term_h = FONT_PX * LINE_HEIGHT * ROWS as f32;
                let home = std::env::var("HOME").ok();
                for i in 0..TERMINALS {
                    let col = (i % 4) as f32;
                    let row = (i / 4) as f32;
                    let mut t = Terminal::spawn(home.as_deref());
                    t.write(format!("clear; echo terminal {}; ls -la ~/Code | head -{}\n", i + 1, 8 + i * 2).as_bytes());
                    cards.push(Card {
                        name: format!("terminal {}", i + 1),
                        rect: Rect {
                            x: col * (term_w + GAP),
                            y: VIEW_H as f32 + GAP + row * (term_h + GAP),
                            w: term_w,
                            h: term_h,
                        },
                        body: Body::Terminal(t),
                    });
                }
                let all = bounding_rect(&cards.iter().map(|c| c.rect).collect::<Vec<_>>()).unwrap();
                let view_size = Size { w: 1900.0, h: 1150.0 };
                cx.new(|cx| Canvas {
                    cards,
                    viewport: fit_rect(all, view_size),
                    anim: None,
                    focused: None,
                    pan: None,
                    view_size,
                    focus: cx.focus_handle(),
                    font: font_,
                    cell_w,
                    frames: 0,
                    window_start: Instant::now(),
                    fps: 0.0,
                })
            },
        )
        .expect("window");
        cx.activate(true);

        cx.spawn(async move |cx: &mut AsyncApp| loop {
            do_message_loop_work();
            cx.background_executor().timer(Duration::from_millis(4)).await;
        })
        .detach();
    });
    shutdown();
}
