//! The browser card: a CEF surface painted as a texture inside the card.
//! Port of `BrowserCard.svelte` on `infiniterm-browser`, which is the
//! spike's `browser.rs` made permanent.
//!
//! The page lays out at the card's size in world units and is drawn at
//! whatever the zoom makes of it, so labels, rings and the palette paint
//! over it like over any card and it scales with the canvas. Every frame
//! CEF paints is copied into a gpui image when it changed; an idle page
//! costs nothing. Keys: the app owns Cmd except the edit chords a page
//! needs (copy, paste, cut, select all, undo) and the page-zoom chords
//! the model rebinds (`browser_keys.rs`); everything else goes to the
//! page once it has focus. Focus is deliberate: a click on an UNFOCUSED
//! card only focuses it (the scrim takes it), the next reaches the page;
//! arrow-focusing a card never gives the page focus.
//!
//! Without CEF (the bare binary outside a bundle) the card says so and
//! stays a rectangle.
use crate::body::{BodyAction, CardBody};
use gpui::{
    fill, font, point, px, size, App, Bounds, Hsla, Keystroke, Pixels, RenderImage, Window,
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

pub struct BrowserBody {
    pub card_id: String,
    pub url: String,
    pub surface: Option<Surface>,
    /// Why there is no surface, for the card to say.
    pub unavailable: Option<String>,
    texture: Option<Arc<RenderImage>>,
    /// The page's own zoom the card carries (`card.zoom`), applied once.
    pub zoom: Option<f64>,
    applied_zoom: Option<f64>,
    world: Size,
    scale: f32,
    painted_focused: bool,
    /// The page has keyboard focus: a click reached it since the card
    /// was focused.
    pub page_focused: bool,
    left_down: bool,
    pub inactive_dim: f64,
    pub card_bg: Hsla,
    pub text: Hsla,
    pub font_family: String,
    dirty: bool,
    pub popups: Vec<String>,
    /// The page title as last reported, so a change can be told from the
    /// same title arriving every frame. Not the card's title: that is a
    /// name somebody CHOSE, and a page must never overwrite it.
    title: Option<String>,
}

impl BrowserBody {
    pub fn new(
        card_id: &str,
        url: &str,
        world: Size,
        scale: f32,
        cef_running: bool,
    ) -> BrowserBody {
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
        BrowserBody {
            card_id: card_id.to_string(),
            url: url.to_string(),
            surface,
            unavailable,
            texture: None,
            zoom: None,
            applied_zoom: None,
            world,
            scale,
            painted_focused: false,
            page_focused: false,
            left_down: false,
            inactive_dim: crate::chrome::INACTIVE_DIM_DEFAULT,
            card_bg: gpui::rgb(0x0e101a).into(),
            text: gpui::rgb(0xb9c4d2).into(),
            font_family: "Menlo".into(),
            dirty: true,
            popups: vec![],
            title: None,
        }
    }

    /// Each frame: new pixels, popups, the address the page moved to.
    /// Returns the current url when the page changed it.
    pub fn sync(&mut self) -> Option<String> {
        let Some(surface) = &self.surface else {
            return None;
        };
        if let Some(frame) = surface.take_frame() {
            if let Some(img) = RgbaImage::from_raw(frame.width, frame.height, frame.bgra.clone()) {
                // CEF hands BGRA and gpui stores BGRA: the bytes go in as they are.
                self.texture = Some(Arc::new(RenderImage::new(SmallVec::from_elem(
                    ImageFrame::new(img),
                    1,
                ))));
                self.dirty = true;
            }
        }
        self.popups.extend(surface.take_popups());
        if self.zoom != self.applied_zoom {
            surface.set_zoom(self.zoom.unwrap_or(1.));
            self.applied_zoom = self.zoom;
        }
        let url = surface.url()?;
        if url != self.url && url != "about:blank" {
            self.url = url.clone();
            return Some(url);
        }
        None
    }

    /// The page title, but only when it has changed since the last call.
    /// The omnibox's history wants it; nothing else does.
    pub fn take_title(&mut self) -> Option<String> {
        let t = self.surface.as_ref()?.title()?;
        if Some(&t) == self.title.as_ref() {
            return None;
        }
        self.title = Some(t.clone());
        Some(t)
    }

    pub fn navigate(&mut self, url: &str) {
        self.url = url.to_string();
        if let Some(s) = &self.surface {
            s.navigate(url);
        }
    }

    pub fn set_focus(&mut self, on: bool) {
        if self.page_focused != on {
            self.page_focused = on;
            if let Some(s) = &self.surface {
                s.focus(on);
            }
        }
    }

    pub fn close(&mut self) {
        if let Some(s) = self.surface.take() {
            s.close();
        }
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
        if let Some(s) = &self.surface {
            if (s.shared.borrow().scale - device).abs() > SCALE_CHANGE_EPSILON {
                s.resize(
                    self.world.w.round() as i32,
                    self.world.h.round() as i32,
                    device,
                );
            }
        }
        window.paint_quad(fill(bounds, self.card_bg));
        match &self.texture {
            Some(img) => {
                let _ = window.paint_image(bounds, Default::default(), img.clone(), 0, false);
            }
            None => {
                let font_size = px((STATUS_FONT_PX * scale) as f32);
                if font_size >= px(crate::chrome::LEGIBLE_FONT_PX as f32) {
                    let text = self
                        .unavailable
                        .clone()
                        .unwrap_or_else(|| format!("loading {}", self.url));
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
        let _ = size(px(0.), px(0.));
    }

    fn resized(&mut self, world: Size) {
        self.world = world;
        if let Some(s) = &self.surface {
            let device = s.shared.borrow().scale;
            s.resize(world.w.round() as i32, world.h.round() as i32, device);
        }
        self.dirty = true;
    }

    fn key(&mut self, k: &Keystroke, _now: f64, _cx: &mut App) -> BodyAction {
        let Some(surface) = &self.surface else {
            return BodyAction::None;
        };
        if k.modifiers.platform {
            surface.edit_chord(&k.key);
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
        if self.surface.is_none() {
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
        if let Some(surface) = &self.surface {
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
        let Some(surface) = &self.surface else { return };
        let b = match button {
            gpui::MouseButton::Left => Button::Left,
            gpui::MouseButton::Middle => Button::Middle,
            gpui::MouseButton::Right => Button::Right,
            _ => return,
        };
        if b == Button::Left {
            self.left_down = false;
        }
        surface.mouse_button(
            local.x as f32,
            local.y as f32,
            Self::mods(modifiers),
            b,
            true,
            1,
        );
    }

    fn mouse_move(&mut self, local: Point, modifiers: &gpui::Modifiers) {
        if let Some(surface) = &self.surface {
            surface.mouse_move(
                local.x as f32,
                local.y as f32,
                Self::mods(modifiers),
                self.left_down,
            );
        }
    }

    fn wheel(&mut self, local: Point, dx: f64, dy: f64, modifiers: &gpui::Modifiers) {
        if let Some(surface) = &self.surface {
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
        self.dirty || self.surface.as_ref().is_some_and(|s| s.has_new_frame())
    }

    fn captures_drag(&self) -> bool {
        self.left_down
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
