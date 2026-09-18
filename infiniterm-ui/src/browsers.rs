//! Browser cards and their surfaces: one `BrowserBody` per browser card,
//! opened on the card's url at the card's size, kept in step each frame
//! (new pixels, the page's own zoom from `card.zoom`, a url typed into
//! `browser.navigate`), and what the page did written back: the address
//! it moved to onto the card, popups opened as cards beside it. CEF
//! itself is started once in `main.rs`; `cef_running` says whether it is.
use crate::browser_body::BrowserBody;
use crate::{now_ms, AppView};
use infiniterm_core::grid::Size;
use infiniterm_core::ift::url_plan;
use infiniterm_core::model::{BrowserAction, FindRequest};
use infiniterm_core::saved_layout::CardKind;

impl AppView {
    pub fn browser_for(&mut self, id: &str) -> Option<&mut BrowserBody> {
        self.bodies
            .get_mut(id)
            .and_then(|b| b.as_any_mut().downcast_mut::<BrowserBody>())
    }

    pub fn reconcile_browsers(&mut self) {
        let cef = self.cef_running;
        let scale = self.scale_factor;
        let card_bg = self.chrome.card_bg;
        let text = self.chrome.text;
        let family = crate::terminals::family_of(&self.model.config.terminal.font_family);
        let inactive_dim = self.model.config.ui.inactive_dim;
        let cards: Vec<_> = self
            .model
            .cards
            .iter()
            .filter(|c| c.kind == CardKind::Browser)
            .cloned()
            .collect();
        let mut opens: Vec<(String, String)> = vec![];
        let mut moved: Vec<(String, String)> = vec![];
        let mut titles: Vec<(String, String)> = vec![];
        for card in cards {
            let world = Size {
                w: card.rect.w,
                h: card.rect.h,
            };
            let url = card.url.clone().unwrap_or_else(|| "about:blank".into());
            if self.browser_for(&card.id).is_none() {
                let body = BrowserBody::new(&card.id, &url, world, scale, cef);
                self.bodies.insert(card.id.clone(), Box::new(body));
            }
            let Some(body) = self.browser_for(&card.id) else {
                continue;
            };
            body.card_bg = card_bg;
            body.text = text;
            body.font_family = family.clone();
            body.inactive_dim = inactive_dim;
            body.zoom = card.zoom;
            // `browser.navigate` changed the card's url: the page follows.
            if body.url != url && url != "about:blank" {
                body.navigate(&url);
            }
            if let Some(new_url) = body.sync() {
                moved.push((card.id.clone(), new_url));
            }
            if let Some(title) = body.take_title() {
                titles.push((body.url.clone(), title));
            }
            for popup in std::mem::take(&mut body.popups) {
                opens.push((card.id.clone(), popup));
            }
        }
        for (id, url) in moved {
            // Every navigation passes through here, which is the only place
            // the app sees one: history is recorded here rather than in the
            // model, which never learns where a page went by itself.
            self.model.record_visit(&url, None, now_ms());
            if let Some(c) = self.model.card_mut(&id) {
                c.url = Some(url);
                self.model.dirty_layout = true;
                // The label and the status bar were drawn from the old
                // address before this ran, and a page that has finished
                // loading asks for no further frames: without this the card
                // says where it used to be until something else moves.
                self.redraw = true;
            }
        }
        for (url, title) in titles {
            self.model.history.set_title(&url, &title);
        }
        for (id, url) in opens {
            let plan = url_plan(&url, "");
            self.model.open_in_card(plan, Some(&id));
        }
    }
}

impl AppView {
    /// Back and forward. The page owns its history, so this is the whole
    /// implementation: the model only says which card and which direction.
    pub fn browser_effect(&mut self, card_id: &str, action: BrowserAction) {
        let Some(body) = self
            .bodies
            .get_mut(card_id)
            .and_then(|b| b.as_any_mut().downcast_mut::<BrowserBody>())
        else {
            return;
        };
        let Some(surface) = &body.surface else { return };
        match action {
            BrowserAction::Back => surface.back(),
            BrowserAction::Forward => surface.forward(),
            BrowserAction::Reload => surface.reload(),
        }
    }

    /// Find in page. A None request ends the search and clears the
    /// highlights, which is what closing the bar must always do.
    pub fn find_effect(&mut self, card_id: &str, request: Option<FindRequest>) {
        let Some(body) = self
            .bodies
            .get_mut(card_id)
            .and_then(|b| b.as_any_mut().downcast_mut::<BrowserBody>())
        else {
            return;
        };
        let Some(surface) = &body.surface else { return };
        match request {
            Some(r) => surface.find(&r.text, r.forward, r.next),
            None => surface.stop_find(),
        }
    }

    /// The OS cursor follows whichever browser card the pointer is over
    /// (a pointer over a link, an I-beam over an input), the way it would
    /// in a real browser tab. Anything else hovered, or nothing, keeps the
    /// arrow: nothing else in the app sets a cursor style, so this is
    /// decided fresh every frame rather than left to whatever gpui reset
    /// to on its own.
    pub fn apply_hover_cursor(&mut self, window: &mut gpui::Window) {
        let style = self
            .hover_body
            .clone()
            .and_then(|id| self.browser_for(&id))
            .map(|b| b.cursor_style())
            .unwrap_or(gpui::CursorStyle::Arrow);
        window.set_window_cursor_style(style);
    }

    /// Chromium reports a find's progress several times per search; the
    /// model takes whatever arrived by this frame.
    pub fn drain_find(&mut self) {
        let Some(card_id) = self.model.find.card_id.clone() else {
            return;
        };
        let result = self
            .bodies
            .get_mut(&card_id)
            .and_then(|b| b.as_any_mut().downcast_mut::<BrowserBody>())
            .and_then(|b| b.surface.as_ref())
            .map(|s| s.find_result());
        if let Some((matches, active)) = result {
            let before = (self.model.find.matches, self.model.find.active);
            self.model.find_result(&card_id, matches, active);
            self.redraw |= (self.model.find.matches, self.model.find.active) != before;
        }
    }
}
