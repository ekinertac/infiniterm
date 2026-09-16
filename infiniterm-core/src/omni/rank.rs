//! Sections in a fixed order, the best history match promoted to the top,
//! and the inline completion the field draws behind what you typed.
//!
//! The order IS the ranking: there is no score here. A section exists or it
//! does not, and inside one the provider's own order stands, which is
//! frequency for history and the canvas's order for cards. A score would be
//! a knob nobody could explain and nobody could tune; glass shipped without
//! one too.
//!
//! Called by the model, which holds the state, and by `ift omni`. Related:
//! omni/providers.rs, omni/engines.rs.
use crate::omni::engines::{offer as offer_engine, resolve, scoped_url};
use crate::omni::history::Visit;
use crate::omni::providers::{address_result, card_results, display_url, history_results};
use crate::omni::{OmniAction, OmniCtx, OmniKind, OmniResult, OmniSection};

/// More than this and the network's guesses crowd out the local answers.
pub const SUGGEST_LIMIT: usize = 3;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OmniResponse {
    pub sections: Vec<OmniSection>,
    /// The display URL to complete the field with, if any.
    pub completion: Option<String>,
    /// (keyword, name) of the site Tab would scope to.
    pub offer: Option<(String, String)>,
}

/// The most-visited entry whose display URL starts with what was typed and
/// is not equal to it. History is already in ranked order, so the first hit
/// is the best one.
pub fn best_completion(q: &str, history: &[Visit]) -> Option<String> {
    let needle = q.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    history.iter().find_map(|v| {
        let shown = display_url(&v.url);
        let lower = shown.to_lowercase();
        (lower.starts_with(&needle) && lower != needle).then_some(shown)
    })
}

fn suggestion_results(q: &str, suggestions: &[String]) -> Vec<OmniResult> {
    let typed = q.trim().to_lowercase();
    let mut seen: Vec<String> = vec![];
    suggestions
        .iter()
        .filter(|s| {
            let lower = s.to_lowercase();
            // The one identical to the query is already the top row.
            if lower == typed || seen.contains(&lower) {
                return false;
            }
            seen.push(lower);
            true
        })
        .take(SUGGEST_LIMIT)
        .map(|s| OmniResult {
            id: format!("suggest:{s}"),
            kind: OmniKind::Search,
            title: s.clone(),
            subtitle: Some("Search".into()),
            action: OmniAction::Search(s.clone()),
        })
        .collect()
}

/// Moves the result whose URL matches the completion into its own section at
/// the top, out of whichever section held it. Enter then goes where the
/// field says it will, which is the whole point of showing a completion.
fn promote(sections: Vec<OmniSection>, completion: &str) -> Vec<OmniSection> {
    let target = completion.to_lowercase();
    let mut found: Option<OmniResult> = None;
    let mut rest: Vec<OmniSection> = vec![];
    for section in sections {
        let mut kept = vec![];
        for r in section.results {
            let hit = matches!(&r.action, OmniAction::Navigate(url)
                if display_url(url).to_lowercase() == target);
            if hit && found.is_none() {
                found = Some(r);
            } else {
                kept.push(r);
            }
        }
        if !kept.is_empty() {
            rest.push(OmniSection {
                heading: section.heading,
                results: kept,
            });
        }
    }
    match found {
        Some(r) => {
            let mut out = vec![OmniSection {
                heading: "best match".into(),
                results: vec![r],
            }];
            out.extend(rest);
            out
        }
        None => rest,
    }
}

