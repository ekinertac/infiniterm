//! The window frame the app draws itself on Linux when gpui gives it
//! client-side decorations (#464, Ekin: GNOME style): a row of buttons at
//! the right end of the title bar (minimise, maximise or restore, close) and
//! resize edges along the window's border.
//!
//! Where it fits: `main.rs` asks gpui for client-side decorations on Linux;
//! gpui answers with `Decorations::Client` only where a compositor can draw
//! them (GNOME Wayland, an X11 desktop with a compositor) and falls back to
//! the system frame elsewhere, where none of this is drawn (the title bar
//! then sits under the window manager's own). `overlays.rs` calls
//! `buttons` from the title bar and `edges` from the root element.
//!
//! The decisions are the two pure functions (`visible_buttons`,
//! `active_edges`), tested; the rest is gpui wiring. Not done: a shadow and
//! rounded corners outside the window, which need a transparent window
//! background, and the window opacity setting owns that today.
//!
//! Related: `not_macos.rs::title_drag` (dragging and double-clicking the
//! bar), docs/agent/linux-port.md ("The title bar on Linux").
use crate::AppView;
use gpui::{
    div, prelude::*, px, AnyElement, CursorStyle, Decorations, Hsla, MouseButton, MouseDownEvent,
    ResizeEdge, Tiling, Window, WindowControls,
};

/// A title bar button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Minimise,
    Maximise,
    Restore,
    Close,
}

/// The buttons in GNOME's order, left to right, close last at the window's
/// edge: what the platform supports (`window_controls`, which Wayland
/// fills in from the compositor), maximise drawn as restore while the
/// window is maximised. Close is always there.
pub fn visible_buttons(controls: WindowControls, maximised: bool) -> Vec<Button> {
    let mut out = vec![];
    if controls.minimize {
        out.push(Button::Minimise);
    }
    if controls.maximize {
        out.push(if maximised {
            Button::Restore
        } else {
            Button::Maximise
        });
    }
    out.push(Button::Close);
    out
}

/// The edges and corners that resize the window: every one whose side is
/// not tiled (a maximised window is tiled on all four, so it has none; a
/// window snapped to the left half keeps its right edge).
pub fn active_edges(tiling: Tiling) -> Vec<ResizeEdge> {
    let (t, b, l, r) = (!tiling.top, !tiling.bottom, !tiling.left, !tiling.right);
    let mut out = vec![];
    for (edge, on) in [
        (ResizeEdge::Top, t),
        (ResizeEdge::Bottom, b),
        (ResizeEdge::Left, l),
        (ResizeEdge::Right, r),
        (ResizeEdge::TopLeft, t && l),
        (ResizeEdge::TopRight, t && r),
        (ResizeEdge::BottomLeft, b && l),
        (ResizeEdge::BottomRight, b && r),
    ] {
        if on {
            out.push(edge);
        }
    }
    out
}

/// How deep the resize band is inside the window's border, in window pixels.
/// Inside, not outside: there is no transparent margin (see the header).
const EDGE_PX: f32 = 5.;
/// A button's circle and the gap between buttons, before `ui.scale`.
const BUTTON_PX: f32 = 22.;
const BUTTON_GAP_PX: f32 = 6.;
/// The circle's fill at rest and under the pointer, as a share of the text
/// colour: libadwaita's flat round title bar buttons.
const BUTTON_FILL: f32 = 0.10;
const BUTTON_HOVER_FILL: f32 = 0.20;

fn cursor_for(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// The glyph a button draws: plain characters every font has, so nothing
/// depends on an icon font.
fn glyph(b: Button) -> &'static str {
    match b {
        Button::Minimise => "\u{2013}",
        Button::Maximise => "\u{25a1}",
        Button::Restore => "\u{2750}",
        Button::Close => "\u{00d7}",
    }
}

