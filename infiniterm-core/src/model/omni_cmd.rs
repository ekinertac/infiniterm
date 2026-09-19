//! The omnibox's state and the six moves it answers: open, type, step, tab,
//! unscope, enter. The ranking is `omni::rank`; nothing here decides what a
//! result IS, only what happens to the one that was chosen.
//!
//! Two starting states, and the difference is the whole reason the field is
//! ever prefilled: on a browser card Cmd+L edits THAT card's address and
//! Enter navigates it in place, the way it does in a browser. Anywhere else
//! the field is empty and Enter makes a card.
//!
//! Related: infiniterm-core/src/omni/, infiniterm-ui/src/omnibox.rs,
//! model/palette_state.rs (where the overlay check lives).
use super::{Effect, Model};
use crate::omni::address::search_url;
use crate::omni::rank::{rank, OmniResponse};
use crate::omni::{OmniAction, OmniCard, OmniCtx, OmniResult};
use crate::saved_layout::CardKind;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OmniState {
    pub open: bool,
    pub query: String,
    pub index: usize,
    /// The engine keyword a Tab scoped to.
    pub scope: Option<String>,
    /// Bumped on every keystroke. A suggestion response carrying an older id
    /// is dropped, which is the whole concurrency story.
    pub query_id: u64,
    pub suggestions: Vec<String>,
    /// The browser card being edited, when Cmd+L was pressed on one.
    pub target: Option<String>,
    /// This omnibox is making a card for the phantom slot Enter opened it
    /// from, not one beside whatever is active: `open_omnibox_for_new_card`
    /// with `phantom: true`. Meaningless while `target` is `Some`.
    pub phantom: bool,
}

impl Model {
    pub fn open_omnibox(&mut self) {
        let browser = self
            .focused()
            .filter(|c| c.kind == CardKind::Browser)
            .cloned();
        self.omni = OmniState {
            open: true,
            query: browser
                .as_ref()
                .and_then(|c| c.url.clone())
                .unwrap_or_default(),
            target: browser.map(|c| c.id),
            ..OmniState::default()
        };
    }

    /// Cmd+Shift+T's placement menu and a phantom's kind picker both make a
    /// NEW card rather than edit whatever is focused, so `target` is always
    /// `None` here even when a browser card happens to be active — unlike
    /// `open_omnibox`, which edits it. `phantom` says where Enter lands the
    /// result: `fill_phantom` for the slot Enter was pressed on, else
    /// beside the active card, `open_omnibox`'s own rule.
    pub fn open_omnibox_for_new_card(&mut self, phantom: bool) {
        self.omni = OmniState {
            open: true,
            phantom,
            ..OmniState::default()
        };
    }

    pub fn close_omnibox(&mut self) {
        self.omni = OmniState::default();
    }

    pub fn omni_type(&mut self, text: &str) {
        self.omni.query = text.to_string();
        self.omni.index = 0;
        self.omni.query_id += 1;
        // The old query's suggestions describe text nobody can see any more.
        self.omni.suggestions.clear();
        if self.config.browser.suggestions && !self.omni.query.trim().is_empty() {
            self.effects.push(Effect::FetchSuggestions {
                query_id: self.omni.query_id,
                query: self.omni.query.clone(),
            });
        }
    }

    pub fn omni_step(&mut self, delta: i32) {
        let n = self.omni_flat().len();
        if n == 0 {
            return;
        }
        let next = self.omni.index as i32 + delta;
        self.omni.index = next.rem_euclid(n as i32) as usize;
    }

    pub fn omni_tab(&mut self) {
        if let Some((keyword, _)) = self.omni_response().offer {
            self.omni.scope = Some(keyword);
            // The field clears for the query itself: what was typed was the
            // site's name, and it is now the chip instead.
            self.omni.query.clear();
            self.omni.index = 0;
            self.omni.suggestions.clear();
        }
    }

    pub fn omni_unscope(&mut self) {
        self.omni.scope = None;
        self.omni.index = 0;
    }

