//! Spike (#356): one card in a window of its own, without the canvas.
//!
//! `CardWindow` is a thin view. It holds the one `AppView` entity and the
//! card's id, and forwards every key, click, wheel and composed character to
//! the card's body in `AppView::bodies`. The model, the backend and the
//! canvas stay where they are; the card keeps its slot and paints a
//! placeholder there (paint.rs, `AppView::extracted`).
//!
//! Opened from the poll task in main.rs (`AppView::extract_request`), never
//! from inside an `AppView` update: a second window that touches the app
//! while the app is borrowed aborts with `RefCell already borrowed`.
//!
//! Related: body.rs (the trait the body answers), ime.rs (the same
//! `EntityInputHandler` on the main view), terminals.rs (`extracted_size`
//! sizes the grid to this window), main.rs (the poll task that refreshes
//! the window when its body has output).
//!
//! Not built, on purpose: the registry and its commands, find and the status
//! bar in this window, the editor, the Cmd+Escape lock, and the second
//! window's own fullscreen and frame memory. Those are the issue's
//! "not decided" list.

use std::ops::Range;

use gpui::{
    canvas, div, prelude::*, px, size, App, Bounds, Context, Entity, EntityInputHandler,
    FocusHandle, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Render, ScrollWheelEvent, TitlebarOptions, UTF16Selection, Window,
    WindowBounds, WindowHandle, WindowOptions,
};

use crate::body::BodyAction;
use crate::AppView;

pub struct CardWindow {
    pub app: Entity<AppView>,
    pub card: String,
    pub focus: FocusHandle,
    /// Text an input method is composing in this window (`ime.rs` does the
    /// same for the main window).
    composing: Option<String>,
}

impl CardWindow {
    /// Opens the card in its own window and returns its handle. The handle
    /// is kept by `AppView::extracted`; closing the window (the red button
    /// or Cmd+W) gives the card back to the canvas.
    pub fn open(
        app: Entity<AppView>,
        card: String,
        cx: &mut App,
    ) -> Option<WindowHandle<CardWindow>> {
        let bounds = Bounds::centered(None, size(px(900.), px(600.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("infiniterm card".into()),
                appears_transparent: false,
                traffic_light_position: None,
            }),
            ..Default::default()
        };
        cx.open_window(options, move |window, cx| {
            let focus = cx.focus_handle();
            let closing = app.clone();
            let closing_card = card.clone();
            window.on_window_should_close(cx, move |_, cx| {
                closing.update(cx, |a, cx| a.extracted_closed(&closing_card, cx));
                true
            });
            let view = cx.new(|_| CardWindow {
                app,
                card,
                focus: focus.clone(),
                composing: None,
            });
            window.focus(&focus);
            view
        })
        .ok()
    }

    /// Back to the canvas: the window goes, the card's slot shows the card
    /// again, and its grid goes back to the card's rect.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let card = self.card.clone();
        self.app.update(cx, |a, cx| a.extracted_closed(&card, cx));
        window.remove_window();
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let m = &k.modifiers;
        // A dead key is composed by macOS through `replace_text_in_range`,
        // not here; see ime.rs for why the NSEvent decides it.
        if crate::keycode::last_dead() && !m.platform && !m.control {
            return;
        }
        if m.platform && k.key == "w" {
            self.close(window, cx);
            cx.stop_propagation();
            return;
        }
        let card = self.card.clone();
        let now = crate::now_ms();
        let taken = self.app.update(cx, |app, cx| {
            app.bodies.get_mut(&card).map(|b| b.key(k, now, cx))
        });
        cx.notify();
        if matches!(taken, None | Some(BodyAction::Ignored)) {
            return;
        }
        cx.stop_propagation();
    }

    /// A body's answer that needs the main app (opening a card, a file) is
    /// not handled here yet: the spike is for keys, mouse, draw and resize.
    fn with_body<R>(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut dyn crate::body::CardBody, &mut App) -> R,
    ) -> Option<R> {
        let card = self.card.clone();
        self.app.update(cx, |app, cx| {
            app.bodies.get_mut(&card).map(|b| f(b.as_mut(), cx))
        })
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, cx: &mut Context<Self>) {
        let (local, button, mods, clicks) =
            (card_point(e.position), e.button, e.modifiers, e.click_count);
        self.with_body(cx, |b, _| b.mouse_down(local, button, &mods, clicks));
        cx.notify();
    }

    fn mouse_up(&mut self, e: &MouseUpEvent, cx: &mut Context<Self>) {
        let (local, button, mods) = (card_point(e.position), e.button, e.modifiers);
        self.with_body(cx, |b, _| b.mouse_up(local, button, &mods));
        cx.notify();
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let (local, mods) = (card_point(e.position), e.modifiers);
        self.with_body(cx, |b, _| b.mouse_move(local, &mods));
    }

    fn scroll(&mut self, e: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta: Point<Pixels> = e.delta.pixel_delta(px(16.));
        let (local, mods) = (card_point(e.position), e.modifiers);
        self.with_body(cx, |b, _| {
            b.wheel(
                local,
                f32::from(delta.x) as f64,
                f32::from(delta.y) as f64,
                &mods,
            )
        });
        cx.notify();
    }
}

