//! What a browser card's tab needs from the thing that shows its page, and the
//! plain types that cross between the two: the card (`browser_body.rs`) and
//! the model glue (`browsers.rs`) talk to a `PageSurface`, and the only one is
//! the system's own web view (WKWebView, `webview.rs`) floating above the
//! canvas. #761, #826.
//!
//! Why a trait with one implementor: CEF used to be the other one and was
//! removed in #826; the trait stayed because the card and the glue are
//! written against it and it is where a WebView2 or WebKitGTK engine will
//! plug in on Windows and Linux. Collapse it into `WebSurface` if that does
//! not happen. The methods the web view does not need have do-nothing
//! defaults: its native view gets frames, mouse and keys from AppKit itself.
//!
//! Where it fits: `browser_body.rs` holds a `Box<dyn PageSurface>` per tab;
//! `Engine` carries what a new page needs (its data store);
//! `infiniterm_core::webview_policy` holds the rules `place` is handed the
//! results of. Related: `webview.rs`, `browsers.rs`.
//! Constraint: everything here runs on the UI thread, so no `Send`.
use gpui::Window;
use std::sync::Arc;

/// Whether `INFINITERM_KEYLOG` is set: the log lines of the browser card and
/// of the web view's input monitor.
pub fn keylog() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("INFINITERM_KEYLOG").is_some())
}

/// What a card's tabs are opened with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Engine {
    /// Names the web view's data store (`webview_policy::store_identifier`).
    pub store: [u8; 16],
    /// `browser.devtools`: make the page inspectable from Safari's Develop
    /// menu.
    pub devtools: bool,
}

/// One picture of a page: BGRA, `width * height * 4` bytes, the byte order
/// gpui keeps images in. The web view's snapshot (`webview.rs`) is the only
/// source.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    /// The layout size (CSS pixels) the page had when the picture was taken:
    /// not what the card wants now. The ui draws an old picture at its
    /// natural size while a resize goes on, instead of stretching it to the
    /// new card (#479, `webview_policy::frame_draw_size`).
    pub css_width: i32,
    pub css_height: i32,
}

/// A right-click, in page pixels, and enough of what it landed on for the
/// card's menu (`menus.rs` decides what to show).
#[derive(Clone, Debug)]
pub struct ContextMenuRequest {
    pub x: i32,
    pub y: i32,
    pub link_url: Option<String>,
    pub editable: bool,
    pub has_selection: bool,
    /// The page's history, so the menu offers Back and Forward only when
    /// there is somewhere to go.
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// The page's own address, for "open in the system browser".
    pub page_url: String,
}

/// Modifier keys of a click, as the page sees them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    /// Cmd: with it a clicked link opens in a new tab.
    pub command: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

/// Where and in what state the native view is for one frame. Window pixels,
/// top-left origin, the same space gpui paints in.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// The page's rect: the card under its tab strip.
    pub rect: [f64; 4],
    /// The page zoom to give WebKit (canvas zoom times the page's own).
    pub zoom: f64,
    /// The canvas zoom alone, to turn a window point into a world one.
    pub canvas_scale: f64,
    /// The layout size in world units, which a snapshot is tagged with.
    pub world: (i32, i32),
    /// Live or a picture, decided by `webview_policy::page_view`.
    pub snapshot: bool,
    /// Rects gpui draws over the page (cards in front, an overlay, the drag
    /// ghost): the mask holes AND the click routing are both made from this
    /// one list.
    pub covers: Vec<[f64; 4]>,
    /// The card has the focus, and the keyboard is locked to the page.
    pub focused: bool,
    pub locked: bool,
    /// Alpha of the wash over the page, `webview_policy::dim_alpha`.
    pub dim: f64,
    /// The wash's colour (the card background).
    pub dim_rgb: [f32; 3],
    /// The frame's clock, for the snapshot schedule.
    pub now_ms: f64,
}

pub trait PageSurface {
    fn navigate(&self, url: &str);
    fn back(&self);
    fn forward(&self);
    fn can_go_back(&self) -> bool;
    fn can_go_forward(&self) -> bool;
    fn reload(&self);
    fn find(&self, text: &str, forward: bool, next: bool);
    fn stop_find(&self);
    /// (matches, which one is current), zero matches for none.
    fn find_result(&self) -> (i32, i32);
    fn resize(&self, width: i32, height: i32);
    fn take_popups(&self) -> Vec<String>;
    fn take_close_request(&self) -> bool;
    fn take_external_links(&self) -> Vec<String>;
    fn take_context_menu(&self) -> Option<ContextMenuRequest>;
    fn title(&self) -> Option<String>;
    fn url(&self) -> Option<String>;
    fn close(&self);
    fn focus(&self, on: bool);
    fn edit_chord(&self, key: &str, shift: bool) -> bool;
    fn set_zoom(&self, factor: f64);

    fn has_new_frame(&self) -> bool {
        false
    }
    /// The page needs a paint at `now_ms` for its own reasons (the web view's
    /// snapshot schedule); `has_new_frame` is for what already happened.
    fn wants_frame(&self, _now_ms: f64) -> bool {
        false
    }
    /// The web view's newest snapshot, if a new one arrived.
    fn take_frame(&self) -> Option<Arc<Frame>> {
        None
    }
    /// A click on a page the card forwards (the card's mouse path calls it
    /// for every press and release over the page; the web view uses the
    /// release-free press for Cmd+click and middle-click).
    fn mouse_button(&self, _x: f32, _y: f32, _mods: Mods, _b: Button, _up: bool, _clicks: usize) {}

    /// Whether this page is a native view the card has to place each frame.
    fn is_native(&self) -> bool {
        false
    }
    /// Files the page downloaded since last asked, as a line each for the
    /// card to show ("saved report.pdf in Downloads").
    fn take_downloads(&self) -> Vec<String> {
        vec![]
    }
    /// Times (ms) of bare Escapes the page got while its card was locked, for
    /// the card's double-Escape (the native page takes the key before gpui).
    fn take_escapes(&self) -> Vec<f64> {
        vec![]
    }
    /// A click reached the page of a card that had the focus but not the
    /// lock (so the card locks).
    fn take_page_click(&self) -> bool {
        false
    }
    /// Put the native view where the card is this frame.
    fn place(&self, _window: &Window, _p: &Placement) {}
    /// The card is not on screen this frame (hidden tab, off-window).
    fn hide(&self) {}
}

/// Start of a frame: every native page is marked not placed.
pub fn begin_frame() {
    crate::webview::begin_frame();
}

/// End of a frame: a native page no card placed (its card is off the window,
/// on another workspace, or gone) is hidden, because a native view stays
/// where it was left.
pub fn end_frame() {
    crate::webview::end_frame();
}
