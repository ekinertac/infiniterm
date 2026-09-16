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
