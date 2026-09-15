//! Palette ranking, highlighting, navigation, and recent/rest sections.
//! Port of palette.ts and its tests. UI sources provide items and usage weights.
//! Label matches outrank hints; truncation reports the true total before limiting.
use crate::fuzzy::{fuzzy_match, Match};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    pub id: String,
    pub label: String,
    pub hint: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RankedItem {
    pub item: PaletteItem,
    pub matched: Match,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Ranked {
    pub items: Vec<RankedItem>,
    pub total: usize,
}
pub const MAX_RESULTS: usize = 1000;
const HINT_PENALTY: f64 = 1000.;
pub fn rank(
    items: &[PaletteItem],
    query: &str,
    limit: usize,
    bonus: impl Fn(&str) -> f64,
) -> Ranked {
    let mut scored = vec![];
    for item in items {
        let weight = bonus(&item.id);
        let matched = if let Some(mut m) = fuzzy_match(query, &item.label) {
            m.score += weight;
            Some(m)
        } else {
            item.hint
                .as_deref()
                .filter(|h| !h.is_empty())
                .and_then(|h| fuzzy_match(query, h))
                .map(|m| Match {
                    score: m.score + weight - HINT_PENALTY,
                    matches: vec![],
                })
        };
        if let Some(matched) = matched {
            scored.push(RankedItem {
                item: item.clone(),
                matched,
            });
        }
    }
    // Stable sorting keeps registration order when scores tie.
    scored.sort_by(|a, b| {
        b.matched
            .score
            .partial_cmp(&a.matched.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let total = scored.len();
    scored.truncate(limit);
    Ranked {
        items: scored,
        total,
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub text: String,
    pub hit: bool,
}
pub fn highlight(text: &str, matches: &[usize]) -> Vec<Highlight> {
    if matches.is_empty() {
        return if text.is_empty() {
            vec![]
        } else {
            vec![Highlight {
                text: text.into(),
                hit: false,
            }]
        };
    }
    // UTF-16 indices from fuzzy_match must be resolved before converting to Rust text.
    let mut runs: Vec<(Vec<u16>, bool)> = vec![];
    for (i, unit) in text.encode_utf16().enumerate() {
        let hit = matches.contains(&i);
        if let Some((units, last_hit)) = runs.last_mut().filter(|(_, last_hit)| *last_hit == hit) {
            units.push(unit);
            debug_assert_eq!(*last_hit, hit);
        } else {
            runs.push((vec![unit], hit));
        }
    }
    runs.into_iter()
        .map(|(units, hit)| Highlight {
            text: String::from_utf16_lossy(&units),
            hit,
        })
        .collect()
}
pub fn step_index(index: usize, count: usize, step: isize) -> usize {
    if count == 0 {
        return 0;
    }
    (index as isize + step).rem_euclid(count as isize) as usize
}
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteSection {
    pub title: String,
    pub items: Vec<RankedItem>,
}
pub fn sectionise(
    ranked: &[RankedItem],
    recent: &[String],
    key_of: impl Fn(&PaletteItem) -> String,
    rest_title: &str,
) -> Vec<PaletteSection> {
    let order = |i: &RankedItem| recent.iter().rposition(|key| key == &key_of(&i.item));
    let mut top = vec![];
    let mut rest = vec![];
    for item in ranked {
        if order(item).is_some() {
            top.push(item.clone());
        } else {
            rest.push(item.clone());
        }
    }
    top.sort_by_key(|i| order(i).unwrap_or(0));
    let mut sections = vec![];
    if !top.is_empty() {
        sections.push(PaletteSection {
            title: "recent".into(),
            items: top,
        });
    }
    if !rest.is_empty() {
        sections.push(PaletteSection {
            title: if sections.is_empty() {
                String::new()
            } else {
                rest_title.into()
            },
            items: rest,
        });
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(id: &str, label: &str, hint: Option<&str>) -> PaletteItem {
        PaletteItem {
            id: id.into(),
            label: label.into(),
            hint: hint.map(String::from),
        }
    }
    fn items() -> Vec<PaletteItem> {
        vec![
            item("card.new.terminal", "New terminal card", Some("Cmd T")),
            item("card.close", "Close active card", Some("Cmd W")),
            item("canvas.zoom.in", "Zoom in", Some("Cmd =")),
            item("group.new", "Group the active card", Some("Cmd G")),
        ]
    }
    fn ranked(q: &str) -> Ranked {
        rank(&items(), q, MAX_RESULTS, |_| 0.)
    }
    fn ids(r: &Ranked) -> Vec<&str> {
        r.items.iter().map(|r| r.item.id.as_str()).collect()
    }
    #[test]
    fn empty_query_source_order() {
        assert_eq!(
            ids(&ranked("")),
            items().iter().map(|i| i.id.as_str()).collect::<Vec<_>>()
        );
    }
    #[test]
    fn drops_nonmatches() {
        assert_eq!(ids(&ranked("zoom")), vec!["canvas.zoom.in"]);
    }
    #[test]
    fn best_match_first() {
        assert_eq!(ids(&ranked("ntc"))[0], "card.new.terminal");
        assert_eq!(ids(&ranked("grou"))[0], "group.new");
    }
    #[test]
    fn contiguous_over_scattered() {
        let r = ranked("card");
        let ids = ids(&r);
        assert!(ids.contains(&"card.close"));
        assert_eq!(ids[0], "card.new.terminal");
    }
    #[test]
    fn hint_searchable() {
        assert_eq!(ids(&ranked("Cmd G")), vec!["group.new"]);
    }
    #[test]
    fn label_outranks_hint() {
        let r = rank(
            &[
                item("a", "Something else", Some("zoom")),
                item("b", "Zoom in", None),
            ],
            "zoom",
            MAX_RESULTS,
            |_| 0.,
        );
        assert_eq!(ids(&r)[0], "b");
    }
    #[test]
    fn tie_uses_source_order() {
        let r = rank(
            &[item("second", "aa", None), item("first", "aa", None)],
            "aa",
            MAX_RESULTS,
            |_| 0.,
        );
        assert_eq!(ids(&r), vec!["second", "first"]);
    }
    #[test]
    fn realistic_theme_collection_not_cut() {
        let themes = (0..521)
            .map(|i| item(&format!("t{i}"), &format!("theme {i}"), None))
            .collect::<Vec<_>>();
        assert_eq!(rank(&themes, "", MAX_RESULTS, |_| 0.).items.len(), 521);
        const {
            assert!(MAX_RESULTS > 521);
        }
    }
    #[test]
    fn truncation_reports_true_total() {
        let many = (0..500)
            .map(|i| item(&format!("t{i}"), &format!("theme {i}"), None))
            .collect::<Vec<_>>();
        let r = rank(&many, "", 5, |_| 0.);
        assert_eq!(r.items.len(), 5);
        assert_eq!(r.total, 500);
    }
    #[test]
    fn carries_match_positions() {
        assert_eq!(ranked("zoom").items[0].matched.matches, vec![0, 1, 2, 3]);
    }
    fn run(text: &str, hit: bool) -> Highlight {
        Highlight {
            text: text.into(),
            hit,
        }
    }
    #[test]
    fn highlight_splits_runs() {
        assert_eq!(
            highlight("Zoom in", &[0, 1, 2, 3]),
            vec![run("Zoom", true), run(" in", false)]
        );
    }
    #[test]
    fn highlight_merges_adjacent() {
        assert_eq!(
            highlight("abcd", &[1, 2]),
            vec![run("a", false), run("bc", true), run("d", false)]
        );
    }
    #[test]
    fn no_matches_plain_run() {
        assert_eq!(highlight("Zoom in", &[]), vec![run("Zoom in", false)]);
        assert!(highlight("", &[]).is_empty());
    }
    #[test]
    fn selection_wraps() {
        assert_eq!(step_index(0, 3, -1), 2);
        assert_eq!(step_index(2, 3, 1), 0);
        assert_eq!(step_index(0, 3, 1), 1);
    }
    #[test]
    fn empty_selection() {
        assert_eq!(step_index(0, 0, 1), 0);
    }
    #[test]
    fn usage_bonus_lifts_equal_match() {
        let r = rank(
            &[
                item("rare", "Zoom in", None),
                item("common", "Zoom out", None),
            ],
            "zoom",
            10,
            |id| if id == "common" { 10. } else { 0. },
        );
        assert_eq!(ids(&r)[0], "common");
    }
    #[test]
    fn frequency_cannot_beat_much_better_match() {
        let r = rank(
            &[
                item("exact", "Theme", None),
                item("used", "The rat moves everything", None),
            ],
            "theme",
            10,
            |id| if id == "used" { 12. } else { 0. },
        );
        assert_eq!(ids(&r)[0], "exact");
    }
    fn section(ids: &[&str], recent: &[&str]) -> Vec<PaletteSection> {
        let r = ids
            .iter()
            .map(|id| RankedItem {
                item: item(id, id, None),
                matched: Match {
                    score: 0.,
                    matches: vec![],
                },
            })
            .collect::<Vec<_>>();
        sectionise(
            &r,
            &recent.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            |i| i.id.clone(),
            "commands",
        )
    }
    fn section_ids(s: &PaletteSection) -> Vec<&str> {
        s.items.iter().map(|i| i.item.id.as_str()).collect()
    }
    #[test]
    fn recent_first_in_recency_order() {
        let s = section(&["a", "b", "c"], &["c", "a"]);
        assert_eq!(s[0].title, "recent");
        assert_eq!(section_ids(&s[0]), vec!["c", "a"]);
        assert_eq!(s[1].title, "commands");
        assert_eq!(section_ids(&s[1]), vec!["b"]);
    }
    #[test]
    fn no_duplicate_sections() {
        let s = section(&["a", "b"], &["a"]);
        assert_eq!(
            s.iter().flat_map(section_ids).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }
    #[test]
    fn empty_sections_dropped() {
        assert_eq!(section(&["a"], &["zz"]).len(), 1);
        assert!(section(&[], &["a"]).is_empty());
    }
    #[test]
    fn sole_rest_section_unheaded() {
        assert_eq!(section(&["a"], &[])[0].title, "");
    }
    #[test]
    fn recency_survives_query_rank_order() {
        assert_eq!(
            section_ids(&section(&["b", "a"], &["a", "b"])[0]),
            vec!["a", "b"]
        );
    }
    #[test]
    fn unicode_highlight_preserves_text() {
        assert_eq!(
            highlight("🦀 Zoom", &[3, 4, 5, 6]),
            vec![run("🦀 ", false), run("Zoom", true)]
        );
    }
}
