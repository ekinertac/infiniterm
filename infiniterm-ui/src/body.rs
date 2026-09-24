//! What a card draws inside its frame. The trait the mapping asks for: the
//! model knows a card as a rect, an id and a kind, and the body is the one
//! thing that knows how to paint and take input. The terminal grid
//! (`terminal_body.rs`), the editor, the CEF surface and, later, a Wayland
//! surface all implement it. `Blank` stands in for the kinds not built yet.
use gpui::{fill, App, Bounds, Hsla, Keystroke, Pixels, Window};
use infiniterm_core::grid::{Point, Size};

/// A click on a browser card's tab strip: which of its three affordances
/// (switch, close, open) it landed on, and which tab index. Carried out
/// through the model (`Model::browser_tab_*`, `infiniterm-core/src/model/
/// tabs_cmd.rs`), the same commands a keyboard shortcut already calls,
/// rather than the body mutating its own `tabs`/`active` mirror directly:
/// `Card.tabs` is authoritative and `browsers.rs::reconcile_browsers`
/// already owns the card-to-body sync, so a second, opposite-direction path
/// would fight it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabClick {
    /// Switch to the tab at this index.
    Switch(usize),
    /// Close the tab at this index (closes the card if it was the last tab).
    Close(usize),
    /// Open a new tab at `about:blank`.
    New,
}

/// What a body hands back from a click: something for the model to do.
#[derive(Clone, Debug, PartialEq)]
pub enum BodyAction {
    /// The key was taken; nothing else to do.
    None,
    /// The key was NOT taken: nothing in this body wanted it. The app then
    /// lets macOS's input context have it, which is how a dead key (Option+E
    /// on its own, a bare composition prefix) starts composing instead of
    /// vanishing. A body that says `None` for everything never composes.
    Ignored,
    /// Cmd+click on a URL or a confirmed path: open it beside this card.
    Open(infiniterm_core::ift::OpenPlan),
    /// Cmd+Shift+click: the system's handler.
    OpenExternal {
        url: Option<String>,
        path: Option<(String, String)>,
    },
    /// A click on a spawn error: start the shell again.
    Retry,
    /// A click on a browser card's tab strip.
    BrowserTab(TabClick),
}

pub trait CardBody {
    /// `bounds` is the card's screen rect at `scale` (world units times zoom).
    /// `now` is the frame's clock in ms, for anything that blinks.
    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        scale: f64,
        focused: bool,
        now: f64,
        window: &mut Window,
        cx: &mut App,
    );
    /// The card's rect in world units changed (a drag, a resize, a restore).
    fn resized(&mut self, _world: Size) {}
    /// A key the app did not claim (no Cmd chord bound to it). May ask for
    /// something, the way a click may (a file opened beside from the tree).
    /// Text that arrived without a key: the emoji panel, a finished dead-key
    /// composition, an input method's commit. Delivered as if typed.
    fn insert_text(&mut self, _text: &str) {}

    /// Where the caret is on screen, for the input method's candidate window
    /// to sit under. `None` puts it at the window's corner.
    fn caret_bounds(&self) -> Option<Bounds<Pixels>> {
        None
    }

    /// Cells of text this body would paint glyphs for, for the frame's
    /// glyph budget (`chrome::GLYPH_BUDGET_CELLS`). The grid's size, not
    /// what is in it, so the budget does not flip as output scrolls.
    fn text_cells(&self) -> usize {
        0
    }

    /// The frame is over its glyph budget: paint bars rather than glyphs.
    fn set_crowded(&mut self, _crowded: bool) {}

    fn key(&mut self, _keystroke: &Keystroke, _now: f64, _cx: &mut App) -> BodyAction {
        BodyAction::None
    }
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
    /// The pointer moved off this body onto another card or empty canvas.
    fn mouse_leave(&mut self) {}
    /// A bare scroll over the body: the terminal's scrollback, the page's scroll.
    fn wheel(&mut self, _local: Point, _dx: f64, _dy: f64, _modifiers: &gpui::Modifiers) {}
    /// Whether the next frame would paint differently: output arrived, or a
    /// blink is due at `now`. Frames are painted only on demand.
    fn wants_frame(&self, _now: f64) -> bool {
        false
    }
    /// A press started something that follows the pointer (a text
    /// selection, a drag a program is watching): moves and the release go
    /// to this body even after the pointer leaves the card.
    fn captures_drag(&self) -> bool {
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
        _now: f64,
        window: &mut Window,
        _cx: &mut App,
    ) {
        window.paint_quad(fill(bounds, self.color));
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