pub fn rank(ctx: &OmniCtx, suggestions: &[String]) -> OmniResponse {
    // Scoped, the box asks one question, and the local sources have nothing
    // to say about a search that has not happened yet.
    if let Some(keyword) = ctx.scope {
        let Some(engine) = resolve(keyword, ctx.engines) else {
            return OmniResponse::default();
        };
        let offer = Some((engine.keyword.clone(), engine.name.clone()));
        let typed = ctx.q.trim();
        if typed.is_empty() {
            return OmniResponse {
                sections: vec![],
                completion: None,
                offer,
            };
        }
        return OmniResponse {
            sections: vec![OmniSection {
                heading: format!("search {}", engine.name),
                results: vec![OmniResult {
                    id: format!("scoped:{typed}"),
                    kind: OmniKind::Search,
                    title: typed.to_string(),
                    subtitle: Some(format!("Search {}", engine.name)),
                    action: OmniAction::Navigate(scoped_url(engine, typed)),
                }],
            }],
            completion: None,
            offer,
        };
    }

    let completion = best_completion(ctx.q, ctx.history);
    let offer = offer_engine(ctx.q, ctx.engines).map(|e| (e.keyword.clone(), e.name.clone()));

    let mut sections = vec![];
    for (heading, results) in [
        ("search", address_result(ctx)),
        ("suggestions", suggestion_results(ctx.q, suggestions)),
        ("history", history_results(ctx)),
        ("cards", card_results(ctx)),
    ] {
        if !results.is_empty() {
            sections.push(OmniSection {
                heading: heading.into(),
                results,
            });
        }
    }
    if let Some(c) = &completion {
        sections = promote(sections, c);
    }
    OmniResponse {
        sections,
        completion,
        offer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::omni::engines::{default_engines, Engine};
    use crate::omni::OmniCard;

    fn visit(url: &str, title: &str, visits: u32) -> Visit {
        Visit {
            url: url.into(),
            title: title.into(),
            visits,
            at: visits as f64,
        }
    }

    fn ctx<'a>(
        q: &'a str,
        scope: Option<&'a str>,
        history: &'a [Visit],
        cards: &'a [OmniCard],
        engines: &'a [Engine],
    ) -> OmniCtx<'a> {
        OmniCtx {
            q,
            scope,
            history,
            cards,
            engines,
            template: crate::omni::address::SEARCH_TEMPLATE,
        }
    }

    #[test]
    fn sections_come_in_a_fixed_order_and_empty_ones_are_dropped() {
        let h = vec![visit("https://news.ycombinator.com", "Hacker News", 5)];
        let c = vec![OmniCard {
            id: "c1".into(),
            title: "Hacker News".into(),
            url: "https://news.ycombinator.com".into(),
        }];
        let e = default_engines();
        let r = rank(&ctx("news", None, &h, &c, &e), &[]);
        let headings: Vec<&str> = r.sections.iter().map(|s| s.heading.as_str()).collect();
        assert_eq!(headings, vec!["best match", "search", "cards"]);
        // Nothing matches: only what was typed is left.
        let r = rank(&ctx("zzzz", None, &h, &c, &e), &[]);
        assert_eq!(
            r.sections
                .iter()
                .map(|s| s.heading.as_str())
                .collect::<Vec<_>>(),
            vec!["search"]
        );
    }

    #[test]
    fn the_best_history_prefix_becomes_the_completion_and_is_promoted() {
        let h = vec![visit("https://news.ycombinator.com", "Hacker News", 5)];
        let (c, e) = (vec![], default_engines());
        let r = rank(&ctx("news.y", None, &h, &c, &e), &[]);
        assert_eq!(r.completion.as_deref(), Some("news.ycombinator.com"));
        assert_eq!(r.sections[0].heading, "best match");
        assert_eq!(
            r.sections[0].results[0].action,
            OmniAction::Navigate("https://news.ycombinator.com".into())
        );
        // Typed in full: nothing left to complete.
        let r = rank(&ctx("news.ycombinator.com", None, &h, &c, &e), &[]);
        assert_eq!(r.completion, None);
    }

    #[test]
    fn a_known_site_is_offered_for_tab() {
        let (h, c, e) = (vec![], vec![], default_engines());
        let r = rank(&ctx("git", None, &h, &c, &e), &[]);
        assert_eq!(r.offer, Some(("github.com".into(), "GitHub".into())));
    }

    // Scoped, the box is one thing: search THAT site. Nothing else runs.
    #[test]
    fn a_scope_replaces_every_section_with_one_result() {
        let h = vec![visit("https://news.ycombinator.com", "Hacker News", 5)];
        let c = vec![OmniCard {
            id: "c1".into(),
            title: "x".into(),
            url: "https://x.example".into(),
        }];
        let e = default_engines();
        let r = rank(&ctx("rope", Some("github.com"), &h, &c, &e), &[]);
        assert_eq!(r.sections.len(), 1);
        assert_eq!(r.sections[0].heading, "search GitHub");
        assert_eq!(
            r.sections[0].results[0].action,
            OmniAction::Navigate("https://github.com/search?q=rope".into())
        );
        assert_eq!(r.offer, Some(("github.com".into(), "GitHub".into())));
    }

    #[test]
    fn suggestions_arrive_as_their_own_section() {
        let (h, c, e) = (vec![], vec![], default_engines());
        let r = rank(
            &ctx("rust", None, &h, &c, &e),
            &["rust book".into(), "rust".into()],
        );
        let s = r
            .sections
            .iter()
            .find(|s| s.heading == "suggestions")
            .expect("section");
        assert_eq!(s.results.len(), 1);
        assert_eq!(s.results[0].action, OmniAction::Search("rust book".into()));
    }
}
