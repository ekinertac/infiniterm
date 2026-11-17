//! The browser card: one system web view per tab (`webview.rs`, behind the
//! `PageSurface` trait), floating over the card's page area, with a snapshot
//! drawn by gpui as the tab's texture whenever the page cannot be shown live
//! (far zoom, a card over it, a drag). Extended for tabs per
//! `docs/superpowers/specs/2026-09-19-browser-tabs-design.md`.
//!
//! Every tab lays out at the card's size in world units and is given the
//! zoom, so it scales with the canvas. Every tab's surface stays LIVE the way
//! Chrome's do: a background tab keeps loading, keeps its scroll position,
//! keeps its size, so switching to it shows the page rather than a blank.
//! Keys: the app owns Cmd except the edit chords a page needs (copy, paste,
//! cut, select all, undo) and the page-zoom chords the model rebinds
//! (`browser_keys.rs`); everything else goes to the page once it has focus
//! (`webview_input.rs`). Focus is deliberate: a click on an UNFOCUSED card
//! only focuses it (the scrim takes it), the next reaches the page;
//! arrow-focusing a card never gives the page focus. That same
//! `page_focused` IS the keyboard lock the tab chords need, so the
//! tab-management chords never reach a surface at all: they are
//! `Model::browser_tab_*` (`tabs_cmd.rs`), and what arrives here is the
//! changed `card.tabs` that `browsers.rs` reconciles against `self.tabs`.
//!
//! Where it fits: built by `browsers.rs` (`reconcile_browsers`), painted by
//! `paint.rs`, takes its mouse and keys from `input.rs`. This file is
//! compiled on macOS only; elsewhere `browser_stub.rs` stands in for it.
use crate::body::{BodyAction, CardBody};
use crate::page_surface::{Button, Engine, Mods, PageSurface, Placement};
use crate::tab_strip::{
    hover_lines, hovered, paint_hover_label, paint_strip, strip_hit, strip_world_h, StripStyle,
    TabDrag,
};
use gpui::{fill, point, px, size, App, Bounds, Hsla, Keystroke, Pixels, RenderImage, Window};
use image::{Frame as ImageFrame, RgbaImage};
use infiniterm_core::grid::{Point, Size};
use infiniterm_core::webview_policy as policy;
use smallvec::SmallVec;
use std::sync::Arc;

/// The placeholder text's inset from the card's corner.
const STATUS_TEXT_PAD_PX: f64 = 12.;
/// The placeholder text's line height, looser than its font size.
const STATUS_LINE_HEIGHT_RATIO: f32 = 1.5;

/// One tab: everything that was a single field on `BrowserBody` before
/// tabs existed, once per open page.
pub struct Tab {
    pub url: String,
    pub surface: Option<Box<dyn PageSurface>>,
    texture: Option<Arc<RenderImage>>,
    /// The layout size (CSS pixels) `texture` was taken at, so an old
    /// snapshot is drawn at its natural size while a resize goes on (#479).
    texture_css: (i32, i32),
    /// The page title as last reported, so a change can be told from the
    /// same title arriving every frame. Not the card's title: that is a
    /// name somebody CHOSE, and a page must never overwrite it.
    title: Option<String>,
    /// The card's page zoom as last pushed to THIS surface. Per tab
    /// because a tab opened after the zoom was set has not had it yet.
    applied_zoom: Option<f64>,
    /// A page opened this tab (`window.open`, `target=_blank`, #672): its own
    /// `window.close()` may close it, as in Chrome. A tab you opened yourself
    /// ignores `window.close()`.
    from_popup: bool,
}

impl Tab {
    fn open(url: &str, world: Size, engine: Engine) -> Tab {
        // The web view itself is made lazily, in `place`, when a window is
        // at hand.
        let surface = crate::webview::WebSurface::open(
            url,
            engine.store,
            (world.w.round() as i32, world.h.round() as i32),
            engine.devtools,
        );
        Tab {
            url: url.to_string(),
            surface: Some(Box::new(surface)),
            texture: None,
            texture_css: (0, 0),
            title: None,
            applied_zoom: None,
            from_popup: false,
        }
    }

    fn close(&mut self) {
        if let Some(s) = self.surface.take() {
            s.close();
        }
    }
}

/// A tab's strip label: the page title once it has one, else the url it was
/// opened on. Matches `paint`'s existing `format!("loading {}", ...)`
/// fallback in spirit: something to show before the page has said anything.
fn tab_label(tab: &Tab) -> &str {
    tab.title.as_deref().unwrap_or(&tab.url)
}

/// A closed tab's surface has to be told to go: `Surface` has no `Drop` of
/// its own, and a `Vec::remove` is the only thing that closes a tab.
impl Drop for Tab {
    fn drop(&mut self) {
        self.close();
    }
}

