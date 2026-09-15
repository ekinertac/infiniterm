//! What a card draws inside its frame. The trait the mapping asks for: the
//! model knows a card as a rect, an id and a kind, and the body is the one
//! thing that knows how to paint and take input. The terminal grid
//! (`terminal_body.rs`), the editor, the CEF surface and, later, a Wayland
//! surface all implement it. `Blank` stands in for the kinds not built yet.
use gpui::{fill, App, Bounds, Hsla, Keystroke, Pixels, Window};
use infiniterm_core::grid::{Point, Size};

/// What a body hands back from a click: something for the model to do.
#[derive(Clone, Debug, PartialEq)]
pub enum BodyAction {
    None,
    /// Cmd+click on a URL or a confirmed path: open it beside this card.
    Open(infiniterm_core::ift::OpenPlan),
    /// Cmd+Shift+click: the system's handler.
    OpenExternal {
        url: Option<String>,
        path: Option<(String, String)>,
    },
}

pub trait CardBody {
    /// `bounds` is the card's screen rect at `scale` (world units times zoom).
    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        scale: f64,
        focused: bool,
        window: &mut Window,
        cx: &mut App,
    );
    /// The card's rect in world units changed (a drag, a resize, a restore).
    fn resized(&mut self, _world: Size) {}
    /// A key the app did not claim (no Cmd chord bound to it).
    fn key(&mut self, _keystroke: &Keystroke, _cx: &mut App) {}
    /// A click in the body, in card pixels (the zoom undone).
    fn mouse_down(
        &mut self,
        _local: Point,
        _button: gpui::MouseButton,
        _modifiers: &gpui::Modifiers,
        _click_count: usize,
    ) -> BodyAction {
        BodyAction::None
    }
    fn mouse_up(
        &mut self,
        _local: Point,
        _button: gpui::MouseButton,
        _modifiers: &gpui::Modifiers,
    ) {
    }
    fn mouse_move(&mut self, _local: Point, _modifiers: &gpui::Modifiers) {}
    /// A bare scroll over the body: the terminal's scrollback, the page's scroll.
    fn wheel(&mut self, _local: Point, _dx: f64, _dy: f64, _modifiers: &gpui::Modifiers) {}
    /// The body no longer needs to paint every frame (nothing arrived).
    fn wants_frame(&self) -> bool {
        false
    }
    /// For the ui to reach a concrete body (the terminal, to feed it).
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

/// A coloured rectangle standing in for a body that is not built yet.
pub struct Blank {
    pub color: Hsla,
}

impl CardBody for Blank {
    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        _scale: f64,
        _focused: bool,
        window: &mut Window,
        _cx: &mut App,
    ) {
        window.paint_quad(fill(bounds, self.color));
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
