//! What you have run before, so the palette stops making you search for the
//! five things you actually use. Port of paletteUsage.ts and its tests.
//!
//! Recency and frequency do different jobs and are never merged into one
//! "frecency" number: the recent section is pure recency (a list of five
//! whose positions you can learn), the rest of the list is fuzzy score plus
//! a capped frequency bonus (a nudge, never an override). One blended score
//! would give neither and could not be predicted, which for a palette is
//! the whole game.
//!
//! Entries keep insertion order the way a JS object does, because the
//! reference's ties (equal `at`) resolve by that order. `palette.rs` takes
//! the bonus as a callback; `saved_layout.rs` persists the map.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Use {
    /// Times run.
    pub n: f64,
    /// Epoch ms of the last run.
    pub at: f64,
}

/// Keyed by `use_key`, in insertion order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage(pub Vec<(String, Use)>);

impl Usage {
    pub fn get(&self, key: &str) -> Option<&Use> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, u)| u)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(k, _)| k.as_str())
    }
}

/// Keyed by source AND id: sources share the palette and their ids come from
/// different namespaces, so a theme called `card.close` must not inherit the
/// command's history.
pub fn use_key(source_id: &str, id: &str) -> String {
    format!("{source_id}:{id}")
}

/// How many entries the recent section shows.
pub const RECENT_LIMIT: usize = 5;

/// Entries kept before the oldest are dropped. There are 521 themes, and
/// choosing one records a use; without a cap the map grows for the life of
/// the install and is written to disk on every change.
pub const USAGE_LIMIT: usize = 200;

/// Returns a new map; the input is untouched. An existing key keeps its
/// position, a new one goes last, as a JS object spread does.
pub fn record_use(usage: &Usage, key: &str, now: f64) -> Usage {
    let mut out = usage.clone();
    match out.0.iter_mut().find(|(k, _)| k == key) {
        Some((_, u)) => {
            *u = Use {
                n: u.n + 1.,
                at: now,
            }
        }
        None => out.0.push((key.to_string(), Use { n: 1., at: now })),
    }
    out
}