pub struct BrowserBody {
    pub card_id: String,
    pub tabs: Vec<Tab>,
    pub active: usize,
    /// Mirrors `card.locked`, written by `browsers.rs` every frame, so the
    /// tab strip can draw the lock state without a `Card` to read.
    pub locked: bool,
    /// Which tab was last told it has the keyboard focus, so the handover
    /// on a tab switch takes it off exactly one surface and a fresh tab,
    /// which has never had it, takes typing.
    focused_tab: Option<usize>,
    /// The page's own zoom the card carries (`card.zoom`), applied to every
    /// tab: a background tab must already be at the card's zoom when it
    /// becomes the visible one.
    pub zoom: Option<f64>,
    /// The page zoom of the last frame and when it last changed, for the
    /// picture shown while the canvas zooms (`webview_policy::zoom_settling`).
    last_page_zoom: Option<f64>,
    zoom_changed_at: Option<f64>,
    world: Size,
    painted_focused: bool,
    /// The page has keyboard focus: a click reached it since the card
    /// was focused. This is also what the keyboard lock IS (see the
    /// design doc); `browsers.rs` mirrors it onto `card.locked`.
    pub page_focused: bool,
    /// The last Escape's timestamp while locked, for double-Escape
    /// detection; `None` after any other key, after unlocking, or before
    /// the first Escape.
    pub last_escape_ms: Option<f64>,
    left_down: bool,
    /// A tab held by the pointer, from the press to the release (#682).
    tab_drag: Option<TabDrag>,
    /// The tab under the pointer, whose full title and address the hover
    /// label shows (#742).
    hover_tab: Option<usize>,
    /// A drop since `browsers.rs` last looked: (from, to) for `Model::move_tab`.
    pub tab_drop: Option<(usize, usize)>,
    pub inactive_dim: f64,
    /// The card's number, mirrored from `card.number` by `reconcile_browsers`
    /// so the strip can paint it: the corner label used to carry this, and
    /// was removed when the strip took over the job.
    pub card_number: u32,
    /// Mirrors `card.protected`: `card.protect`'s own lock, a different
    /// one from `locked` above (the keyboard's), shown beside `#N`.
    pub protected: bool,
    /// Mirrors `Model::ui_scale`, the same way `card_bg`/`text` do, so the
    /// strip's screen-pixel sizes scale with the interface multiplier like
    /// every other piece of chrome.
    pub ui_scale: f32,
    pub card_bg: Hsla,
    pub text: Hsla,
    /// The strip's colours and font, `tab_strip.rs`'s shared shape: the
    /// editor's tab strip is the same struct, so the two look and size
    /// alike under the UI typography settings.
    pub style: StripStyle,
    dirty: bool,
    pub popups: Vec<String>,
    /// Links to another app (`slack://...`, #668) any tab's page asked for
    /// since the last drain: `browsers.rs` queues each for its question.
    pub external_links: Vec<String>,
    /// Lines about files a page downloaded since `browsers.rs` last looked
    /// (the webview engine saves them to Downloads), for it to show.
    pub downloads: Vec<String>,
    /// The addresses `browsers.rs` is about to open as tabs because a page
    /// opened them (#672): the tab made for one of these is `from_popup`.
    pub popup_urls: Vec<String>,
    /// Tabs whose own page called `window.close()` and may go (#672), by
    /// index; `browsers.rs` closes them through the model, which owns the
    /// tab list.
    pub page_closed: Vec<usize>,
    /// A right-click since the last drain, for `browsers.rs` to turn into
    /// the app-level menu overlay. Aggregated across tabs rather than kept
    /// per tab: a menu only ever opens from a click on the visible page.
    pub context_menu: Option<crate::page_surface::ContextMenuRequest>,
    /// The texture last painted into gpui's sprite atlas, so `paint` can
    /// evict it the moment a new one takes its place. gpui has no atlas
    /// LRU: every `RenderImage::new` mints a fresh id, and nothing but
    /// `Window::drop_image` ever frees the tile it was painted into, so
    /// without this an open browser card grows the atlas by one tile per
    /// snapshot for as long as it runs.
    painted_texture: Option<Arc<RenderImage>>,
    /// What a tab opened later is built with, the same as the first one.
    engine: Engine,
    /// Window-pixel rects painted over this card this frame (cards in front,
    /// an overlay, the drag ghost), set by `paint.rs` before `paint`
    /// (`CardBody::set_covers`). Only a native page cares.
    covers: Vec<[f64; 4]>,
}

impl BrowserBody {
    /// The tab under a point of the card (a right-click's), `None` off the
    /// tabs, on the "+" and below the strip.
    pub fn tab_at(&self, local: Point) -> Option<usize> {
        // A browser card has no folder header ahead of its tabs (0 wide).
        hovered(strip_hit(
            local,
            self.style.font_px,
            self.ui_scale,
            self.tabs.len(),
            0.,
            self.style.pad_x(),
        ))
    }

    /// Notes the tab under the pointer, asking for a frame when it changed.
    fn set_hover(&mut self, tab: Option<usize>) {
        if self.hover_tab != tab {
            self.hover_tab = tab;
            self.dirty = true;
        }
    }

