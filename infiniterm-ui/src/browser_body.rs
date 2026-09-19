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
use crate::body::{BodyAction, CardBody, TabClick};
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
/// Below this the device scale hasn't really changed, just drifted in
/// floating point; resizing the surface for it would be wasted work.
const SCALE_CHANGE_EPSILON: f32 = 0.01;
/// The "loading"/"unavailable" placeholder text's font size.
const STATUS_FONT_PX: f64 = 13.;
/// The placeholder text's inset from the card's corner.
const STATUS_TEXT_PAD_PX: f64 = 12.;
/// The placeholder text's line height, looser than its font size.
const STATUS_LINE_HEIGHT_RATIO: f32 = 1.5;
/// The tab strip's height in screen pixels, divided by zoom like every
/// other piece of chrome.
const TAB_STRIP_HEIGHT_PX: f64 = 28.;
/// A tab's width, same units. Fixed rather than proportional: a card with
/// many tabs scrolls the strip in a later slice rather than shrinking
/// every tab to a sliver, which is not in this one (see the design doc).
const TAB_STRIP_TAB_WIDTH_PX: f64 = 140.;
const TAB_STRIP_FONT_PX: f64 = 11.;
const TAB_STRIP_LABEL_PAD_PX: f64 = 8.;
/// The close button's width within a tab's own band, screen pixels: the
/// rightmost slice of every tab is its `×`, everything left of that
/// switches to it.
const TAB_STRIP_CLOSE_WIDTH_PX: f64 = 20.;

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
    /// Mirrors `Model::ui_scale`, the same way `card_bg`/`text` do, so the
    /// strip's screen-pixel sizes scale with the interface multiplier like
    /// every other piece of chrome.
    pub ui_scale: f32,
    pub card_bg: Hsla,
    pub text: Hsla,
    pub font_family: String,
    dirty: bool,
    pub popups: Vec<String>,
    /// A right-click since the last drain, for `browsers.rs` to turn into
    /// the app-level menu overlay. Aggregated across tabs rather than kept
    /// per tab: a menu only ever opens from a click on the visible page.
    pub context_menu: Option<infiniterm_browser::ContextMenuRequest>,
    /// Remembered so a tab opened later is built the same way the first
    /// one was.
    cef_running: bool,
}