    /// The flat list the selection indexes into, in the order it is drawn.
    pub fn omni_flat(&self) -> Vec<OmniResult> {
        self.omni_response()
            .sections
            .into_iter()
            .flat_map(|s| s.results)
            .collect()
    }

    pub fn omni_response(&self) -> OmniResponse {
        // This workspace only, like everything else that reads geometry.
        let cards: Vec<OmniCard> = self
            .cards
            .iter()
            .filter(|c| {
                c.kind == CardKind::Browser
                    && Some(c.workspace_id.as_str()) == self.active_workspace.as_deref()
                    // The card being edited is not a result: it is where
                    // Enter is already going.
                    && Some(&c.id) != self.omni.target.as_ref()
            })
            .map(|c| OmniCard {
                id: c.id.clone(),
                title: c.title.clone(),
                url: c.url.clone().unwrap_or_default(),
            })
            .collect();
        let engines = self.config.engines();
        let ctx = OmniCtx {
            q: &self.omni.query,
            scope: self.omni.scope.as_deref(),
            history: self.history.list(),
            cards: &cards,
            engines: &engines,
            template: &self.config.browser.search_engine,
        };
        rank(&ctx, &self.omni.suggestions)
    }

    pub fn omni_suggestions(&mut self, query_id: u64, items: Vec<String>) {
        if query_id != self.omni.query_id {
            return;
        }
        self.omni.suggestions = items;
    }

    pub fn omni_enter(&mut self) {
        let chosen = self.omni_flat().into_iter().nth(self.omni.index);
        let target = self.omni.target.clone();
        let phantom = self.omni.phantom;
        self.close_omnibox();
        let Some(result) = chosen else { return };
        match result.action {
            OmniAction::FocusCard(id) => {
                self.set_focus(Some(&id));
                self.reveal_focused();
            }
            OmniAction::Navigate(url) => self.omni_go(url, target, phantom),
            OmniAction::Search(query) => {
                let url = search_url(&self.config.browser.search_engine, &query);
                self.omni_go(url, target, phantom);
            }
        }
    }

    /// `Alt+Enter`: what was typed opens as a NEW tab on the target card
    /// instead of navigating it in place, matching Chrome's own binding.
    /// Outside a browser card (no `omni.target`) there is no tab strip to
    /// add to, so this is identical to `omni_enter`. A `FocusCard` result
    /// has no url to open as a tab either way, so it just focuses, same as
    /// `Enter`.
    pub fn omni_enter_new_tab(&mut self) {
        let Some(target) = self.omni.target.clone() else {
            self.omni_enter();
            return;
        };
        let chosen = self.omni_flat().into_iter().nth(self.omni.index);
        self.close_omnibox();
        let Some(result) = chosen else { return };
        let url = match result.action {
            OmniAction::FocusCard(id) => {
                self.set_focus(Some(&id));
                self.reveal_focused();
                return;
            }
            OmniAction::Navigate(url) => url,
            OmniAction::Search(query) => search_url(&self.config.browser.search_engine, &query),
        };
        self.browser_tab_open(&target, Some(&url));
    }

    /// Navigates the card Cmd+L was opened on, fills the phantom slot Enter
    /// was pressed on, or makes a card beside the active one: in that
    /// order, the same precedence `open_omnibox`/`open_omnibox_for_new_card`
    /// set up when this omnibox opened.
    fn omni_go(&mut self, url: String, target: Option<String>, phantom: bool) {
        if let Some(card) = target.and_then(|id| self.card_mut(&id)) {
            card.url = Some(url);
            self.dirty_layout = true;
            return;
        }
        if phantom {
            self.fill_phantom(CardKind::Browser, Some(url));
            return;
        }
        self.new_beside_active(CardKind::Browser, None, Some(url));
    }

    /// A browser card went somewhere. Called from the ui, which is the only
    /// place a navigation is observed.
    pub fn record_visit(&mut self, url: &str, title: Option<&str>, now: f64) {
        self.history.record(url, now);
        if let Some(t) = title {
            self.history.set_title(url, t);
        }
    }
}
