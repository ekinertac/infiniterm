//! The sites the omnibox can scope to: "Tab to search GitHub". A static seed
//! list keyed by host, overridable from settings (`browser.engines`).
//!
//! Called by `omni::rank`, which detects the offer when unscoped and builds
//! the one result when scoped. Deriving engines from history or from a
//! site's OpenSearch description is a later slice; this is the list that
//! makes the feature useful on day one. Related: omni/address.rs.
use crate::omni::address::search_url;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Engine {
    /// What you type to reach it: the host, so "github.com" and "git" both
    /// find GitHub.
    pub keyword: String,
    pub name: String,
    /// Template; `%s` is the encoded query.
    pub search_url: String,
}

/// keyword, name, template.
pub const DEFAULT_ENGINES: &[(&str, &str, &str)] = &[
    ("google.com", "Google", "https://www.google.com/search?q=%s"),
    ("github.com", "GitHub", "https://github.com/search?q=%s"),
    (
        "youtube.com",
        "YouTube",
        "https://www.youtube.com/results?search_query=%s",
    ),
    (
        "wikipedia.org",
        "Wikipedia",
        "https://en.wikipedia.org/w/index.php?search=%s",
    ),
    (
        "stackoverflow.com",
        "Stack Overflow",
        "https://stackoverflow.com/search?q=%s",
    ),
    (
        "developer.mozilla.org",
        "MDN",
        "https://developer.mozilla.org/en-US/search?q=%s",
    ),
    ("npmjs.com", "npm", "https://www.npmjs.com/search?q=%s"),
    ("crates.io", "crates.io", "https://crates.io/search?q=%s"),
    (
        "docs.rs",
        "docs.rs",
        "https://docs.rs/releases/search?query=%s",
    ),
];

pub fn default_engines() -> Vec<Engine> {
    DEFAULT_ENGINES
        .iter()
        .map(|(keyword, name, url)| Engine {
            keyword: (*keyword).into(),
            name: (*name).into(),
            search_url: (*url).into(),
        })
        .collect()
}

/// Strips scheme, leading www. and trailing slashes so a typed address
/// matches a keyword.
fn normalise(q: &str) -> String {
    let lower = q.trim().to_lowercase();
    let no_scheme = lower
        .split_once("://")
        .map(|(_, rest)| rest.to_string())
        .unwrap_or(lower);
    let no_www = no_scheme.strip_prefix("www.").unwrap_or(&no_scheme);
    no_www.trim_end_matches('/').to_string()
}

/// The engine to offer for what is typed so far, by prefix of its name, its
/// keyword, or the keyword's first label ("youtube" from "youtube.com").
///
/// A prefix and not a full match, because the offer is only useful while you
/// are still typing the first few letters.
pub fn offer<'a>(q: &str, engines: &'a [Engine]) -> Option<&'a Engine> {
    let n = normalise(q);
    if n.is_empty() {
        return None;
    }
    engines.iter().find(|e| {
        let name = e.name.to_lowercase();
        let keyword = e.keyword.to_lowercase();
        let label = keyword.split('.').next().unwrap_or("").to_string();
        name.starts_with(&n) || keyword.starts_with(&n) || label.starts_with(&n)
    })
}

pub fn resolve<'a>(keyword: &str, engines: &'a [Engine]) -> Option<&'a Engine> {
    engines
        .iter()
        .find(|e| e.keyword.eq_ignore_ascii_case(keyword))
}

pub fn scoped_url(engine: &Engine, query: &str) -> String {
    search_url(&engine.search_url, query)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engines() -> Vec<Engine> {
        default_engines()
    }

    // The offer is a PREFIX match, so it appears while you are still typing
    // the first few letters, which is the only time it is useful.
    #[test]
    fn a_prefix_of_a_name_host_or_label_offers_that_engine() {
        assert_eq!(
            offer("git", &engines()).map(|e| e.name.as_str()),
            Some("GitHub")
        );
        assert_eq!(
            offer("y", &engines()).map(|e| e.name.as_str()),
            Some("YouTube")
        );
        assert_eq!(
            offer("https://www.youtube.com", &engines()).map(|e| e.name.as_str()),
            Some("YouTube")
        );
        assert_eq!(offer("zzz", &engines()), None);
        assert_eq!(offer("   ", &engines()), None);
    }

    #[test]
    fn a_scope_resolves_by_keyword_and_builds_the_url() {
        let all = engines();
        let e = resolve("github.com", &all).expect("seeded");
        assert_eq!(
            scoped_url(e, "rope crate"),
            "https://github.com/search?q=rope%20crate"
        );
        assert!(resolve("nowhere.example", &all).is_none());
    }

    // The registry is settings-driven, so a seed that lies about its shape
    // would be a bug nobody sees until Tab does nothing.
    #[test]
    fn the_seed_list_is_well_formed() {
        for e in default_engines() {
            assert!(e.search_url.contains("%s"), "{} has no %s", e.name);
            assert!(!e.keyword.is_empty());
        }
    }
}
