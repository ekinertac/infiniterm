//! What a browser card is in a build without CEF.
//!
//! `infiniterm-ui`'s `browser` feature is on by default, so macOS is the tree
//! as it was: `browsers.rs` reconciles real `BrowserBody` surfaces and
//! `main.rs` starts CEF. With the feature off, `browsers.rs` and
//! `browser_body.rs` are not compiled and THIS FILE IS COMPILED AS `browsers`
//! instead (`#[path]` in main.rs). That is why it carries the same names for
//! the same things: every call site in `input.rs`, `overlays.rs`, `paint.rs`
//! and `runtime.rs` stays exactly as it is, and the difference between the
//! two builds lives in one place rather than in a cfg at each of them.
//!
//! So this file must answer everything the rest of the app asks
//! `browsers.rs`: `reconcile_browsers`, `browser_for`, `browser_effect`,
//! `find_effect`, `apply_hover_cursor`, `drain_find`, `render_context_menu`
//! and the `CardContextMenu` type. If a new one appears over there, the
//! build without the feature stops compiling until it appears here, which is
//! the point.
//!
//! Two reasons the feature exists. Windows has no CEF build yet (phase 5 of
//! docs/windows-handoff.md), and the Lite/Pro split planned in
//! ekinertac/notes#50 wants exactly this shape: a build-time split with a
//! stub card, not a runtime key.
//!
//! A browser card is still a card here. It opens, it has a place on the
//! canvas, it saves and restores with its url, and it says plainly what it
//! is not. The day CEF is in the build it is a browser again.
//!
//! Related: `body.rs` (the trait), `browsers.rs` and `browser_body.rs` (the
//! real thing, and the `ift-browser` session's files), `paint.rs` (the
//! caller of `reconcile_browsers`).

use crate::body::{BodyAction, CardBody};
use crate::text;
use crate::AppView;
use gpui::{div, fill, font, point, px, App, Bounds, Hsla, Pixels, Window};
use infiniterm_core::model::{BrowserAction, FindRequest};
use infiniterm_core::saved_layout::CardKind;

/// Said plainly rather than hinting at a setting to turn on: there is none.
const LINE: &str = "browser cards are not in this build";

/// The real one carries where the right-click happened and what was under
/// it. Nothing here ever builds one: a page has to exist to right-click on.
/// It stays because `AppView::context_menu` is typed by it and the field is
/// read in `input.rs` and `overlays.rs` without either knowing about CEF.
pub struct CardContextMenu {
    pub card_id: String,
    pub x: f64,
    pub y: f64,
    pub link_url: Option<String>,
    pub editable: bool,
    pub has_selection: bool,
}

pub struct BrowserStubBody {
    pub card_bg: Hsla,
    pub text: Hsla,
    pub font_family: String,
    /// The url the card holds, drawn under the line so a restored card still
    /// shows what it was pointing at.
    pub url: String,
    /// The double-Escape that unlocks a locked card, which `input.rs` steps
    /// on whichever body the card has. A stub card never locks, so this is
    /// only here to keep that one call site from needing to know.
    pub last_escape_ms: Option<f64>,
}

impl BrowserStubBody {
    /// The real body hands focus to the page. There is no page.
    pub fn set_focus(&mut self, _focused: bool) {}
}

impl CardBody for BrowserStubBody {
    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        _scale: f64,
        _focused: bool,
        _now: f64,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.paint_quad(fill(bounds, self.card_bg));
        // Screen pixels, like every other affordance, so the message stays
        // readable as the canvas zooms out and does not dominate zoomed in.
        let size = px(13.0);
        let face = font(self.font_family.clone());
        let room = f32::from(bounds.size.width) - 16.;
        let mut y = bounds.origin.y + bounds.size.height / 2. - size * 1.5;
        for line in [LINE, self.url.as_str()] {
            if line.is_empty() {
                continue;
            }
            let shown = text::elide(line, room, |s| {
                f32::from(text::shape(window, s, size, &face, self.text).width)
            });
            let run = text::shape(window, &shown, size, &face, self.text);
            let x = bounds.origin.x + (bounds.size.width - run.width) / 2.;
            let _ = run.paint(point(x, y), size * 1.4, window, cx);
            y += size * 1.6;
        }
    }

    fn key(&mut self, _k: &gpui::Keystroke, _now: f64, _cx: &mut App) -> BodyAction {
        // Nothing here wants a key, and saying so is what lets a dead key
        // compose rather than vanish (see `BodyAction::Ignored`).
        BodyAction::Ignored
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl AppView {
    pub fn browser_for(&mut self, id: &str) -> Option<&mut BrowserStubBody> {
        self.bodies
            .get_mut(id)
            .and_then(|b| b.as_any_mut().downcast_mut::<BrowserStubBody>())
    }

    /// Every browser card gets a stub body, kept in the chrome's colours the
    /// way the real one is. Called once a frame from `paint.rs`.
    pub fn reconcile_browsers(&mut self) {
        let card_bg = self.chrome.card_bg;
        let text = self.chrome.text;
        let family = crate::terminals::family_of(&self.model.config.terminal.font_family);
        let browsers: Vec<(String, String)> = self
            .model
            .cards
            .iter()
            .filter(|c| c.kind == CardKind::Browser)
            .map(|c| {
                (
                    c.id.clone(),
                    c.url.clone().unwrap_or_else(|| "about:blank".into()),
                )
            })
            .collect();
        for (id, url) in browsers {
            if self.browser_for(&id).is_none() {
                self.bodies.insert(
                    id.clone(),
                    Box::new(BrowserStubBody {
                        card_bg,
                        text,
                        font_family: family.clone(),
                        url: url.clone(),
                        last_escape_ms: None,
                    }),
                );
            }
            if let Some(body) = self.browser_for(&id) {
                body.card_bg = card_bg;
                body.text = text;
                body.font_family = family.clone();
                body.url = url;
            }
        }
    }

    /// Back, forward and reload belong to a page's own history. No page, so
    /// the command runs and nothing happens, which is what the palette row
    /// doing nothing on a stub card should look like.
    pub fn browser_effect(&mut self, _card_id: &str, _action: BrowserAction) {}

    /// Find in page, likewise: the editor's find is its own (`editors.rs`),
    /// and this effect only ever reaches a browser card.
    pub fn find_effect(&mut self, _card_id: &str, _request: Option<FindRequest>) {}

    /// Nothing in this build sets a cursor style, so every frame puts it
    /// back to the arrow rather than leaving whatever gpui last reset to.
    pub fn apply_hover_cursor(&mut self, window: &mut gpui::Window) {
        window.set_window_cursor_style(gpui::CursorStyle::Arrow);
    }

    /// No Chromium, no find results to drain.
    pub fn drain_find(&mut self) {}

    /// Never reached: `context_menu` is only ever set from a page's own
    /// right-click, and there are no pages. It exists so `overlays.rs` can
    /// go on asking for it without a cfg.
    pub fn render_context_menu(&self, _cx: &mut gpui::Context<Self>) -> gpui::Div {
        div()
    }
}