impl BrowserBody {
    pub fn new(
        card_id: &str,
        url: &str,
        world: Size,
        scale: f32,
        cef_running: bool,
    ) -> BrowserBody {
        BrowserBody {
            card_id: card_id.to_string(),
            tabs: vec![Tab::open(url, world, scale, cef_running)],
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
            ui_scale: 1.,
            card_bg: gpui::rgb(0x0e101a).into(),
            text: gpui::rgb(0xb9c4d2).into(),
            font_family: "Menlo".into(),
            dirty: true,
            popups: vec![],
            context_menu: None,
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
        self.tabs
            .push(Tab::open(url, self.world, self.scale, self.cef_running));
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
        let (world, scale, cef) = (self.world, self.scale, self.cef_running);
        self.tabs = urls
            .iter()
            .map(|u| Tab::open(u, world, scale, cef))
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

    fn mods(m: &gpui::Modifiers) -> Mods {
        Mods {
            shift: m.shift,
            control: m.control,
            alt: m.alt,
        }
    }
}

impl Drop for BrowserBody {
    fn drop(&mut self) {
        self.close();
    }
}

/// Which of the strip's affordances a click at `local` lands on, or `None`
/// below the strip (a page click). `local` is `CardBody::mouse_down`'s own
/// space: card pixels, the zoom (`scale`) already undone. `paint` computes
/// the strip's screen-pixel sizes as `PX * ui_scale * scale` because it
/// draws into `Bounds<Pixels>`, already at `scale`; here that last
/// multiplication is skipped because `local` is one step earlier, world
/// units, not screen ones. Get the two out of step and a click lands on the
/// tab next to the one drawn under the cursor.
fn strip_hit(local: Point, ui_scale: f32, tab_count: usize) -> Option<TabClick> {
    let strip_h = TAB_STRIP_HEIGHT_PX * ui_scale as f64;
    if local.y < 0. || local.y >= strip_h {
        return None;
    }
    let tab_w = TAB_STRIP_TAB_WIDTH_PX * ui_scale as f64;
    let index = (local.x / tab_w).floor().max(0.) as usize;
    if index == tab_count {
        return Some(TabClick::New);
    }
    if index > tab_count {
        // Past the "+" button: empty strip, same as clicking dead space.
        return None;
    }
    let x_in_tab = local.x - (index as f64) * tab_w;
    if x_in_tab >= close_band_left_px() * ui_scale as f64 {
        Some(TabClick::Close(index))
    } else {
        Some(TabClick::Switch(index))
    }
}

/// Where a tab's close band starts, counted from the tab's own left edge,
/// in the same pre-`ui_scale`/`scale` PX units as `TAB_STRIP_TAB_WIDTH_PX`.
/// One subtraction, shared by `strip_hit` (the hit test) and `paint` (the
/// glyph's position), so the clickable region and the painted `×` are
/// computed from the same number rather than two numbers that could drift
/// apart the way the click-accuracy bug in this feature already did once.
fn close_band_left_px() -> f64 {
    TAB_STRIP_TAB_WIDTH_PX - TAB_STRIP_CLOSE_WIDTH_PX
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
        // Every tab follows the zoom, not just the visible one: a background
        // tab has to be the right size the moment it becomes active, or the
        // first frame after the switch is a stretched page.
        for tab in &self.tabs {
            if let Some(s) = &tab.surface {
                if (s.shared.borrow().scale - device).abs() > SCALE_CHANGE_EPSILON {
                    s.resize(
                        self.world.w.round() as i32,
                        self.world.h.round() as i32,
                        device,
                    );
                }
            }
        }
        // The page paints at the FULL `bounds`, unchanged from before the
        // strip existed: `local` in `mouse_down`/`mouse_move`/`wheel` is
        // computed by `input.rs::hit()` from the card's WORLD rect and
        // forwarded straight to `Surface::mouse_button`/`mouse_move`/`wheel`
        // with no strip awareness, so if the page were painted into a
        // shorter box here, what the user clicked and what they saw under
        // the cursor would disagree (worst near the top, by the strip's own
        // height). Squeezing the texture into a shorter `page_bounds` was
        // tried and reverted for exactly this: it fixed nothing CEF-side
        // and broke every click's y-coordinate against what was on screen.
        // The strip is painted AFTER the page instead (below), covering its
        // own band of page content the way a real browser's toolbar does,
        // which is what the design doc actually asked for.
        window.paint_quad(fill(bounds, self.card_bg));
        match self.active_tab().and_then(|t| t.texture.clone()) {
            Some(img) => {
                let _ = window.paint_image(bounds, Default::default(), img, 0, false);
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
                            bounds.origin.x + px((STATUS_TEXT_PAD_PX * scale) as f32),
                            bounds.origin.y + px((STATUS_TEXT_PAD_PX * scale) as f32),
                        ),
                        font_size * STATUS_LINE_HEIGHT_RATIO,
                        window,
                        cx,
                    );
                }
            }
        }
        if !focused && self.inactive_dim > 0. {
            window.paint_quad(fill(
                bounds,
                crate::chrome::with_alpha(self.card_bg, self.inactive_dim as f32),
            ));
        }
        // The strip's height in screen pixels, `ui_scale`-aware like every
        // other piece of chrome, clamped to the card's own height so a
        // tiny/zoomed-out card cannot make it taller than the card itself.
        let strip_h =
            px((TAB_STRIP_HEIGHT_PX * self.ui_scale as f64 * scale) as f32).min(bounds.size.height);
        let strip = Bounds::new(bounds.origin, size(bounds.size.width, strip_h));
        // The strip itself, painted last so it sits OVER the page rather
        // than under it: each tab's elided title, the active one visually
        // distinct, the card's number where the corner label used to be.
        // This covers the top `strip_h` of page content until Task 8b gives
        // the strip its own clicks; a bounded, known cost rather than the
        // whole-page click-accuracy regression the `page_bounds` approach had.
        window.paint_quad(fill(strip, self.card_bg));
        let tab_w = px((TAB_STRIP_TAB_WIDTH_PX * self.ui_scale as f64 * scale) as f32);
        let strip_font = px((TAB_STRIP_FONT_PX * self.ui_scale as f64 * scale) as f32);
        // Screen pixels, same convention as `tab_w`/`strip_font` above:
        // `close_left` is the same boundary `strip_hit` compares against
        // (`close_band_left_px`), so the glyph and the clickable region
        // always agree.
        let close_w = px((TAB_STRIP_CLOSE_WIDTH_PX * self.ui_scale as f64 * scale) as f32);
        let close_left = px((close_band_left_px() * self.ui_scale as f64 * scale) as f32);
        if strip_font >= px(crate::chrome::LEGIBLE_FONT_PX as f32) {
            let pad = px((TAB_STRIP_LABEL_PAD_PX * self.ui_scale as f64 * scale) as f32);
            for (i, tab) in self.tabs.iter().enumerate() {
                let tab_bounds = Bounds::new(
                    point(strip.origin.x + tab_w * (i as f32), strip.origin.y),
                    size(tab_w, strip_h),
                );
                if i == self.active {
                    window.paint_quad(fill(tab_bounds, crate::chrome::with_alpha(self.text, 0.08)));
                }
                // Stops short of the close glyph's band, not the tab's own
                // edge, or a long title runs under the `×`.
                let room = f32::from(close_left) - f32::from(pad) * 2.;
                let label = crate::text::elide(tab_label(tab), room, |t| {
                    f32::from(
                        crate::text::shape(
                            window,
                            t,
                            strip_font,
                            &font(self.font_family.clone()),
                            self.text,
                        )
                        .width,
                    )
                });
                let line = crate::text::shape(
                    window,
                    &label,
                    strip_font,
                    &font(self.font_family.clone()),
                    self.text,
                );
                crate::text::paint_in(window, cx, &line, tab_bounds, pad);

                // The close glyph, centred in the close band (`close_left`
                // to the tab's right edge): exactly the region `strip_hit`
                // treats as `TabClick::Close(i)`.
                let close_line = crate::text::shape(
                    window,
                    "×",
                    strip_font,
                    &font(self.font_family.clone()),
                    self.text,
                );
                let close_bounds = Bounds::new(
                    point(
                        tab_bounds.origin.x + close_left + (close_w - close_line.width) / 2.,
                        tab_bounds.origin.y,
                    ),
                    size(close_line.width, strip_h),
                );
                crate::text::paint_in(window, cx, &close_line, close_bounds, px(0.));
            }
            // The "+" for a new tab, in the band right after the last tab:
            // exactly the region `strip_hit` treats as `TabClick::New`
            // (`index == tab_count`).
            let new_tab_bounds = Bounds::new(
                point(
                    strip.origin.x + tab_w * (self.tabs.len() as f32),
                    strip.origin.y,
                ),
                size(tab_w, strip_h),
            );
            let plus_line = crate::text::shape(
                window,
                "+",
                strip_font,
                &font(self.font_family.clone()),
                self.text,
            );
            let plus_bounds = Bounds::new(
                point(
                    new_tab_bounds.origin.x + tab_w / 2. - plus_line.width / 2.,
                    new_tab_bounds.origin.y,
                ),
                size(plus_line.width, strip_h),
            );
            crate::text::paint_in(window, cx, &plus_line, plus_bounds, px(0.));
            // The card's number, right-aligned in the strip: the corner
            // label used to carry it and was removed for exactly this.
            if self.card_number > 0 {
                let number = format!("#{}", self.card_number);
                let line = crate::text::shape(
                    window,
                    &number,
                    strip_font,
                    &font(self.font_family.clone()),
                    self.text,
                );
                let number_bounds = Bounds::new(
                    point(
                        strip.origin.x + strip.size.width - line.width - pad,
                        strip.origin.y,
                    ),
                    size(line.width + pad, strip_h),
                );
                crate::text::paint_in(window, cx, &line, number_bounds, px(0.));
            }
        }
    }

    fn resized(&mut self, world: Size) {
        self.world = world;
        for tab in &self.tabs {
            if let Some(s) = &tab.surface {
                let device = s.shared.borrow().scale;
                s.resize(world.w.round() as i32, world.h.round() as i32, device);
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
        // The strip is chrome, not page content: a click on it is ours to
        // handle before anything about the page (its surface, its focus
        // scrim) is even considered. A non-left click here does nothing
        // rather than falling through to the page below (the page didn't
        // paint under it; the strip is drawn over it, see `paint`).
        if let Some(hit) = strip_hit(local, self.ui_scale, self.tabs.len()) {
            return if button == gpui::MouseButton::Left {
                BodyAction::BrowserTab(hit)
            } else {
                BodyAction::None
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
                local.y as f32,
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
        if let Some(surface) = self.active_surface() {
            surface.mouse_button(
                local.x as f32,
                local.y as f32,
                Self::mods(modifiers),
                b,
                true,
                1,
            );
        }
    }

    fn mouse_move(&mut self, local: Point, modifiers: &gpui::Modifiers) {
        if let Some(surface) = self.active_surface() {
            surface.mouse_move(local.x as f32, local.y as f32, Self::mods(modifiers));
        }
    }

    fn mouse_leave(&mut self) {
        if let Some(surface) = self.active_surface() {
            surface.mouse_leave();
        }
    }

    fn wheel(&mut self, local: Point, dx: f64, dy: f64, modifiers: &gpui::Modifiers) {
        if let Some(surface) = self.active_surface() {
            surface.wheel(
                local.x as f32,
                local.y as f32,
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
        );
        assert_eq!(body.tabs.len(), 1);
        assert!(body.tabs[0].surface.is_none());
        assert!(body.tabs[0].unavailable.is_some());
        assert_eq!(body.active, 0);
    }

    #[test]
    fn a_closed_tab_before_the_active_one_pulls_the_active_index_down() {
        let mut body = BrowserBody::new("card-1", "a", Size { w: 10., h: 10. }, 1., false);
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
mod tab_strip_hit_tests {
    use super::*;
    use crate::body::TabClick;

    #[test]
    fn a_point_below_the_strip_height_is_a_page_click_not_a_strip_one() {
        assert!(strip_hit(Point { x: 5., y: 5. }, 1., 2).is_some());
        assert!(strip_hit(Point { x: 5., y: 50. }, 1., 2).is_none());
    }

    #[test]
    fn a_point_in_a_tabs_main_band_switches_to_it() {
        // Tab width 140px at ui_scale 1: well left of tab 0's and tab 1's
        // own close sub-regions.
        assert_eq!(
            strip_hit(Point { x: 30., y: 5. }, 1., 2),
            Some(TabClick::Switch(0))
        );
        assert_eq!(
            strip_hit(Point { x: 170., y: 5. }, 1., 2),
            Some(TabClick::Switch(1))
        );
    }

    #[test]
    fn a_point_in_a_tabs_close_sub_region_closes_it_instead_of_switching() {
        // Tab 0 spans 0..140; its close band is the rightmost 20px, so 125
        // is inside the tab but past where a switch click stops.
        assert_eq!(
            strip_hit(Point { x: 125., y: 5. }, 1., 2),
            Some(TabClick::Close(0))
        );
    }

    #[test]
    fn a_point_past_the_last_tab_is_the_new_tab_button() {
        // 2 tabs, 140px each: the "+" is the next 140px band, 280..420.
        assert_eq!(
            strip_hit(Point { x: 300., y: 5. }, 1., 2),
            Some(TabClick::New)
        );
    }

    #[test]
    fn a_point_past_the_new_tab_button_hits_nothing() {
        assert_eq!(strip_hit(Point { x: 450., y: 5. }, 1., 2), None);
    }

    #[test]
    fn the_close_bands_left_edge_is_the_number_paint_also_multiplies() {
        // 140px tabs, 20px close band: paint's glyph and strip_hit's
        // boundary both derive from this one subtraction, so a point just
        // inside it still switches and a point just past it still closes,
        // matching the existing boundary tests above.
        assert_eq!(
            close_band_left_px(),
            TAB_STRIP_TAB_WIDTH_PX - TAB_STRIP_CLOSE_WIDTH_PX
        );
        assert_eq!(
            strip_hit(Point { x: 119., y: 5. }, 1., 2),
            Some(TabClick::Switch(0))
        );
        assert_eq!(
            strip_hit(Point { x: 120., y: 5. }, 1., 2),
            Some(TabClick::Close(0))
        );
    }

    #[test]
    fn ui_scale_stretches_the_strips_geometry_like_paint_does() {
        // At 2x ui_scale the strip is twice as tall and each tab twice as
        // wide, mirroring the same multiplication `paint` applies.
        assert!(strip_hit(Point { x: 5., y: 50. }, 1., 2).is_none());
        assert!(strip_hit(Point { x: 5., y: 50. }, 2., 2).is_some());
        assert_eq!(
            strip_hit(Point { x: 100., y: 5. }, 2., 2),
            Some(TabClick::Switch(0))
        );
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
