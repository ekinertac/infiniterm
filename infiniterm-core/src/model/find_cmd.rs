//! Find in page: the bar's state and the four moves it answers. Chromium
//! does the searching and the highlighting; this only says what to look for
//! and which way to step.
//!
//! NOT an overlay in the sense the palette and the omnibox are: it does not
//! dim the canvas and does not stop the wheel, because the whole point is
//! reading the page while it is open. It does take the keys.
//!
//! One bar for the app, not one per card: it is tied to the card it was
//! opened on, and switching cards closes it, the way a browser closes its
//! find bar when you change tab.
//!
//! Related: model/cards_cmd.rs (the command), infiniterm-ui/src/omnibox.rs
//! (drawn beside the address bar), infiniterm-browser/src/surface.rs.
use super::{Effect, FindRequest, Model};
use crate::saved_layout::CardKind;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FindState {
    pub open: bool,
    pub query: String,
    /// The browser card being searched. The bar closes when focus leaves it.
    pub card_id: Option<String>,
    /// What Chromium last reported: matches, and which one is current.
    pub matches: i32,
    pub active: i32,
}

impl Model {
    pub fn open_find(&mut self) {
        self.with_active_card(|m, id| {
            // A page is searched by Chromium, a terminal's scrollback by
            // alacritty (`Grid::find`); the bar is the same.
            if !matches!(
                m.card(&id).map(|c| c.kind),
                Some(CardKind::Browser | CardKind::Terminal)
            ) {
                m.notify("nothing to find in here");
                return;
            }
            // Reopening on the same card keeps the query, so Cmd+F twice is
            // not a way to lose what you were looking for.
            let same = m.find.card_id.as_deref() == Some(id.as_str());
            m.find = FindState {
                open: true,
                query: if same {
                    std::mem::take(&mut m.find.query)
                } else {
                    String::new()
                },
                card_id: Some(id),
                matches: 0,
                active: 0,
            };
        });
    }

    /// Typing runs a NEW search on every keystroke, which is what makes the
    /// count follow what you type.
    pub fn find_type(&mut self, text: &str) {
        self.find.query = text.to_string();
        self.find.matches = 0;
        self.find.active = 0;
        let Some(card_id) = self.find.card_id.clone() else {
            return;
        };
        if self.find.query.is_empty() {
            self.effects.push(Effect::Find {
                card_id,
                request: None,
            });
            return;
        }
        let request = FindRequest {
            text: self.find.query.clone(),
            forward: true,
            next: false,
        };
        self.effects.push(Effect::Find {
            card_id,
            request: Some(request),
        });
    }

    pub fn find_step(&mut self, forward: bool) {
        let (Some(card_id), false) = (self.find.card_id.clone(), self.find.query.is_empty()) else {
            return;
        };
        let request = FindRequest {
            text: self.find.query.clone(),
            forward,
            next: true,
        };
        self.effects.push(Effect::Find {
            card_id,
            request: Some(request),
        });
    }

    /// The count Chromium reported, which arrives several times per search.
    pub fn find_result(&mut self, card_id: &str, matches: i32, active: i32) {
        if self.find.card_id.as_deref() != Some(card_id) {
            return;
        }
        self.find.matches = matches;
        self.find.active = active;
    }

    /// Cmd+F on whatever card you are on: its own search.
    pub fn find_in_card(&mut self) {
        let Some(kind) = self.focused().map(|c| c.kind) else {
            return;
        };
        match kind {
            // The editor's search is its own bar, in the body; opening it
            // also locks the editor so the query can be typed.
            CardKind::Editor => self.with_active_card(|m, id| {
                m.effects.push(Effect::Editor {
                    card_id: id,
                    action: super::EditorAction::Find,
                })
            }),
            _ => self.open_find(),
        }
    }

    /// Cmd+E: the selection becomes the query. Only the body knows its
    /// selection, so the ui answers (`Effect::FindSelection`).
    pub fn find_selection(&mut self) {
        self.with_active_card(|m, id| {
            if m.card(&id).map(|c| c.kind) == Some(CardKind::Terminal) {
                m.effects.push(Effect::FindSelection(id));
            } else {
                m.notify("use selection for find works in terminals");
            }
        });
    }

    /// The ui's answer to `FindSelection`: open the bar holding `text`.
    pub fn find_with(&mut self, text: &str) {
        self.open_find();
        if self.find.open {
            self.find_type(text);
        }
    }

    pub fn close_find(&mut self) {
        if let Some(card_id) = self.find.card_id.take() {
            // Highlights that outlive the bar are litter nobody can clear.
            self.effects.push(Effect::Find {
                card_id,
                request: None,
            });
        }
        self.find = FindState::default();
    }
}
