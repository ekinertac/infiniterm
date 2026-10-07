//! The browser card: one CEF surface per tab, the active one painted as a
//! texture inside the card. Port of `BrowserCard.svelte` on
//! `infiniterm-browser`, which is the spike's `browser.rs` made permanent,
//! extended for tabs per
//! `docs/superpowers/specs/2026-09-19-browser-tabs-design.md`.
//!
//! Every tab lays out at the card's size in world units and is drawn at
//! whatever the zoom makes of it, so labels, rings and the palette paint
//! over it like over any card and it scales with the canvas. Only the
//! active tab's pixels are kept, but every tab's surface stays LIVE the
//! way Chrome's do: a background tab keeps loading, keeps its scroll
//! position, keeps its size, so switching to it shows the page rather
//! than a blank waiting to re-render. Keys: the app owns Cmd except the
//! edit chords a page needs (copy, paste, cut, select all, undo) and the
//! page-zoom chords the model rebinds (`browser_keys.rs`); everything
//! else goes to the page once it has focus. Focus is deliberate: a click
//! on an UNFOCUSED card only focuses it (the scrim takes it), the next
//! reaches the page; arrow-focusing a card never gives the page focus.
//! That same `page_focused` IS the keyboard lock the tab chords need, so
//! the tab-management chords never reach a surface at all: they are
//! `Model::browser_tab_*` (`tabs_cmd.rs`), and what arrives here is the
//! changed `card.tabs` that `browsers.rs` reconciles against `self.tabs`.
//!
//! Without CEF (the bare binary outside a bundle) the card says so and
//! stays a rectangle, with one placeholder tab.
use crate::body::{BodyAction, CardBody};
use crate::tab_strip::{paint_strip, strip_hit, strip_world_h, StripStyle};
use gpui::{
    fill, font, point, px, size, App, Bounds, CursorStyle, Hsla, Keystroke, Pixels, RenderImage,
    Window,
};
use image::{Frame as ImageFrame, RgbaImage};
use infiniterm_browser::{Button, Mods, Surface};
use infiniterm_core::grid::{Point, Size};
use smallvec::SmallVec;
use std::sync::Arc;

/// The device scale CEF renders at moves in steps this fine, so an
/// animated zoom settles on one value instead of asking for a new frame
/// size every tick.
const DEVICE_SCALE_STEPS_PER_UNIT: f64 = 2.;
/// CEF is never asked to render past this device scale: a sanity ceiling
/// on how many pixels a zoomed-in browser card can demand.
const DEVICE_SCALE_MAX: f64 = 3.;
/// The "loading"/"unavailable" placeholder text's font size.
const STATUS_FONT_PX: f64 = 13.;
/// The placeholder text's inset from the card's corner.
const STATUS_TEXT_PAD_PX: f64 = 12.;
/// The placeholder text's line height, looser than its font size.
const STATUS_LINE_HEIGHT_RATIO: f32 = 1.5;

/// One tab: everything that was a single field on `BrowserBody` before
/// tabs existed, once per open page.
pub struct Tab {
    pub url: String,
    pub surface: Option<Surface>,
    /// Why there is no surface, for the card to say.
    pub unavailable: Option<String>,
    texture: Option<Arc<RenderImage>>,
    /// The page title as last reported, so a change can be told from the
    /// same title arriving every frame. Not the card's title: that is a
    /// name somebody CHOSE, and a page must never overwrite it.
    title: Option<String>,
    /// The card's page zoom as last pushed to THIS surface. Per tab
    /// because a tab opened after the zoom was set has not had it yet.
    applied_zoom: Option<f64>,
}