/// Whether gpui is letting the app draw the frame, and the tiling if so.
pub fn client_tiling(window: &Window) -> Option<Tiling> {
    match window.window_decorations() {
        Decorations::Client { tiling } => Some(tiling),
        Decorations::Server => None,
    }
}

impl AppView {
    /// The button row for the right end of the title bar.
    pub fn window_buttons(&self, window: &Window, text: Hsla) -> AnyElement {
        let ui = self.model.ui_scale as f32;
        let mut row = div()
            .flex()
            .items_center()
            .gap(px(BUTTON_GAP_PX * ui))
            .ml_2();
        for b in visible_buttons(window.window_controls(), window.is_maximized()) {
            row = row.child(
                div()
                    .id(("window-button", b as usize))
                    .size(px(BUTTON_PX * ui))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(text)
                    .bg(crate::chrome::with_alpha(text, BUTTON_FILL))
                    .hover(move |s| s.bg(crate::chrome::with_alpha(text, BUTTON_HOVER_FILL)))
                    .cursor_pointer()
                    // The press is the button's, not the bar's: no window
                    // move, no double-click maximise behind it.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| match b {
                        Button::Minimise => window.minimize_window(),
                        Button::Maximise | Button::Restore => window.zoom_window(),
                        // The same path as the app's Quit: `on_app_quit`
                        // writes the canvas and the window frame.
                        Button::Close => cx.quit(),
                    })
                    .child(glyph(b)),
            );
        }
        row.into_any_element()
    }

    /// The resize bands, absolute over the window's border, for the root
    /// element. Empty when the window is tiled on every side.
    pub fn window_edges(&self, tiling: Tiling) -> Vec<AnyElement> {
        let e = px(EDGE_PX);
        active_edges(tiling)
            .into_iter()
            .map(|edge| {
                let band = div().absolute().occlude();
                let band = match edge {
                    ResizeEdge::Top => band.top_0().left(e).right(e).h(e),
                    ResizeEdge::Bottom => band.bottom_0().left(e).right(e).h(e),
                    ResizeEdge::Left => band.left_0().top(e).bottom(e).w(e),
                    ResizeEdge::Right => band.right_0().top(e).bottom(e).w(e),
                    ResizeEdge::TopLeft => band.top_0().left_0().size(e),
                    ResizeEdge::TopRight => band.top_0().right_0().size(e),
                    ResizeEdge::BottomLeft => band.bottom_0().left_0().size(e),
                    ResizeEdge::BottomRight => band.bottom_0().right_0().size(e),
                };
                band.cursor(cursor_for(edge))
                    .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        window.start_window_resize(edge);
                    })
                    .into_any_element()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controls(minimize: bool, maximize: bool) -> WindowControls {
        WindowControls {
            fullscreen: true,
            maximize,
            minimize,
            window_menu: true,
        }
    }

    #[test]
    fn the_buttons_follow_gnome_with_close_last() {
        assert_eq!(
            visible_buttons(controls(true, true), false),
            [Button::Minimise, Button::Maximise, Button::Close]
        );
        assert_eq!(
            visible_buttons(controls(true, true), true),
            [Button::Minimise, Button::Restore, Button::Close]
        );
        // A compositor without minimise (GNOME's default) shows what it has.
        assert_eq!(
            visible_buttons(controls(false, true), false),
            [Button::Maximise, Button::Close]
        );
        assert_eq!(
            visible_buttons(controls(false, false), false),
            [Button::Close]
        );
    }

    #[test]
    fn a_tiled_side_does_not_resize() {
        let free = Tiling::default();
        assert_eq!(active_edges(free).len(), 8);
        assert!(
            active_edges(Tiling::tiled()).is_empty(),
            "maximised: no edges"
        );
        // Snapped to the left half: only the right edge stays.
        let left_half = Tiling {
            top: true,
            bottom: true,
            left: true,
            right: false,
        };
        assert_eq!(active_edges(left_half), [ResizeEdge::Right]);
    }
}
