//! What a browser card's tab needs from the thing that shows its page, so the
//! card (`browser_body.rs`) and the model glue (`browsers.rs`) do not care
//! whether the page is a CEF browser painted into a texture or the system's
//! own web view (WKWebView, `webview.rs`) floating above the canvas. #761.
//!
//! The seam is narrow on purpose. `infiniterm_browser::Surface` implements
//! the trait by forwarding every call to its own method, so the CEF path does
//! exactly what it did; the methods only the texture engine needs (frames,
//! mouse, keys, kick) have do-nothing defaults the web view takes as they are,
//! because its native view gets those events from AppKit itself. The one
//! method only the web view fills is `place`: where, how zoomed, and in what
//! state its native view is this frame.
//!
//! Where it fits: `browser_body.rs` holds a `Box<dyn PageSurface>` per tab;
//! `Engine` says which kind a card opens (`browser.engine`);
//! `infiniterm_core::webview_policy` holds the rules `place` is handed the
//! results of. Related: `webview.rs`, `browsers.rs`.
//! Constraint: everything here runs on the UI thread, so no `Send`.
use gpui::Window;
use infiniterm_browser::{Button, ContextMenuRequest, Frame, Mods, Surface};
use std::sync::Arc;

/// Which engine a card's tabs open with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Cef,
    /// The system web view. The identifier names its data store
    /// (`webview_policy::store_identifier`).
    Webview {
        store: [u8; 16],
        /// `browser.devtools`: make the page inspectable from Safari's
        /// Develop menu.
        devtools: bool,
    },
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
    /// The device scale last given to `resize`.
    fn device_scale(&self) -> f32;
    fn resize(&self, width: i32, height: i32, scale: f32);
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

    /// CEF's page id on its debugging port; the web view has none.
    fn target_id(&self) -> Option<String> {
        None
    }
    /// CEF asked to paint again (`frame_fit::needs_kick`).
    fn kick(&self) {}
    fn has_new_frame(&self) -> bool {
        false
    }
    /// The page needs a paint at `now_ms` for its own reasons (the web view's
    /// snapshot schedule); `has_new_frame` is for what already happened.
    fn wants_frame(&self, _now_ms: f64) -> bool {
        false
    }
    /// New pixels: a CEF frame, or the web view's newest snapshot.
    fn take_frame(&self) -> Option<Arc<Frame>> {
        None
    }
    fn cursor(&self) -> &'static str {
        "default"
    }
    fn mouse_move(&self, _x: f32, _y: f32, _mods: Mods, _held: Option<Button>) {}
    fn mouse_leave(&self) {}
    fn mouse_button(&self, _x: f32, _y: f32, _mods: Mods, _b: Button, _up: bool, _clicks: usize) {}
    fn wheel(&self, _x: f32, _y: f32, _mods: Mods, _dx: f32, _dy: f32) {}
    fn key(&self, _name: &str, _text: Option<&str>, _mods: Mods) {}

    /// Whether this page is a native view the card has to place each frame.
    fn is_native(&self) -> bool {
        false
    }
    /// Files the page downloaded since last asked, as a line each for the
    /// card to show ("saved report.pdf in Downloads").
    fn take_downloads(&self) -> Vec<String> {
        vec![]
    }
    /// A click reached the page of a card that had the focus but not the
    /// lock (so the card locks, as a click that reaches a CEF page does).
    fn take_page_click(&self) -> bool {
        false
    }
    /// Put the native view where the card is this frame.
    fn place(&self, _window: &Window, _p: &Placement) {}
    /// The card is not on screen this frame (hidden tab, off-window).
    fn hide(&self) {}
}

impl PageSurface for Surface {
    fn navigate(&self, url: &str) {
        Surface::navigate(self, url)
    }
    fn back(&self) {
        Surface::back(self)
    }
    fn forward(&self) {
        Surface::forward(self)
    }
    fn can_go_back(&self) -> bool {
        Surface::can_go_back(self)
    }
    fn can_go_forward(&self) -> bool {
        Surface::can_go_forward(self)
    }
    fn reload(&self) {
        Surface::reload(self)
    }
    fn find(&self, text: &str, forward: bool, next: bool) {
        Surface::find(self, text, forward, next)
    }
    fn stop_find(&self) {
        Surface::stop_find(self)
    }
    fn find_result(&self) -> (i32, i32) {
        Surface::find_result(self)
    }
    fn device_scale(&self) -> f32 {
        self.shared.borrow().scale
    }
    fn resize(&self, width: i32, height: i32, scale: f32) {
        Surface::resize(self, width, height, scale)
    }
    fn take_popups(&self) -> Vec<String> {
        Surface::take_popups(self)
    }
    fn take_close_request(&self) -> bool {
        Surface::take_close_request(self)
    }
    fn take_external_links(&self) -> Vec<String> {
        Surface::take_external_links(self)
    }
    fn take_context_menu(&self) -> Option<ContextMenuRequest> {
        Surface::take_context_menu(self)
    }
    fn title(&self) -> Option<String> {
        Surface::title(self)
    }
    fn url(&self) -> Option<String> {
        Surface::url(self)
    }
    fn close(&self) {
        Surface::close(self)
    }
    fn focus(&self, on: bool) {
        Surface::focus(self, on)
    }
    fn edit_chord(&self, key: &str, shift: bool) -> bool {
        Surface::edit_chord(self, key, shift)
    }
    fn set_zoom(&self, factor: f64) {
        Surface::set_zoom(self, factor)
    }
    fn target_id(&self) -> Option<String> {
        Surface::target_id(self)
    }
    fn kick(&self) {
        Surface::kick(self)
    }
    fn has_new_frame(&self) -> bool {
        Surface::has_new_frame(self)
    }
    fn take_frame(&self) -> Option<Arc<Frame>> {
        Surface::take_frame(self)
    }
    fn cursor(&self) -> &'static str {
        Surface::cursor(self)
    }
    fn mouse_move(&self, x: f32, y: f32, mods: Mods, held: Option<Button>) {
        Surface::mouse_move(self, x, y, mods, held)
    }
    fn mouse_leave(&self) {
        Surface::mouse_leave(self)
    }
    fn mouse_button(&self, x: f32, y: f32, mods: Mods, b: Button, up: bool, clicks: usize) {
        Surface::mouse_button(self, x, y, mods, b, up, clicks)
    }
    fn wheel(&self, x: f32, y: f32, mods: Mods, dx: f32, dy: f32) {
        Surface::wheel(self, x, y, mods, dx, dy)
    }
    fn key(&self, name: &str, text: Option<&str>, mods: Mods) {
        Surface::key(self, name, text, mods)
    }
}

/// Start of a frame: every native page is marked not placed.
pub fn begin_frame() {
    #[cfg(target_os = "macos")]
    crate::webview::begin_frame();
}

/// End of a frame: a native page no card placed (its card is off the window,
/// on another workspace, or gone) is hidden, because a native view stays
/// where it was left.
pub fn end_frame() {
    #[cfg(target_os = "macos")]
    crate::webview::end_frame();
}
