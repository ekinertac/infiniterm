//! The omnibox: Cmd+L, an address or a search, ranked against what is open
//! and where you have been. Every decision here is a pure function; the
//! model holds state only, and `infiniterm-ui/src/omnibox.rs` draws it.
//!
//! Ported from the omnibox in ~/Code/glass (Electron), which settled the
//! ranking and the tab-to-search flow. See
//! docs/superpowers/specs/2026-09-16-omnibox-design.md.
//!
//! NOT the command palette: that matches a fixed list of commands by fuzzy
//! score, this takes free text and mixes sources.
pub mod address;
pub mod engines;
pub mod history;
pub mod providers;

use crate::omni::engines::Engine;
use crate::omni::history::Visit;

/// What choosing a result does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OmniAction {
    Navigate(String),
    /// The raw query. The template turns it into a URL at the edge, so a
    /// changed engine applies to a result already on screen.
    Search(String),
    /// A browser card already on the canvas: focus it, do not navigate.
    /// This is glass's "switch to tab", and here a tab is a card.
    FocusCard(String),
}

/// Which icon and which colour, and nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OmniKind {
    Address,
    Search,
    History,
    Card,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmniResult {
    /// Stable within one response, so the selection survives the network
    /// results landing under it.
    pub id: String,
    pub kind: OmniKind,
    pub title: String,
    pub subtitle: Option<String>,
    pub action: OmniAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmniSection {
    /// A String rather than a &'static str: a scoped section is named after
    /// the engine ("search GitHub"), which comes from settings.
    pub heading: String,
    pub results: Vec<OmniResult>,
}

/// A browser card as the omnibox sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmniCard {
    pub id: String,
    pub title: String,
    pub url: String,
}

/// Everything a provider may read, and nothing else. Borrowed, because the
/// model owns all of it: a provider that wants the Model has been written in
/// the wrong crate.
pub struct OmniCtx<'a> {
    pub q: &'a str,
    /// The engine keyword a Tab scoped the box to, if any.
    pub scope: Option<&'a str>,
    pub history: &'a [Visit],
    pub cards: &'a [OmniCard],
    pub engines: &'a [Engine],
    /// The search template, `%s` for the query.
    pub template: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;

    // The context is what every provider reads and nothing more.
    #[test]
    fn a_context_is_borrowed_and_carries_no_model() {
        let history = vec![];
        let cards = vec![OmniCard {
            id: "c1".into(),
            title: "Hacker News".into(),
            url: "https://news.ycombinator.com".into(),
        }];
        let engines = crate::omni::engines::default_engines();
        let ctx = OmniCtx {
            q: "news",
            scope: None,
            history: &history,
            cards: &cards,
            engines: &engines,
            template: crate::omni::address::SEARCH_TEMPLATE,
        };
        assert_eq!(ctx.cards.len(), 1);
        assert!(ctx.scope.is_none());
    }
}