impl Render for CardWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The card's grid is fitted to this window, in the card's own world
        // units, which at scale 1 are window points. A new size reflows the
        // shell through terminals.rs on the main view's next frame.
        let viewport = window.viewport_size();
        let world = infiniterm_core::grid::Size {
            w: f32::from(viewport.width) as f64,
            h: f32::from(viewport.height) as f64,
        };
        // The grid is this window's: it refits on every resize, and the card
        // gets its canvas size back when the window closes (Ekin, 2026-10-11).
        let card = self.card.clone();
        self.app.update(cx, |app, cx| {
            if app.extracted_size.get(&card) != Some(&world) {
                app.extracted_size.insert(card, world);
                cx.notify();
            }
        });

        let app = self.app.clone();
        let paint_card = self.card.clone();
        let focus = self.focus.clone();
        let this = cx.entity();
        div()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(
                cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| this.mouse_down(e, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, e: &MouseUpEvent, _, cx| this.mouse_up(e, cx)),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| this.mouse_move(e, cx)))
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, cx| this.scroll(e, cx)))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        // Registered every paint, as overlays.rs does for the
                        // main view: the input system asks again each frame.
                        window.handle_input(
                            &focus,
                            gpui::ElementInputHandler::new(bounds, this.clone()),
                            cx,
                        );
                        let now = crate::now_ms();
                        let started = std::time::Instant::now();
                        let wants = app.update(cx, |app, cx| {
                            // The output is parsed in a frame, and the main
                            // canvas is not drawing while this window has the
                            // keys: feed what the program wrote here (#356
                            // spike; Ekin saw frozen content and lag).
                            // The refit happens in the canvas's frame too, which
                            // is not drawing; this window does it for its card.
                            app.reconcile_terminals(window);
                            app.feed_terminals(now, cx);
                            let body = app.bodies.get_mut(&paint_card)?;
                            body.paint(bounds, 1., true, now, window, cx);
                            Some(body.wants_frame(now))
                        });
                        if std::env::var_os("INFINITERM_KEYLOG").is_some() {
                            eprintln!("[own-window] paint {} us", started.elapsed().as_micros());
                        }
                        if wants == Some(true) {
                            window.request_animation_frame();
                        }
                    },
                )
                .size_full(),
            )
    }
}

impl EntityInputHandler for CardWindow {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        self.composing.clone()
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self
            .composing
            .as_deref()
            .map(|m| m.encode_utf16().count())
            .unwrap_or(0);
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.composing
            .as_deref()
            .map(|m| 0..m.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.composing = None;
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composing = None;
        if text.is_empty() {
            return;
        }
        self.with_body(cx, |b, _| b.insert_text(text));
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composing = (!new_text.is_empty()).then(|| new_text.to_string());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(Bounds::new(element_bounds.origin, size(px(0.), px(0.))))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

/// A window point as the body's own point. The window is the card, so the
/// origin is the card's and the scale is 1 (`CardBody::paint` is called with
/// the window's bounds at 1).
fn card_point(p: Point<Pixels>) -> infiniterm_core::grid::Point {
    infiniterm_core::grid::Point {
        x: f32::from(p.x) as f64,
        y: f32::from(p.y) as f64,
    }
}

impl AppView {
    /// The card's window closed (the red button, or Cmd+W in it): the card
    /// goes back to the canvas and its grid to its rect.
    pub fn extracted_closed(&mut self, card: &str, cx: &mut Context<Self>) {
        self.extracted.remove(card);
        self.extracted_size.remove(card);
        cx.notify();
    }
}