/// The most recently used keys, newest first.
pub fn recent_keys(usage: &Usage, limit: usize) -> Vec<String> {
    let mut entries: Vec<&(String, Use)> = usage.0.iter().collect();
    // Stable, so equal timestamps keep insertion order like the reference.
    entries.sort_by(|a, b| {
        b.1.at
            .partial_cmp(&a.1.at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    entries
        .into_iter()
        .take(limit)
        .map(|(k, _)| k.clone())
        .collect()
}

/// Weight added to a fuzzy score for something used before. Logarithmic:
/// never versus once is what matters, forty versus fifty is nothing. Capped
/// well below what a better textual match is worth.
pub const USE_BONUS: f64 = 4.;

pub fn usage_bonus(use_: Option<&Use>) -> f64 {
    match use_ {
        Some(u) if u.n > 0. => (1. + u.n).log2() * USE_BONUS,
        _ => 0.,
    }
}

/// Drops the least recently used entries once the map is over `limit`.
/// A map under the limit comes back as the same value.
pub fn prune_usage(usage: &Usage, limit: usize) -> Usage {
    if usage.len() <= limit {
        return usage.clone();
    }
    let mut entries = usage.0.clone();
    entries.sort_by(|a, b| {
        b.1.at
            .partial_cmp(&a.1.at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    entries.truncate(limit);
    Usage(entries)
}

/// Reads a `usage` map off a parsed file, keeping only well-formed entries.
pub fn parse_usage(raw: &Value) -> Usage {
    let Some(map) = raw.as_object() else {
        return Usage::default();
    };
    let mut out = Usage::default();
    for (key, value) in map {
        let Some(entry) = value.as_object() else {
            continue;
        };
        let (Some(n), Some(at)) = (
            entry.get("n").and_then(Value::as_f64),
            entry.get("at").and_then(Value::as_f64),
        ) else {
            continue;
        };
        if !n.is_finite() || !at.is_finite() || n <= 0. {
            continue;
        }
        out.0.push((key.clone(), Use { n, at }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn usage(entries: &[(&str, f64, f64)]) -> Usage {
        Usage(
            entries
                .iter()
                .map(|(k, n, at)| (k.to_string(), Use { n: *n, at: *at }))
                .collect(),
        )
    }

    // useKey: a theme called `card.close` must not inherit the command's history.
    #[test]
    fn namespaces_by_source() {
        assert_ne!(
            use_key("commands", "card.close"),
            use_key("themes", "card.close")
        );
    }

    // recordUse
    #[test]
    fn counts_the_first_run_and_every_one_after() {
        let u = record_use(&Usage::default(), "a", 1000.);
        assert_eq!(u.get("a"), Some(&Use { n: 1., at: 1000. }));
        let u = record_use(&u, "a", 2000.);
        assert_eq!(u.get("a"), Some(&Use { n: 2., at: 2000. }));
    }

    #[test]
    fn does_not_mutate_what_it_was_given() {
        let before = usage(&[("a", 1., 1.)]);
        record_use(&before, "a", 2.);
        assert_eq!(before.get("a").unwrap().n, 1.);
    }

    // recentKeys: pure recency, "the thing I just did, again".
    #[test]
    fn is_newest_first_regardless_of_how_often_each_was_used() {
        let u = usage(&[("old", 50., 1.), ("middle", 1., 5.), ("newest", 1., 9.)]);
        assert_eq!(recent_keys(&u, 3), ["newest", "middle", "old"]);
    }

    #[test]
    fn honours_the_limit() {
        let u = usage(&[("old", 50., 1.), ("middle", 1., 5.), ("newest", 1., 9.)]);
        assert_eq!(recent_keys(&u, 2), ["newest", "middle"]);
        assert!(recent_keys(&Usage::default(), 5).is_empty());
    }

    // usageBonus
    #[test]
    fn is_nothing_for_something_never_used() {
        assert_eq!(usage_bonus(None), 0.);
        assert_eq!(usage_bonus(Some(&Use { n: 0., at: 1. })), 0.);
    }

    #[test]
    fn grows_with_use() {
        assert!(
            usage_bonus(Some(&Use { n: 3., at: 1. })) > usage_bonus(Some(&Use { n: 1., at: 1. }))
        );
    }

    // The gap between never and once is what matters; forty versus fifty is not.
    #[test]
    fn has_diminishing_returns() {
        let b = |n: f64| usage_bonus(Some(&Use { n, at: 1. }));
        assert!(b(50.) - b(49.) < b(2.) - b(1.));
    }

    // A nudge, not an override: it must not beat a much better textual match.
    #[test]
    fn stays_small_enough_to_lose_to_a_better_match() {
        assert!(usage_bonus(Some(&Use { n: 1000., at: 1. })) < 45.);
    }

    // pruneUsage
    #[test]
    fn leaves_a_map_under_the_limit_alone() {
        let u = usage(&[("a", 1., 1.)]);
        assert_eq!(prune_usage(&u, 10), u);
    }

    #[test]
    fn drops_the_least_recently_used() {
        let u = usage(&[("a", 9., 1.), ("b", 1., 2.), ("c", 1., 3.)]);
        let mut keys: Vec<String> = prune_usage(&u, 2).keys().map(String::from).collect();
        keys.sort();
        assert_eq!(keys, ["b", "c"]);
    }

    #[test]
    fn defaults_to_a_cap_that_521_themes_cannot_blow_past_silently() {
        let many = Usage(
            (0..600)
                .map(|i| {
                    (
                        format!("k{i}"),
                        Use {
                            n: 1.,
                            at: i as f64,
                        },
                    )
                })
                .collect(),
        );
        assert_eq!(prune_usage(&many, USAGE_LIMIT).len(), USAGE_LIMIT);
    }

    // parseUsage
    #[test]
    fn reads_well_formed_entries() {
        assert_eq!(
            parse_usage(&json!({"a": {"n": 2, "at": 100}})),
            usage(&[("a", 2., 100.)])
        );
    }

    #[test]
    fn drops_anything_malformed_rather_than_throwing() {
        let raw =
            json!({"a": {"n": "two", "at": 1}, "b": null, "c": {"n": 1}, "d": {"n": 0, "at": 1}});
        assert!(parse_usage(&raw).is_empty());
        assert!(parse_usage(&Value::Null).is_empty());
        assert!(parse_usage(&json!([])).is_empty());
        assert!(parse_usage(&json!("nope")).is_empty());
    }
}