    pub fn new(
        card_id: &str,
        url: &str,
        world: Size,
        style: StripStyle,
        engine: Engine,
    ) -> BrowserBody {
        BrowserBody {
            card_id: card_id.to_string(),
            // ui_scale defaults to 1. below, same as every other mirrored
            // field, until `reconcile_browsers` pushes the real one.
            tabs: vec![Tab::open(
                url,
                page_world_size(world, style.font_px, 1.),
                engine,
            )],
            active: 0,
            locked: false,
            focused_tab: None,
            zoom: None,
            last_page_zoom: None,
            zoom_changed_at: None,
            world,
            painted_focused: false,
            page_focused: false,
            last_escape_ms: None,
            left_down: false,
            tab_drag: None,
            hover_tab: None,
            tab_drop: None,
            inactive_dim: crate::chrome::INACTIVE_DIM_DEFAULT,
            card_number: 0,
            protected: false,
            ui_scale: 1.,
            card_bg: gpui::rgb(0x0e101a).into(),
            text: gpui::rgb(0xb9c4d2).into(),
            style,
            dirty: true,
            popups: vec![],
            external_links: vec![],
            downloads: vec![],
            popup_urls: vec![],
            page_closed: vec![],
            context_menu: None,
            painted_texture: None,
            engine,
            covers: vec![],
        }
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    /// The surface every key, click and command goes to: the visible tab's.
    pub fn active_surface(&self) -> Option<&dyn PageSurface> {
        self.active_tab().and_then(|t| t.surface.as_deref())
    }

    /// The active tab's url, for `browsers.rs` to compare against
    /// `card.tabs[card.active_tab]`.
    pub fn active_url(&self) -> &str {
        self.active_tab().map_or("", |t| t.url.as_str())
    }

    /// Opens a tab at `url`, at the end, the way `Model::browser_tab_open`
    /// appends one. It does NOT become active on its own: the active index
    /// arrives from `card.active_tab` like everything else.
    pub fn open_tab(&mut self, url: &str) {
        let page = page_world_size(self.world, self.style.font_px, self.ui_scale);
        let mut tab = Tab::open(url, page, self.engine);
        // Made for an address a page opened: that page may close it again.
        if let Some(at) = self.popup_urls.iter().position(|u| u == url) {
            self.popup_urls.remove(at);
            tab.from_popup = true;
        }
        self.tabs.push(tab);
        self.dirty = true;
    }

    /// Throws every tab away and opens `urls` instead, the state a body is
    /// built in. The escape hatch for a card whose list moved by more than
    /// one step between frames, where no diff can say WHICH tabs moved: it
    /// costs every tab's live state (scroll, form input, whatever its JS
    /// holds), and it is the only thing that always converges. `browsers.rs`
    /// reaches for it last.
    pub fn rebuild_tabs(&mut self, urls: &[String]) {
        // Cleared first so the old surfaces are gone before the new ones
        // are asked for, rather than twice the browsers alive at once.
        self.tabs.clear();
        self.focused_tab = None;
        let page = page_world_size(self.world, self.style.font_px, self.ui_scale);
        let engine = self.engine;
        self.tabs = urls.iter().map(|u| Tab::open(u, page, engine)).collect();
        self.active = 0;
        self.apply_focus();
        self.dirty = true;
    }

    /// Closes the tab at `index`, which closes its surface with it. Out of
    /// range is a no-op rather than a panic: the index comes from a diff
    /// against `card.tabs`, and a painter must not be able to kill the app.
    pub fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        // Both remembered indices move when a tab before them goes; the
        // focused one has to follow or the handover would take the keyboard
        // off a tab that still has it. Closing the active tab leaves the
        // index where it was, so the next tab along takes the slot, which is
        // what Chrome does.
        if index < self.active {
            self.active -= 1;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
        self.focused_tab = match self.focused_tab {
            Some(f) if f == index => None,
            Some(f) if f > index => Some(f - 1),
            other => other,
        };
        self.apply_focus();
        self.dirty = true;
    }

    /// Puts tab `from` at `to`, as `Model::move_tab` did to `card.tabs`
    /// (#682): each surface goes with its tab, so no page reloads, and the
    /// remembered indices follow their tabs. Out of range is a no-op.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() || to >= self.tabs.len() || from == to {
            return;
        }
        use infiniterm_core::model::tabs_cmd::index_after_move;
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = index_after_move(self.active, from, to);
        self.focused_tab = self.focused_tab.map(|f| index_after_move(f, from, to));
        self.dirty = true;
    }

    /// Switches to the tab at `index`. Out of range is a no-op: a card whose
    /// tabs the body has not caught up with yet asks for one for a frame.
    pub fn set_active(&mut self, index: usize) {
        if index == self.active || index >= self.tabs.len() {
            return;
        }
        self.active = index;
        self.apply_focus();
        self.dirty = true;
    }

    /// Exactly the active tab's surface holds the keyboard focus, and only
    /// while the card's page is focused. Enforced in one place because three
    /// things move it: a click, a tab switch, and a closed tab shifting the
    /// indices under both.
    fn apply_focus(&mut self) {
        let want = self.page_focused.then_some(self.active);
        if want == self.focused_tab {
            return;
        }
        if let Some(s) = self
            .focused_tab
            .and_then(|i| self.tabs.get(i))
            .and_then(|t| t.surface.as_ref())
        {
            s.focus(false);
        }
        if let Some(s) = want
            .and_then(|i| self.tabs.get(i))
            .and_then(|t| t.surface.as_ref())
        {
            s.focus(true);
        }
        self.focused_tab = want;
    }

    /// Each frame, for EVERY tab: new pixels, popups, the address it moved
    /// to. A background tab still runs and can still navigate itself, so its
    /// url and title have to stay in step while it is not the one painting.
    ///
    /// Returns the url when the ACTIVE tab moved, the one case `browsers.rs`
    /// needs to hear about (history, `card.url`).
    pub fn sync(&mut self) -> Option<String> {
        let mut active_moved = None;
        let mut leave = false;
        let zoom = self.zoom;
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            let Some(surface) = &tab.surface else {
                continue;
            };
            let is_active = i == self.active;
            // `window.close()` (#672): taken from every tab so the request
            // cannot pile up, honoured only for a tab a page opened.
            // A click reached a native page whose card had the focus but not
            // the lock: the card locks.
            if surface.take_page_click() && is_active && !self.page_focused {
                self.page_focused = true;
                self.dirty = true;
            }
            // Double-Escape for a native page (#820): its Escapes never reach
            // gpui, so the monitor stamped them (`browser_keys::escape_step`
            // counts, 400 ms); the first Escape has already reached the page.
            for at in surface.take_escapes() {
                if is_active && self.page_focused {
                    let (double, next) =
                        infiniterm_core::browser_keys::escape_step(self.last_escape_ms, at);
                    self.last_escape_ms = next;
                    leave |= double;
                }
            }
            if surface.take_close_request() && tab.from_popup {
                self.page_closed.push(i);
            }
            if let Some(frame) = surface.take_frame() {
                // Only the visible tab's pixels are kept: a background tab's
                // texture would be a card-sized image nothing paints.
                if is_active {
                    if let Some(img) =
                        RgbaImage::from_raw(frame.width, frame.height, frame.bgra.clone())
                    {
                        // The snapshot is BGRA and gpui stores BGRA: the
                        // bytes go in as they are.
                        tab.texture = Some(Arc::new(RenderImage::new(SmallVec::from_elem(
                            ImageFrame::new(img),
                            1,
                        ))));
                        tab.texture_css = (frame.css_width, frame.css_height);
                        if crate::page_surface::keylog() {
                            eprintln!(
                                "[browser] ui takes frame {}x{}px css {}x{}",
                                frame.width, frame.height, frame.css_width, frame.css_height
                            );
                        }
                        self.dirty = true;
                    }
                }
            }
            // An app link is the page's own doing, in any tab, and is asked
            // about whichever tab it came from (#668).
            self.external_links.extend(surface.take_external_links());
            self.downloads.extend(surface.take_downloads());
            if is_active {
                self.popups.extend(surface.take_popups());
                if let Some(request) = surface.take_context_menu() {
                    self.context_menu = Some(request);
                }
            } else {
                // A background tab's queues are still drained so they cannot
                // grow: a popup from a tab you are not looking at has nowhere
                // to land, and a menu belongs to the click that opened it.
                let _ = surface.take_popups();
                let _ = surface.take_context_menu();
            }
            if tab.applied_zoom != zoom {
                surface.set_zoom(zoom.unwrap_or(1.));
                tab.applied_zoom = zoom;
            }
            if let Some(url) = surface.url() {
                if url != tab.url && url != "about:blank" {
                    tab.url = url.clone();
                    if is_active {
                        active_moved = Some(url);
                    }
                }
            }
        }
        if leave {
            // Unlocked: `browsers.rs` mirrors `page_focused` onto
            // `card.locked`, and the keyboard goes back to gpui.
            self.set_focus(false);
            self.dirty = true;
        }
        active_moved
    }

    /// The active tab's page title, but only when it has changed since the
    /// last call. The omnibox's history wants it; nothing else does yet.
    pub fn take_title(&mut self) -> Option<String> {
        let tab = self.tabs.get_mut(self.active)?;
        let t = tab.surface.as_ref()?.title()?;
        if Some(&t) == tab.title.as_ref() {
            return None;
        }
        tab.title = Some(t.clone());
        Some(t)
    }

    /// The active tab's page title as last reported (`take_title` keeps it
    /// current), or `None` before the page has said one.
    pub fn active_title(&self) -> Option<&str> {
        self.tabs.get(self.active)?.title.as_deref()
    }

    pub fn navigate(&mut self, url: &str) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        tab.url = url.to_string();
        if let Some(s) = &tab.surface {
            s.navigate(url);
        }
    }

    pub fn set_focus(&mut self, on: bool) {
        if self.page_focused != on {
            self.page_focused = on;
            self.apply_focus();
        }
    }

    pub fn close(&mut self) {
        for tab in &mut self.tabs {
            tab.close();
        }
    }

    /// Releases the last texture painted, if any, from gpui's sprite atlas.
    /// Called from `paint.rs::reconcile_bodies` right before this card's
    /// body is dropped: a closed card's last frame is not otherwise freed
    /// (see `painted_texture`'s own doc).
    pub fn drop_texture(&mut self, window: &mut Window) {
        if let Some(img) = self.painted_texture.take() {
            let _ = window.drop_image(img);
        }
    }

    // Only a click's modifiers reach a page from here (`mouse_button`); typing
    // and the wheel go to the web view through AppKit.
    fn mods(m: &gpui::Modifiers) -> Mods {
        Mods {
            shift: m.shift,
            control: m.control,
            alt: m.alt,
            command: m.platform,
        }
    }
}

