//! The "what's new" overlay (#402): the changelog, newest release first and
//! without the Unreleased days, in a sheet over the canvas on the first
//! launch after an update. Also `Help: show what's new` (`help.whatsNew`).
//!
//! The text is a `PageBody` (the Page card's Markdown layout and scroll,
//! `page_body.rs`) reading the file `runtime.rs` writes at launch
//! (`infiniterm_core::changelog::whats_new_path`); the sheet only gives it a
//! place to paint. It is modal like the shortcuts panel: while it is up no
//! key reaches the canvas behind it. Escape and the X in its corner close
//! it; the arrows, Page Up/Down, Home and End scroll it; any other key, or a
//! click outside, only makes the sheet flash (`Model::flash_whats_new`), so a
//! person typing as it opens sees why nothing happened.
//!
//! Called by `overlays.rs` (render), `input.rs` and `ime.rs` (keys and
//! composed text), `editors.rs` (`reconcile_editors` builds and drops the
//! body). Related: `page_body.rs`, `infiniterm-core/src/changelog.rs`.
use crate::page_body::{PageBody, PageColors};
use crate::AppView;
use gpui::{
    canvas, div, prelude::*, px, App, Context, Keystroke, MouseButton, MouseDownEvent,
    ScrollWheelEvent, Window,
};
use infiniterm_core::grid::Size;

/// The sheet's widest and tallest, and the margin kept to the window's edge.
const SHEET_MAX_W_PX: f32 = 900.;
const SHEET_MAX_H_PX: f32 = 720.;
const SHEET_MARGIN_PX: f32 = 80.;
/// Smallest it shrinks to in a tiny window.
const SHEET_MIN_W_PX: f32 = 320.;
const SHEET_MIN_H_PX: f32 = 240.;
/// A wheel tick, for a mouse that reports lines.
const LINE_PX: f32 = 16.;

impl AppView {
    /// Builds the page when the sheet opens and drops it when it closes.
    pub fn reconcile_whats_new(&mut self, window: &Window) {
        if !self.model.whats_new_open {
            self.whats_new_body = None;
            return;
        }
        if self.whats_new_body.is_some() {
            return;
        }
        let metrics = self.metrics(window);
        let theme_hex = |k: &str| {
            self.chrome
                .theme
                .as_ref()
                .and_then(|t| t.get(k))
                .and_then(|s| crate::chrome::hex(s))
        };
        let colors = PageColors {
            background: self.chrome.overlay_bg,
            text: self.chrome.text,
            faint: self.chrome.text_faint,
            heading: theme_hex("blue").unwrap_or(self.chrome.text_bright),
            code: theme_hex("green").unwrap_or(self.chrome.text_mid),
            link: theme_hex("cyan").unwrap_or(self.chrome.focus_ring),
        };
        let path = infiniterm_core::changelog::whats_new_path()
            .to_string_lossy()
            .into_owned();
        self.whats_new_body = Some(PageBody::new(
            path,
            &metrics,
            colors,
            Size { w: 800., h: 600. },
            // The app's own file, written at launch: always this disk.
            std::sync::Arc::new(infiniterm_core::file_source::LocalSource),
        ));
    }

    /// Every key while the sheet is up. Always taken: nothing reaches the
    /// canvas or a shell from here.
    pub fn whats_new_key(&mut self, k: &Keystroke, cx: &mut App) -> bool {
        let m = &k.modifiers;
        let plain = !m.platform && !m.control && !m.alt;
        if plain && k.key == "escape" {
            self.model.close_whats_new();
            self.redraw = true;
            return true;
        }
        let scrolled = plain
            && matches!(
                k.key.as_str(),
                "up" | "down" | "pageup" | "pagedown" | "home" | "end"
            )
            && self.whats_new_body.as_mut().is_some_and(|b| {
                !matches!(
                    crate::body::CardBody::key(b, k, crate::now_ms(), cx),
                    crate::body::BodyAction::Ignored
                )
            });
        if !scrolled {
            self.model.flash_whats_new();
        }
        self.redraw = true;
        true
    }

    pub fn render_whats_new(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let chrome = &self.chrome;
        let ui = self.model.ui_scale as f32;
        let flashing = self.model.whats_new_flashing(crate::now_ms());
        let w =
            (self.model.view_size.w as f32 - SHEET_MARGIN_PX).clamp(SHEET_MIN_W_PX, SHEET_MAX_W_PX);
        let h =
            (self.model.view_size.h as f32 - SHEET_MARGIN_PX).clamp(SHEET_MIN_H_PX, SHEET_MAX_H_PX);
        let entity = cx.entity();
        let title = format!("What's new in infiniterm {}", env!("CARGO_PKG_VERSION"));
        let border = if flashing {
            chrome.focus_ring
        } else {
            chrome.overlay_border
        };
        div()
            .id("whats-new")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(chrome.overlay_backdrop)
            // A click outside the sheet does not close it (Escape and the X
            // do); it flashes, like a key.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.model.flash_whats_new();
                    this.redraw = true;
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, cx| {
                let d = e.delta.pixel_delta(px(LINE_PX));
                if let Some(b) = this.whats_new_body.as_mut() {
                    crate::body::CardBody::wheel(
                        b,
                        infiniterm_core::grid::Point { x: 0., y: 0. },
                        f32::from(d.x) as f64,
                        f32::from(d.y) as f64,
                        &e.modifiers,
                    );
                }
                this.redraw = true;
                cx.notify();
            }))
            .child(
                div()
                    .id("whats-new-sheet")
                    .w(px(w))
                    .h(px(h))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded_lg()
                    .bg(chrome.overlay_bg)
                    // Two pixels always, so the flash changes the colour only
                    // and the text does not shift.
                    .border_2()
                    .border_color(border)
                    // A click on the sheet stays on the sheet.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .py_2()
                            .border_b_1()
                            .border_color(chrome.overlay_border)
                            .child(
                                div()
                                    .font_weight(chrome.typography.bold.weight)
                                    .text_size(px(chrome.typography.size(15.) * ui))
                                    .text_color(chrome.text_bright)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .id("whats-new-close")
                                    .cursor_pointer()
                                    .px_2()
                                    .rounded_sm()
                                    .text_size(px(chrome.typography.size(18.) * ui))
                                    .text_color(chrome.text_muted)
                                    .hover(|s| s.bg(crate::chrome::hover_fill(chrome.row_selected)))
                                    .child("\u{00d7}")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                            this.model.close_whats_new();
                                            this.redraw = true;
                                            cx.stop_propagation();
                                            cx.notify();
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        div().flex_1().child(
                            canvas(
                                |_, _, _| (),
                                move |bounds, _, window: &mut Window, cx| {
                                    entity.update(cx, |this, cx| {
                                        if let Some(b) = this.whats_new_body.as_mut() {
                                            crate::body::CardBody::paint(
                                                b,
                                                bounds,
                                                1.,
                                                true,
                                                crate::now_ms(),
                                                window,
                                                cx,
                                            );
                                        }
                                    });
                                },
                            )
                            .size_full(),
                        ),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_1()
                            .border_t_1()
                            .border_color(chrome.overlay_border)
                            .text_size(px(chrome.typography.size(11.) * ui))
                            .text_color(chrome.text_faint)
                            .child("Esc or \u{00d7} closes. Help: show what's new opens it again."),
                    ),
            )
    }
}
