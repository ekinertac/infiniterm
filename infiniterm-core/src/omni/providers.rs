//! The omnibox's sources. Each is a pure function over `OmniCtx` returning
//! typed results; `omni::rank` runs them in order and assembles sections.
//! A new source is a new function here and one line there.
//!
//! Related: omni/mod.rs (the context and the result), omni/rank.rs,
//! omni/history.rs.
use crate::omni::address::{parse_address, Address};
use crate::omni::{OmniAction, OmniCtx, OmniKind, OmniResult};

/// More history than this and the list stops being a list.
pub const HISTORY_LIMIT: usize = 8;
/// Open cards are few, and showing all of them is the point of the section.
pub const CARD_LIMIT: usize = 6;

/// Scheme and leading www. stripped, so a URL reads the way an address bar
/// shows it.
pub fn display_url(url: &str) -> String {
    let no_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    no_scheme
        .strip_prefix("www.")
        .unwrap_or(no_scheme)
        .to_string()
}

/// The top row: what you typed, as an address if it parses as one.
pub fn address_result(ctx: &OmniCtx) -> Vec<OmniResult> {
    match parse_address(ctx.q) {
        Some(Address::Navigate(url)) => vec![OmniResult {
            id: format!("address:{url}"),
            kind: OmniKind::Address,
            title: display_url(&url),
            subtitle: Some("Open".into()),
            action: OmniAction::Navigate(url),
        }],
        Some(Address::Search(query)) => vec![OmniResult {
            id: format!("search:{query}"),
            kind: OmniKind::Search,
            title: query.clone(),
            subtitle: Some("Search".into()),
            action: OmniAction::Search(query),
        }],
        None => vec![],
    }
}

/// Matches the title and the URL WITHOUT its query string: a buried `?q=`
/// parameter would otherwise make a sign-in redirect match anything anybody
/// ever searched for, which is the single biggest source of junk results.
fn haystack(title: &str, url: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    format!("{title} {path}").to_lowercase()
}

pub fn history_results(ctx: &OmniCtx) -> Vec<OmniResult> {
    let needle = ctx.q.trim().to_lowercase();
    ctx.history
        .iter()
        .filter(|v| needle.is_empty() || haystack(&v.title, &v.url).contains(&needle))
        .take(HISTORY_LIMIT)
        .map(|v| OmniResult {
            id: format!("history:{}", v.url),
            kind: OmniKind::History,
            // The title until it loads, then the URL moves to the subtitle.
            title: if v.title.is_empty() {
                v.url.clone()
            } else {
                v.title.clone()
            },
            subtitle: (!v.title.is_empty()).then(|| v.url.clone()),
            action: OmniAction::Navigate(v.url.clone()),
        })
        .collect()
}

pub fn card_results(ctx: &OmniCtx) -> Vec<OmniResult> {
    let needle = ctx.q.trim().to_lowercase();
    ctx.cards
        .iter()
        .filter(|c| needle.is_empty() || haystack(&c.title, &c.url).contains(&needle))
        .take(CARD_LIMIT)
        .map(|c| OmniResult {
            id: format!("card:{}", c.id),
            kind: OmniKind::Card,
            title: if c.title.is_empty() {
                c.url.clone()
            } else {
                c.title.clone()
            },
            subtitle: Some(display_url(&c.url)),
            action: OmniAction::FocusCard(c.id.clone()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::omni::engines::{default_engines, Engine};
    use crate::omni::history::Visit;
    use crate::omni::OmniCard;

    fn ctx<'a>(
        q: &'a str,
        history: &'a [Visit],
        cards: &'a [OmniCard],
        engines: &'a [Engine],
    ) -> OmniCtx<'a> {
        OmniCtx {
            q,
            scope: None,
            history,
            cards,
            engines,
            template: crate::omni::address::SEARCH_TEMPLATE,
        }
    }

    #[test]
    fn what_you_typed_is_the_first_result_either_way() {
        let (h, c, e) = (vec![], vec![], default_engines());
        let r = address_result(&ctx("example.com", &h, &c, &e));
        assert_eq!(
            r[0].action,
            OmniAction::Navigate("https://example.com".into())
        );
        let r = address_result(&ctx("rust lifetimes", &h, &c, &e));
        assert_eq!(r[0].action, OmniAction::Search("rust lifetimes".into()));
        assert!(address_result(&ctx("  ", &h, &c, &e)).is_empty());
    }

    #[test]
    fn history_matches_the_title_and_the_path_but_not_the_query_string() {
        let h = vec![
            Visit {
                url: "https://a.example/login?next=rust".into(),
                title: "Sign in".into(),
                visits: 9,
                at: 9.,
            },
            Visit {
                url: "https://doc.rust-lang.org/book".into(),
                title: "The Book".into(),
                visits: 1,
                at: 1.,
            },
        ];
        let (c, e) = (vec![], default_engines());
        let r = history_results(&ctx("rust", &h, &c, &e));
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "The Book");
        assert_eq!(
            r[0].subtitle.as_deref(),
            Some("https://doc.rust-lang.org/book")
        );
    }

    #[test]
    fn an_untitled_entry_shows_its_url_as_the_title() {
        let h = vec![Visit {
            url: "https://a.example".into(),
            title: String::new(),
            visits: 1,
            at: 1.,
        }];
        let (c, e) = (vec![], default_engines());
        let r = history_results(&ctx("a.ex", &h, &c, &e));
        assert_eq!(r[0].title, "https://a.example");
        assert_eq!(r[0].subtitle, None);
    }

    #[test]
    fn an_open_card_is_focused_not_navigated() {
        let c = vec![OmniCard {
            id: "c1".into(),
            title: "Hacker News".into(),
            url: "https://news.ycombinator.com".into(),
        }];
        let (h, e) = (vec![], default_engines());
        let r = card_results(&ctx("hacker", &h, &c, &e));
        assert_eq!(r[0].action, OmniAction::FocusCard("c1".into()));
        assert_eq!(r[0].kind, OmniKind::Card);
        // An empty query lists every open card: Cmd+L on its own is a card
        // switcher.
        assert_eq!(card_results(&ctx("", &h, &c, &e)).len(), 1);
    }

    #[test]
    fn a_display_url_reads_like_an_address_bar() {
        assert_eq!(display_url("https://www.example.com/a"), "example.com/a");
        assert_eq!(display_url("http://localhost:8087"), "localhost:8087");
    }
}