impl Drop for BrowserBody {
    fn drop(&mut self) {
        self.close();
    }
}

/// The size the page is laid out at: the card's world size minus the strip's
/// own band (`tab_strip::strip_world_h`, shared with the editor's strip).
/// The strip used to be an overlay painted OVER the page, which covered real
/// page content under the strip's own rows rather than making room for it.
/// This is real space instead: the page is laid out in the band below the
/// strip, and every mouse coordinate forwarded to a surface is shifted by the
/// same amount (see `mouse_down`).
fn page_world_size(world: Size, font_px: f64, ui_scale: f32) -> Size {
    Size {
        w: world.w,
        h: (world.h - strip_world_h(font_px, ui_scale)).max(1.),
    }
}

/// Whether the tile last painted (`last`), if any, should be evicted now
/// that `new` was just painted into gpui's sprite atlas: never when the
/// two are the SAME image, which is every frame nothing new has arrived
/// from the web view and the unchanged texture is simply repainted again.
fn texture_changed(last: Option<gpui::ImageId>, new: gpui::ImageId) -> bool {
    last.is_some_and(|id| id != new)
}

impl CardBody for BrowserBody {
    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        scale: f64,
        focused: bool,
        now: f64,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.dirty = false;
        self.painted_focused = focused;
        if !focused {
            self.set_focus(false);
        }
        // Every tab follows the zoom and the strip's own share of the card,
        // not just the visible one: a background tab has to be the right
        // size the moment it becomes active. The web view's `resize` no-ops
        // when nothing changed, so calling it every frame also picks up a
        // live `ui.scale` change without a second code path.
        let page = page_world_size(self.world, self.style.font_px, self.ui_scale);
        let (page_w, page_h) = (page.w.round() as i32, page.h.round() as i32);
        for tab in &self.tabs {
            if let Some(s) = &tab.surface {
                s.resize(page_w, page_h);
            }
        }
        // The strip is real space, not an overlay: the page is laid out and
        // placed in the band BELOW it, matching what `page_world_size`
        // asked for. `mouse_down` and friends shift every coordinate they
        // forward by the same amount. `strip_h` here is the same formula
        // `paint_strip` uses internally (screen pixels: world height times
        // `scale`), computed ahead of it so the page lands in the right band
        // BEFORE the strip is drawn on top of it, last, below.
        let strip_h = px((strip_world_h(self.style.font_px, self.ui_scale) * scale) as f32);
        let page_bounds = Bounds::new(
            point(bounds.origin.x, bounds.origin.y + strip_h),
            size(
                bounds.size.width,
                (bounds.size.height - strip_h).max(px(1.)),
            ),
        );
        // A native page (the system web view) is placed over the page area,
        // not painted: its rect, its page zoom, whether it shows live or as
        // the snapshot that arrives below as this tab's texture, and its
        // wash. A tab that is not the visible one is hidden.
        let page_zoom = policy::page_zoom(self.zoom, scale);
        let page_rect = [
            f32::from(page_bounds.origin.x) as f64,
            f32::from(page_bounds.origin.y) as f64,
            f32::from(page_bounds.size.width) as f64,
            f32::from(page_bounds.size.height) as f64,
        ];
        // While the canvas zooms the page is its picture (#839), back live
        // once the zoom has been still for `ZOOM_SETTLE_MS`.
        self.zoom_changed_at =
            policy::zoom_changed_at(self.last_page_zoom, page_zoom, self.zoom_changed_at, now);
        self.last_page_zoom = Some(page_zoom);
        let zooming = policy::zoom_settling(self.zoom_changed_at, now);
        if !zooming {
            self.zoom_changed_at = None;
        }
        let view_state = policy::page_view(&policy::PageState {
            zoom: page_zoom,
            fully_covered: policy::fully_covered(&page_rect, &self.covers),
            zooming,
        });
        let wash: gpui::Rgba = self.card_bg.into();
        for (i, tab) in self.tabs.iter().enumerate() {
            let Some(s) = tab.surface.as_deref().filter(|s| s.is_native()) else {
                continue;
            };
            if i != self.active {
                s.hide();
                continue;
            }
            s.place(
                window,
                &Placement {
                    rect: page_rect,
                    zoom: page_zoom,
                    canvas_scale: scale,
                    world: (page_w, page_h),
                    snapshot: view_state == policy::PageView::Snapshot,
                    covers: self.covers.clone(),
                    focused,
                    locked: self.page_focused,
                    dim: policy::dim_alpha(focused, self.inactive_dim),
                    dim_rgb: [wash.r, wash.g, wash.b],
                    now_ms: now,
                },
            );
        }
        let radius = crate::chrome::card_radius(cx);
        window.paint_quad(fill(bounds, self.card_bg).corner_radii(radius));
        let texture_css = self.active_tab().map(|t| t.texture_css).unwrap_or((0, 0));
        match self.active_tab().and_then(|t| t.texture.clone()) {
            Some(img) => {
                // The page is the card's bottom edge (the strip covers the top).
                let page_corners = gpui::Corners {
                    top_left: px(0.),
                    top_right: px(0.),
                    bottom_left: radius,
                    bottom_right: radius,
                };
                // The snapshot is drawn at the size it was LAID OUT at (times
                // the canvas zoom), anchored top-left and clipped to the page
                // area: while a resize waits for a new snapshot the newest
                // one is the old size, and stretching it to the new bounds
                // squashed the page (#479). A settled card, and a canvas
                // zoom, give the page area back exactly
                // (`webview_policy::frame_draw_size`).
                let (draw_w, draw_h) = policy::frame_draw_size(
                    texture_css.0,
                    texture_css.1,
                    scale,
                    f32::from(page_bounds.size.width) as f64,
                    f32::from(page_bounds.size.height) as f64,
                );
                let fills = (draw_w - f32::from(page_bounds.size.width) as f64).abs() < 0.01
                    && (draw_h - f32::from(page_bounds.size.height) as f64).abs() < 0.01;
                if crate::page_surface::keylog() && !fills {
                    eprintln!(
                        "[browser] draws old size {draw_w:.0}x{draw_h:.0} in page area {:.0}x{:.0} (css {}x{} zoom {scale:.2})",
                        f32::from(page_bounds.size.width),
                        f32::from(page_bounds.size.height),
                        texture_css.0,
                        texture_css.1
                    );
                }
                if fills {
                    let _ = window.paint_image(page_bounds, page_corners, img.clone(), 0, false);
                } else {
                    // A transient frame of another size: square corners, the
                    // card's own rounded quad is already painted under it.
                    let drawn = Bounds::new(
                        page_bounds.origin,
                        size(px(draw_w as f32), px(draw_h as f32)),
                    );
                    window.with_content_mask(
                        Some(gpui::ContentMask {
                            bounds: page_bounds,
                        }),
                        |window| {
                            let _ = window.paint_image(
                                drawn,
                                gpui::Corners::default(),
                                img.clone(),
                                0,
                                false,
                            );
                        },
                    );
                }
                // Only a DIFFERENT image is ever evicted: the same texture
                // is repainted, unchanged, every frame nothing new has
                // arrived, and dropping what this very call just referenced
                // would be a bug, not a fix.
                let last_id = self.painted_texture.as_ref().map(|p| p.id);
                if texture_changed(last_id, img.id) {
                    if let Some(old) = self.painted_texture.replace(img) {
                        let _ = window.drop_image(old);
                    }
                } else if self.painted_texture.is_none() {
                    self.painted_texture = Some(img);
                }
            }
            None => {
                let font_size = crate::glyph_size::zoomed(self.style.font_px, scale, window);
                if font_size >= px(crate::chrome::LEGIBLE_FONT_PX as f32) {
                    let text = format!("loading {}", self.active_url());
                    let line = crate::text::shape(
                        window,
                        &text,
                        font_size,
                        &self.style.regular,
                        self.text,
                    );
                    let _ = line.paint(
                        point(
                            page_bounds.origin.x + px((STATUS_TEXT_PAD_PX * scale) as f32),
                            page_bounds.origin.y + px((STATUS_TEXT_PAD_PX * scale) as f32),
                        ),
                        font_size * STATUS_LINE_HEIGHT_RATIO,
                        window,
                        cx,
                    );
                }
            }
        }
        if !focused && self.inactive_dim > 0. {
            window.paint_quad(crate::chrome::card_wash(
                cx,
                bounds,
                crate::chrome::with_alpha(self.card_bg, self.inactive_dim as f32),
            ));
        }
        // The strip, painted LAST so it sits over the page rather than
        // under it: shared with the editor's strip, so the two look and
        // size alike (`ui.fontSize`, times `ui_scale`, times the zoom).
        let labels: Vec<String> = self.tabs.iter().map(|t| tab_label(t).to_string()).collect();
        paint_strip(
            bounds,
            scale,
            self.ui_scale,
            &labels,
            self.active,
            self.tab_drag.as_ref(),
            self.card_number,
            self.protected,
            None,
            &self.style,
            window,
            cx,
        );
        // Over the page, after the strip. Not while a tab is being dragged.
        let dragging = self.tab_drag.is_some_and(|d| d.moving);
        if let Some(tab) = self.hover_tab.filter(|_| !dragging) {
            if let Some(t) = self.tabs.get(tab) {
                let lines = hover_lines(tab_label(t), Some(&t.url), self.style.hover_detail);
                paint_hover_label(
                    bounds,
                    scale,
                    self.ui_scale,
                    tab,
                    0.,
                    &lines,
                    &self.style,
                    window,
                    cx,
                );
            }
        }
    }

    fn resized(&mut self, world: Size) {
        self.world = world;
        let page = page_world_size(world, self.style.font_px, self.ui_scale);
        for tab in &self.tabs {
            if let Some(s) = &tab.surface {
                s.resize(page.w.round() as i32, page.h.round() as i32);
            }
        }
        self.dirty = true;
    }

    fn key(&mut self, k: &Keystroke, _now: f64, _cx: &mut App) -> BodyAction {
        // The keyboard's way into focus lock: arrow-focusing a card never
        // gives the page focus, and until now there was no way to lock one
        // without reaching for the mouse. A bare Enter on a focused-but-
        // unlocked card is the keyboard's scrim click — it activates the
        // card the same way Enter already does everywhere else in this
        // app (a phantom slot, an omnibox result), and is swallowed rather
        // than forwarded, so the page never sees the Enter that opened it.
        if !self.page_focused
            && k.key == "enter"
            && !k.modifiers.platform
            && !k.modifiers.control
            && !k.modifiers.alt
            && !k.modifiers.shift
        {
            self.set_focus(true);
            self.dirty = true;
            return BodyAction::None;
        }
        let Some(surface) = self.active_surface() else {
            return BodyAction::None;
        };
        // Typing never gets here (the page has the keyboard); an edit chord
        // the app did not claim does.
        if k.modifiers.platform {
            surface.edit_chord(&k.key, k.modifiers.shift);
        }
        BodyAction::None
    }

    fn mouse_down(
        &mut self,
        local: Point,
        button: gpui::MouseButton,
        modifiers: &gpui::Modifiers,
        clicks: usize,
    ) -> BodyAction {
        // The strip is chrome, not page content, and the page no longer
        // paints under it (see `page_world_size`): anywhere in its row is
        // ours, a live button or dead space between them, never the page's.
        let strip_h = strip_world_h(self.style.font_px, self.ui_scale);
        if local.y < strip_h {
            return match strip_hit(
                local,
                self.style.font_px,
                self.ui_scale,
                self.tabs.len(),
                0.,
                self.style.pad_x(),
            ) {
                Some(hit) if button == gpui::MouseButton::Left => {
                    // The press switches; held and moved, it drags (#682).
                    if let crate::body::TabClick::Switch(i) = hit {
                        self.tab_drag = Some(TabDrag::new(i, local.x));
                    }
                    BodyAction::BrowserTab(hit)
                }
                // Middle-click closes a tab, as in every browser.
                Some(crate::body::TabClick::Switch(i)) if button == gpui::MouseButton::Middle => {
                    BodyAction::BrowserTab(crate::body::TabClick::Close(i))
                }
                _ => BodyAction::None,
            };
        }
        if self.active_surface().is_none() {
            return BodyAction::None;
        }
        // The scrim takes the first click: it focused the card.
        if !self.painted_focused && !self.page_focused {
            self.set_focus(true);
            self.dirty = true;
            return BodyAction::None;
        }
        let b = match button {
            gpui::MouseButton::Left => Button::Left,
            gpui::MouseButton::Middle => Button::Middle,
            gpui::MouseButton::Right => Button::Right,
            _ => return BodyAction::None,
        };
        self.set_focus(true);
        if b == Button::Left {
            self.left_down = true;
        }
        if let Some(surface) = self.active_surface() {
            surface.mouse_button(
                local.x as f32,
                (local.y - strip_h) as f32,
                Self::mods(modifiers),
                b,
                false,
                clicks,
            );
        }
        BodyAction::None
    }

    fn mouse_up(&mut self, local: Point, button: gpui::MouseButton, modifiers: &gpui::Modifiers) {
        if let Some(drag) = self.tab_drag.take() {
            self.tab_drop = drag.drop(self.style.font_px, self.ui_scale, self.tabs.len());
            self.dirty |= drag.moving;
            return;
        }
        let b = match button {
            gpui::MouseButton::Left => Button::Left,
            gpui::MouseButton::Middle => Button::Middle,
            gpui::MouseButton::Right => Button::Right,
            _ => return,
        };
        // Released before the surface is looked up: a card whose tab has no
        // surface must still end the drag it captured, or the pointer stays
        // owned by a body that will never let go.
        if b == Button::Left {
            self.left_down = false;
        }
        // A drag that started on the page (the only kind that reaches here:
        // `captures_drag` is `left_down`, only ever set below the strip) can
        // still end with the pointer dragged back up over the strip's row;
        // clamped rather than sent negative, off the top of the page.
        let page_y = (local.y - strip_world_h(self.style.font_px, self.ui_scale)).max(0.);
        if let Some(surface) = self.active_surface() {
            surface.mouse_button(
                local.x as f32,
                page_y as f32,
                Self::mods(modifiers),
                b,
                true,
                1,
            );
        }
    }

    // Pointer moves over the page itself go to the web view through AppKit,
    // so only the strip's hover and a tab drag are ours.
    fn mouse_move(&mut self, local: Point, _modifiers: &gpui::Modifiers) {
        if let Some(drag) = self.tab_drag.as_mut() {
            if drag.moved(local.x, self.style.font_px, self.ui_scale) {
                self.dirty = true;
            }
            return;
        }
        let over = self.tab_at(local);
        self.set_hover(over);
    }

    fn mouse_leave(&mut self) {
        self.set_hover(None);
    }

    fn set_covers(&mut self, covers: Vec<[f64; 4]>) {
        if self.covers != covers {
            self.covers = covers;
            self.dirty = true;
        }
    }

    fn wants_frame(&self, now: f64) -> bool {
        // Any tab's new frame asks for one: a background tab's pixels are
        // dropped, but `sync` is what drops them, and it runs per frame.
        // A native page also asks when its snapshot is due, and a zoom
        // until the frame after it settles, which shows the page live again.
        self.dirty
            || self.zoom_changed_at.is_some()
            || self.tabs.iter().any(|t| {
                t.surface
                    .as_ref()
                    .is_some_and(|s| s.has_new_frame() || s.wants_frame(now))
            })
    }

    fn captures_drag(&self) -> bool {
        self.left_down || self.tab_drag.is_some()
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// The strip style tests build a body with: exact colours don't matter to
/// any of them, only `font_px`, which every strip-geometry test needs.
#[cfg(test)]
fn test_strip_style() -> StripStyle {
    StripStyle {
        font_px: 14.,
        ..crate::tab_strip::strip_style(
            &crate::chrome::Chrome::default_chrome(),
            &Default::default(),
        )
    }
}

#[cfg(test)]
mod tab_tests {
    use super::*;

    // The web view is made lazily when a window places it, so a body can be
    // built and its tabs managed with no window (these tests).
    fn test_engine() -> Engine {
        Engine {
            store: [0; 16],
            devtools: false,
        }
    }

    #[test]
    fn a_new_body_has_one_tab() {
        let body = BrowserBody::new(
            "card-1",
            "https://a.example",
            Size { w: 10., h: 10. },
            test_strip_style(),
            test_engine(),
        );
        assert_eq!(body.tabs.len(), 1);
        assert_eq!(body.active, 0);
    }

    #[test]
    fn a_closed_tab_before_the_active_one_pulls_the_active_index_down() {
        let mut body = BrowserBody::new(
            "card-1",
            "a",
            Size { w: 10., h: 10. },
            test_strip_style(),
            test_engine(),
        );
        body.open_tab("b");
        body.open_tab("c");
        body.set_active(2);
        body.close_tab(0);
        assert_eq!(body.tabs.len(), 2);
        assert_eq!(body.active, 1);
        assert_eq!(body.active_url(), "c");
    }

    // #672: only a tab a page opened may be closed by that page's own
    // `window.close()`. The tab made for an address `browsers.rs` noted as a
    // popup is one; a tab the person opened, or the card's first, is not.
    // The note is used once: a second tab with the same address by hand is
    // the person's.
    #[test]
    fn a_tab_made_for_a_popup_address_may_be_closed_by_its_page_and_others_not() {
        let mut body = BrowserBody::new(
            "card-1",
            "https://first.example",
            Size { w: 10., h: 10. },
            test_strip_style(),
            test_engine(),
        );
        body.popup_urls.push("https://login.example/popup".into());
        body.open_tab("https://by-hand.example");
        body.open_tab("https://login.example/popup");
        body.open_tab("https://login.example/popup");
        let flags: Vec<bool> = body.tabs.iter().map(|t| t.from_popup).collect();
        assert_eq!(flags, [false, false, true, false]);
        assert!(body.popup_urls.is_empty(), "each note is used once");
    }
}

#[cfg(test)]
mod tab_label_tests {
    use super::*;

    #[test]
    fn a_tab_shows_its_title_or_falls_back_to_its_url() {
        let mut tab = Tab {
            url: "https://a.example".into(),
            surface: None,
            texture: None,
            texture_css: (0, 0),
            title: None,
            applied_zoom: None,
            from_popup: false,
        };
        assert_eq!(tab_label(&tab), "https://a.example");
        tab.title = Some("Example Domain".into());
        assert_eq!(tab_label(&tab), "Example Domain");
    }
}

#[cfg(test)]
mod mouse_down_strip_tests {
    use super::*;
    use crate::body::CardBody;
    use gpui::Modifiers;

    // Past the "+" button but still inside the strip's row, `strip_hit`
    // returns `None` (dead chrome space); this must be swallowed rather
    // than falling through as a page click, since the page no longer
    // paints under the strip at all.
    #[test]
    fn dead_space_in_the_strips_row_is_swallowed_not_forwarded_to_the_page() {
        let mut body = BrowserBody::new(
            "card-1",
            "https://a.example",
            Size { w: 800., h: 600. },
            test_strip_style(),
            crate::page_surface::Engine {
                store: [0; 16],
                devtools: false,
            },
        );
        let action = body.mouse_down(
            Point { x: 1000., y: 5. },
            gpui::MouseButton::Left,
            &Modifiers::default(),
            1,
        );
        assert_eq!(action, BodyAction::None);
    }
}

#[cfg(test)]
mod page_world_size_tests {
    use super::*;

    // What the page is laid out at: the strip's band comes off the
    // card's own height, never its width, and never at `ui_scale` 1's cost
    // alone — a bigger interface takes a bigger bite.
    #[test]
    fn the_strips_band_comes_off_the_cards_height_only() {
        let world = Size { w: 800., h: 600. };
        let page = page_world_size(world, 14., 1.);
        assert_eq!(page.w, 800.);
        assert_eq!(page.h, 600. - strip_world_h(14., 1.));

        let scaled = page_world_size(world, 14., 2.);
        assert_eq!(scaled.h, 600. - strip_world_h(14., 2.));
    }

    // A card shorter than the strip itself must not lay the page out at a
    // negative or zero height: a page-space calculation going negative would
    // send mouse coordinates the wrong direction first.
    #[test]
    fn a_card_shorter_than_the_strip_still_gets_a_positive_page_height() {
        let tiny = Size { w: 100., h: 10. };
        assert!(page_world_size(tiny, 14., 1.).h > 0.);
    }
}

#[cfg(test)]
mod texture_changed_tests {
    use super::*;
    use gpui::ImageId;

    // The first frame ever painted has nothing to evict: `None` means
    // nothing was tracked yet, not "the atlas is somehow already stale".
    #[test]
    fn nothing_painted_yet_is_never_a_change() {
        assert!(!texture_changed(None, ImageId(1)));
    }

    // The same id painted again is every frame no new snapshot has produced a new
    // one: evicting it would drop the exact tile this frame's own scene
    // still points at.
    #[test]
    fn the_same_id_again_is_not_a_change() {
        assert!(!texture_changed(Some(ImageId(7)), ImageId(7)));
    }

    // A different id is a real new snapshot: the old tile is no longer
    // referenced by anything painted and is safe to evict.
    #[test]
    fn a_different_id_is_a_change() {
        assert!(texture_changed(Some(ImageId(7)), ImageId(8)));
    }
}