impl Tab {
    fn open(url: &str, world: Size, scale: f32, cef_running: bool) -> Tab {
        let (surface, unavailable) = if cef_running {
            match Surface::open(url, world.w.round() as i32, world.h.round() as i32, scale) {
                Some(s) => (Some(s), None),
                None => (None, Some("could not create the browser".to_string())),
            }
        } else {
            (
                None,
                Some("browser cards need the app bundle (CEF is not loaded)".to_string()),
            )
        };
        Tab {
            url: url.to_string(),
            surface,
            unavailable,
            texture: None,
            title: None,
            applied_zoom: None,
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
    /// Which tab was last told it has CEF's keyboard focus, so the handover
    /// on a tab switch takes it off exactly one surface. CEF keeps focus per
    /// browser and a fresh one has never had it, so without this a new tab
    /// would take no typing at all.
    focused_tab: Option<usize>,
    /// The page's own zoom the card carries (`card.zoom`), applied to every
    /// tab: a background tab must already be at the card's zoom when it
    /// becomes the visible one.
    pub zoom: Option<f64>,
    world: Size,
    scale: f32,
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
    /// alike (`terminal.fontSize`, not a strip-only constant).
    pub style: StripStyle,
    pub font_family: String,
    dirty: bool,
    pub popups: Vec<String>,
    /// A right-click since the last drain, for `browsers.rs` to turn into
    /// the app-level menu overlay. Aggregated across tabs rather than kept
    /// per tab: a menu only ever opens from a click on the visible page.
    pub context_menu: Option<infiniterm_browser::ContextMenuRequest>,
    /// The texture last painted into gpui's sprite atlas, so `paint` can
    /// evict it the moment a new one takes its place. gpui has no atlas
    /// LRU: every `RenderImage::new` mints a fresh id, and nothing but
    /// `Window::drop_image` ever frees the tile it was painted into, so
    /// without this an open browser card grows the atlas by one tile per
    /// CEF frame for as long as it runs.
    painted_texture: Option<Arc<RenderImage>>,
    /// Remembered so a tab opened later is built the same way the first
    /// one was.
    cef_running: bool,
}

impl BrowserBody {
    /// The tab under a point of the card (a right-click's), `None` off the
    /// tabs, on the "+" and below the strip.
    pub fn tab_at(&self, local: Point) -> Option<usize> {
        match strip_hit(local, self.style.font_px, self.ui_scale, self.tabs.len()) {
            Some(crate::body::TabClick::Switch(i) | crate::body::TabClick::Close(i)) => Some(i),
            _ => None,
        }
    }

    pub fn new(
        card_id: &str,
        url: &str,
        world: Size,
        scale: f32,
        cef_running: bool,
        style: StripStyle,
    ) -> BrowserBody {
        BrowserBody {
            card_id: card_id.to_string(),
            // ui_scale defaults to 1. below, same as every other mirrored
            // field, until `reconcile_browsers` pushes the real one.
            tabs: vec![Tab::open(
                url,
                page_world_size(world, style.font_px, 1.),
                scale,
                cef_running,
            )],
            active: 0,
            locked: false,
            focused_tab: None,
            zoom: None,
            world,
            scale,
            painted_focused: false,
            page_focused: false,
            last_escape_ms: None,
            left_down: false,
            inactive_dim: crate::chrome::INACTIVE_DIM_DEFAULT,
            card_number: 0,
            protected: false,
            ui_scale: 1.,
            card_bg: gpui::rgb(0x0e101a).into(),
            text: gpui::rgb(0xb9c4d2).into(),
            style,
            font_family: "Menlo".into(),
            dirty: true,
            popups: vec![],
            context_menu: None,
            painted_texture: None,
            cef_running,
        }
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    /// The surface every key, click and command goes to: the visible tab's.
    pub fn active_surface(&self) -> Option<&Surface> {
        self.active_tab().and_then(|t| t.surface.as_ref())
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
        self.tabs
            .push(Tab::open(url, page, self.scale, self.cef_running));
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
        let (scale, cef) = (self.scale, self.cef_running);
        self.tabs = urls
            .iter()
            .map(|u| Tab::open(u, page, scale, cef))
            .collect();
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

    /// Exactly the active tab's surface holds CEF's keyboard focus, and only
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
        let zoom = self.zoom;
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            let Some(surface) = &tab.surface else {
                continue;
            };
            let is_active = i == self.active;
            if let Some(frame) = surface.take_frame() {
                // Only the visible tab's pixels are kept: a background tab's
                // texture would be a card-sized image nothing paints.
                if is_active {
                    if let Some(img) =
                        RgbaImage::from_raw(frame.width, frame.height, frame.bgra.clone())
                    {
                        // CEF hands BGRA and gpui stores BGRA: the bytes go in as they are.
                        tab.texture = Some(Arc::new(RenderImage::new(SmallVec::from_elem(
                            ImageFrame::new(img),
                            1,
                        ))));
                        self.dirty = true;
                    }
                }
            }
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

    /// The OS cursor the active page last asked for (a pointer over a link,
    /// an I-beam over an input), or `Arrow` before it has said anything.
    pub fn cursor_style(&self) -> CursorStyle {
        cursor_style_for(self.active_surface().map_or("default", |s| s.cursor()))
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

/// What CEF actually renders into: the card's world size minus the strip's
/// own band (`tab_strip::strip_world_h`, shared with the editor's strip).
/// The strip used to be an overlay painted OVER the page after it painted
/// at the full card height, which covered real page content under the
/// strip's own rows rather than making room for it — tabs read as though
/// they were slicing the top off every page, because they were. This is
/// real space instead: the page is laid out and painted into the band
/// below the strip, and every mouse coordinate forwarded to a surface is
/// shifted by the same amount (see `mouse_down` and friends), so what is
/// under the cursor on screen is what the page sees.
fn page_world_size(world: Size, font_px: f64, ui_scale: f32) -> Size {
    Size {
        w: world.w,
        h: (world.h - strip_world_h(font_px, ui_scale)).max(1.),
    }
}

/// Whether the tile last painted (`last`), if any, should be evicted now
/// that `new` was just painted into gpui's sprite atlas: never when the
/// two are the SAME image, which is every frame nothing new has arrived
/// from CEF and the unchanged texture is simply repainted again.
fn texture_changed(last: Option<gpui::ImageId>, new: gpui::ImageId) -> bool {
    last.is_some_and(|id| id != new)
}

impl CardBody for BrowserBody {
    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        scale: f64,
        focused: bool,
        _now: f64,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.dirty = false;
        self.painted_focused = focused;
        if !focused {
            self.set_focus(false);
        }
        // Drawn above 100%, the page is asked for more pixels rather than
        // upscaled: the device scale follows the zoom in half steps, so an
        // animation settles on one value instead of re-rendering per frame.
        let device = (self.scale as f64 * scale * DEVICE_SCALE_STEPS_PER_UNIT).ceil()
            / DEVICE_SCALE_STEPS_PER_UNIT;
        let device = device.clamp(self.scale as f64, DEVICE_SCALE_MAX) as f32;
        // Every tab follows the zoom and the strip's own share of the card,
        // not just the visible one: a background tab has to be the right
        // size the moment it becomes active, or the first frame after the
        // switch is a stretched page. `Surface::resize` no-ops when nothing
        // actually changed, so calling it every frame (rather than only on
        // a scale change, as before) is what also picks up a live
        // `ui.scale` change without a second code path to keep in step.
        let page = page_world_size(self.world, self.style.font_px, self.ui_scale);
        let (page_w, page_h) = (page.w.round() as i32, page.h.round() as i32);
        for tab in &self.tabs {
            if let Some(s) = &tab.surface {
                s.resize(page_w, page_h, device);
            }
        }
        // The strip is real space now, not an overlay: the page is laid out
        // and painted into the band BELOW it, matching what `page_world_size`
        // asked CEF to render. `mouse_down`/`mouse_up`/`mouse_move`/`wheel`
        // shift every coordinate they forward by the same amount, so what is
        // under the cursor on screen is what the page sees. Painting the
        // page at the full card height and covering its top rows with an
        // opaque strip was tried first and read as tabs slicing the top off
        // every page, which is exactly what it was doing. `strip_h` here is
        // the same formula `paint_strip` uses internally (screen pixels:
        // world height times `scale`), computed ahead of it so the page can
        // be painted into the right band BEFORE the strip is drawn on top
        // of it, last, below.
        let strip_h = px((strip_world_h(self.style.font_px, self.ui_scale) * scale) as f32);
        let page_bounds = Bounds::new(
            point(bounds.origin.x, bounds.origin.y + strip_h),
            size(
                bounds.size.width,
                (bounds.size.height - strip_h).max(px(1.)),
            ),
        );
        let radius = crate::chrome::card_radius(cx);
        window.paint_quad(fill(bounds, self.card_bg).corner_radii(radius));
        match self.active_tab().and_then(|t| t.texture.clone()) {
            Some(img) => {
                // The page is the card's bottom edge (the strip covers the top).
                let page_corners = gpui::Corners {
                    top_left: px(0.),
                    top_right: px(0.),
                    bottom_left: radius,
                    bottom_right: radius,
                };
                let _ = window.paint_image(page_bounds, page_corners, img.clone(), 0, false);
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
                let font_size = px((STATUS_FONT_PX * scale) as f32);
                if font_size >= px(crate::chrome::LEGIBLE_FONT_PX as f32) {
                    let text = self
                        .active_tab()
                        .and_then(|t| t.unavailable.clone())
                        .unwrap_or_else(|| format!("loading {}", self.active_url()));
                    let line = crate::text::shape(
                        window,
                        &text,
                        font_size,
                        &font(self.font_family.clone()),
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
        // size alike (the font is `terminal.fontSize`, times `ui_scale`,
        // times the zoom, not a strip-only constant).
        let labels: Vec<String> = self.tabs.iter().map(|t| tab_label(t).to_string()).collect();
        paint_strip(
            bounds,
            scale,
            self.ui_scale,
            &labels,
            self.active,
            self.card_number,
            self.protected,
            &self.style,
            window,
            cx,
        );
    }

    fn resized(&mut self, world: Size) {
        self.world = world;
        let page = page_world_size(world, self.style.font_px, self.ui_scale);
        for tab in &self.tabs {
            if let Some(s) = &tab.surface {
                let device = s.shared.borrow().scale;
                s.resize(page.w.round() as i32, page.h.round() as i32, device);
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
        if k.modifiers.platform {
            surface.edit_chord(&k.key, k.modifiers.shift);
            return BodyAction::None;
        }
        surface.key(&k.key, k.key_char.as_deref(), Self::mods(&k.modifiers));
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
            return match strip_hit(local, self.style.font_px, self.ui_scale, self.tabs.len()) {
                Some(hit) if button == gpui::MouseButton::Left => BodyAction::BrowserTab(hit),
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
        // clamped rather than sent negative, off the top of what CEF laid out.
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

    fn mouse_move(&mut self, local: Point, modifiers: &gpui::Modifiers) {
        let strip_h = strip_world_h(self.style.font_px, self.ui_scale);
        if local.y < strip_h {
            return;
        }
        if let Some(surface) = self.active_surface() {
            surface.mouse_move(
                local.x as f32,
                (local.y - strip_h) as f32,
                Self::mods(modifiers),
            );
        }
    }

    fn mouse_leave(&mut self) {
        if let Some(surface) = self.active_surface() {
            surface.mouse_leave();
        }
    }

    fn wheel(&mut self, local: Point, dx: f64, dy: f64, modifiers: &gpui::Modifiers) {
        let strip_h = strip_world_h(self.style.font_px, self.ui_scale);
        if local.y < strip_h {
            return;
        }
        if let Some(surface) = self.active_surface() {
            surface.wheel(
                local.x as f32,
                (local.y - strip_h) as f32,
                Self::mods(modifiers),
                dx as f32,
                dy as f32,
            );
        }
    }

    fn wants_frame(&self, _now: f64) -> bool {
        // Any tab's new frame asks for one: a background tab's pixels are
        // dropped, but `sync` is what drops them, and it runs per frame.
        self.dirty
            || self
                .tabs
                .iter()
                .any(|t| t.surface.as_ref().is_some_and(|s| s.has_new_frame()))
    }

    fn captures_drag(&self) -> bool {
        self.left_down
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// The CSS cursor keyword `Surface::cursor` reports, mapped to gpui's
/// cursor styles. gpui has no spinner or "all scroll" cursor, so the CEF
/// types that would need one (`wait`, `progress`, `help`, `move`) fall back
/// to the arrow rather than picking something misleading.
fn cursor_style_for(name: &str) -> CursorStyle {
    match name {
        "pointer" => CursorStyle::PointingHand,
        "text" => CursorStyle::IBeam,
        "crosshair" => CursorStyle::Crosshair,
        "grab" => CursorStyle::OpenHand,
        "grabbing" => CursorStyle::ClosedHand,
        "e-resize" => CursorStyle::ResizeRight,
        "w-resize" => CursorStyle::ResizeLeft,
        "n-resize" => CursorStyle::ResizeUp,
        "s-resize" => CursorStyle::ResizeDown,
        "ns-resize" => CursorStyle::ResizeUpDown,
        "ew-resize" => CursorStyle::ResizeLeftRight,
        "nesw-resize" => CursorStyle::ResizeUpRightDownLeft,
        "nwse-resize" => CursorStyle::ResizeUpLeftDownRight,
        "col-resize" => CursorStyle::ResizeColumn,
        "row-resize" => CursorStyle::ResizeRow,
        "not-allowed" => CursorStyle::OperationNotAllowed,
        "copy" => CursorStyle::DragCopy,
        "alias" => CursorStyle::DragLink,
        "context-menu" => CursorStyle::ContextualMenu,
        "vertical-text" => CursorStyle::IBeamCursorForVerticalLayout,
        "none" => CursorStyle::None,
        _ => CursorStyle::Arrow,
    }
}

/// The strip style tests build a body with: exact colours don't matter to
/// any of them, only `font_px`, which every strip-geometry test needs.
#[cfg(test)]
fn test_strip_style() -> StripStyle {
    StripStyle {
        bg: gpui::black(),
        border: gpui::black(),
        active_bg: gpui::black(),
        text_bright: gpui::white(),
        text_muted: gpui::white(),
        font_family: "Menlo".into(),
        font_px: 14.,
    }
}

#[cfg(test)]
mod tab_tests {
    use super::*;

    #[test]
    fn a_body_with_no_cef_still_reports_one_unavailable_tab() {
        let body = BrowserBody::new(
            "card-1",
            "https://a.example",
            Size { w: 10., h: 10. },
            1.,
            false,
            test_strip_style(),
        );
        assert_eq!(body.tabs.len(), 1);
        assert!(body.tabs[0].surface.is_none());
        assert!(body.tabs[0].unavailable.is_some());
        assert_eq!(body.active, 0);
    }

    #[test]
    fn a_closed_tab_before_the_active_one_pulls_the_active_index_down() {
        let mut body = BrowserBody::new(
            "card-1",
            "a",
            Size { w: 10., h: 10. },
            1.,
            false,
            test_strip_style(),
        );
        body.open_tab("b");
        body.open_tab("c");
        body.set_active(2);
        body.close_tab(0);
        assert_eq!(body.tabs.len(), 2);
        assert_eq!(body.active, 1);
        assert_eq!(body.active_url(), "c");
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
            unavailable: None,
            texture: None,
            title: None,
            applied_zoom: None,
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
            1.,
            false,
            test_strip_style(),
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

    // What CEF is actually asked to render: the strip's band comes off the
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

    // A card shorter than the strip itself must not ask CEF for a negative
    // or zero height: `Surface::open`/`resize` already floor at 1px on
    // their own side, but a page-space calculation going negative would
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

    // The same id painted again is every frame CEF has not produced a new
    // one: evicting it would drop the exact tile this frame's own scene
    // still points at.
    #[test]
    fn the_same_id_again_is_not_a_change() {
        assert!(!texture_changed(Some(ImageId(7)), ImageId(7)));
    }

    // A different id is a real new CEF frame: the old tile is no longer
    // referenced by anything painted and is safe to evict.
    #[test]
    fn a_different_id_is_a_change() {
        assert!(texture_changed(Some(ImageId(7)), ImageId(8)));
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;

    #[test]
    fn a_link_gets_a_pointer_and_an_input_gets_an_ibeam() {
        assert_eq!(cursor_style_for("pointer"), CursorStyle::PointingHand);
        assert_eq!(cursor_style_for("text"), CursorStyle::IBeam);
    }

    #[test]
    fn an_unmapped_or_default_cursor_falls_back_to_the_arrow() {
        assert_eq!(cursor_style_for("default"), CursorStyle::Arrow);
        assert_eq!(cursor_style_for("wait"), CursorStyle::Arrow);
    }
}
