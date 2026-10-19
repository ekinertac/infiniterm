//! Palette ranking, highlighting, navigation, and recent/rest sections.
//! Port of palette.ts and its tests. UI sources provide items and usage weights.
//! Label matches outrank hints; truncation reports the true total before limiting.
use crate::fuzzy::{strict_match, Match};
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
        // `strict_match`: a word of four letters or more must be in the row
        // (#490); a shorter query is the loose abbreviation match as before.
        let matched = if let Some(mut m) = strict_match(query, &item.label) {
            m.score += weight;
            Some(m)
        } else {
            item.hint
                .as_deref()
                .filter(|h| !h.is_empty())
                .and_then(|h| strict_match(query, h))
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
/// A row's label as it is drawn: cut to the room it has, with the matched
/// letters as offsets into the SHOWN text (UTF-16, like `Match::matches`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FittedLabel {
    pub text: String,
    pub matches: Vec<usize>,
}

/// How much of the label before the first match stays when the match lies
/// past the cut and the window moves to it.
const CONTEXT_CHARS: usize = 8;
const ELLIPSIS: char = '\u{2026}';

/// Cuts a row's label to `room` (whatever `measure` counts in) with an
/// ellipsis, whatever its length (#492: a card's label can be a whole
/// sentence, an agent's title, and it ran past the palette's edge). A label
/// that fits is returned as it is. Otherwise it is cut at the end; but when
/// the first matched letter would fall in the cut part the window moves to
/// it (`…` before, some context, the match, `…` after if more follows), so
/// the thing the user typed is on screen. The highlights follow the text:
/// those outside the window are dropped, the rest are shifted.
/// Empty text when not even the ellipsis fits.
pub fn fit_label(
    label: &str,
    matches: &[usize],
    room: f32,
    measure: impl Fn(&str) -> f32,
) -> FittedLabel {
    if label.is_empty() || measure(label) <= room {
        return FittedLabel {
            text: label.to_string(),
            matches: matches.to_vec(),
        };
    }
    let chars: Vec<char> = label.chars().collect();
    // The UTF-16 offset at which each character starts, and the total.
    let mut offsets: Vec<usize> = Vec::with_capacity(chars.len() + 1);
    let mut at = 0;
    for c in &chars {
        offsets.push(at);
        at += c.len_utf16();
    }
    offsets.push(at);
    let char_of = |unit: usize| offsets.partition_point(|&o| o <= unit).saturating_sub(1);
    let first = matches.iter().map(|&m| char_of(m)).min();
    let from = |s: usize, e: usize| -> String {
        let mut text = String::new();
        if s > 0 {
            text.push(ELLIPSIS);
        }
        text.extend(chars[s..e].iter());
        if e < chars.len() {
            text.push(ELLIPSIS);
        }
        text
    };
    // The most characters from `s` that fit, at least `need` of them.
    let widest = |s: usize, need: usize| -> Option<usize> {
        let (mut lo, mut hi) = (s, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if measure(&from(s, mid)) <= room {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        (lo >= need && measure(&from(s, lo)) <= room).then_some(lo)
    };
    let shown = |s: usize, e: usize| -> FittedLabel {
        let lead = usize::from(s > 0);
        FittedLabel {
            text: from(s, e),
            matches: matches
                .iter()
                .filter(|&&m| (offsets[s]..offsets[e]).contains(&m))
                .map(|&m| lead + m - offsets[s])
                .collect(),
        }
    };
    // The end cut first: it keeps the label's start, which names the row.
    if let Some(e) = widest(0, 0) {
        if first.is_none_or(|f| f < e) {
            return shown(0, e);
        }
    } else {
        return FittedLabel {
            text: String::new(),
            matches: vec![],
        };
    }
    // The first match is in the cut part: move the window to it.
    let first = first.unwrap_or(0);
    for context in [CONTEXT_CHARS, 0] {
        let s = first.saturating_sub(context);
        if let Some(e) = widest(s, first + 1) {
            // The window reached the label's end with room to spare: take
            // more of what comes before it instead of leaving the row empty.
            if e == chars.len() {
                let (mut lo, mut hi) = (0usize, s);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    if measure(&from(mid, e)) <= room {
                        hi = mid;
                    } else {
                        lo = mid + 1;
                    }
                }
                return shown(lo, e);
            }
            return shown(s, e);
        }
    }
    // Not even the match fits beside an ellipsis: the end cut, unmatched.
    let e = widest(0, 0).unwrap_or(0);
    shown(0, e)
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
    fn a_word_of_four_letters_lists_only_rows_that_hold_it() {
        // (see also fuzzy.rs: the table of the screenshot)
        // The screenshot of #490: `check` listed rows with scattered letters.
        let items = vec![
            item("canvas.tidy", "Canvas: tidy the cards into a block", None),
            item("app.update", "App: check for updates", None),
            item("card.mask", "Card: mask with a decoy / unmask", None),
            item(
                "window.colour",
                "Window: change the title bar and Dock colour…",
                None,
            ),
        ];
        let r = rank(&items, "check", MAX_RESULTS, |_| 0.);
        assert_eq!(ids(&r), vec!["app.update"]);
        assert_eq!(r.total, 1);
        // An abbreviation of three letters is still the loose match.
        let r = rank(&items, "cdc", MAX_RESULTS, |_| 0.);
        assert!(!r.items.is_empty());
    }

    // A fake measure: every character is ten wide, so the arithmetic is one
    // a reader can do.
    fn ten(text: &str) -> f32 {
        text.chars().count() as f32 * 10.
    }
    fn long_label() -> String {
        // 200 characters, words of nine letters; the word `unchecked` last.
        let mut s = String::new();
        while s.chars().count() + 10 <= 190 {
            s.push_str("sentence. ");
        }
        s.push_str("unchecked");
        while s.chars().count() < 200 {
            s.push('.');
        }
        s
    }
    fn utf16_of(text: &str, unit: usize) -> char {
        let units: Vec<u16> = text.encode_utf16().collect();
        char::from_u32(units[unit] as u32).unwrap()
    }

    #[test]
    fn a_label_that_fits_is_left_alone() {
        let f = fit_label("Zoom in", &[0, 1], 500., ten);
        assert_eq!(f.text, "Zoom in");
        assert_eq!(f.matches, vec![0, 1]);
    }

    #[test]
    fn a_long_label_is_cut_at_the_end_with_an_ellipsis() {
        let label = long_label();
        // A match near the start: the cut is at the end.
        let f = fit_label(&label, &[0, 1], 300., ten);
        assert_eq!(f.text.chars().count(), 30);
        assert!(f.text.ends_with('\u{2026}'));
        assert!(f.text.starts_with("se"));
        assert_eq!(f.matches, vec![0, 1]);
    }

    #[test]
    fn a_match_past_the_cut_moves_the_window_to_it() {
        // `check` inside `unchecked` at the end of 200 characters, the
        // screenshot of #492.
        let label = long_label();
        let at = label.find("unchecked").unwrap() + 2;
        let matches: Vec<usize> = (at..at + 5).collect();
        let f = fit_label(&label, &matches, 300., ten);
        assert!(f.text.chars().count() <= 30, "{:?}", f.text);
        assert!(f.text.starts_with('\u{2026}'), "{:?}", f.text);
        assert_eq!(f.matches.len(), 5);
        let shown: String = f.matches.iter().map(|&m| utf16_of(&f.text, m)).collect();
        assert_eq!(shown, "check");
    }

    #[test]
    fn a_window_at_the_end_of_the_label_uses_the_room() {
        // The match is near the end: the window must not stop at a few
        // characters of context while the row has room for thirty.
        let label = long_label();
        let at = label.find("unchecked").unwrap() + 2;
        let matches: Vec<usize> = (at..at + 5).collect();
        let f = fit_label(&label, &matches, 300., ten);
        assert!(f.text.chars().count() >= 29, "{:?}", f.text);
        assert!(ten(&f.text) <= 300.);
        let shown: String = f.matches.iter().map(|&m| utf16_of(&f.text, m)).collect();
        assert_eq!(shown, "check");
    }

    #[test]
    fn a_label_of_two_hundred_characters_fits_every_width() {
        let label = long_label();
        let at = label.find("unchecked").unwrap() + 2;
        let tail: Vec<usize> = (at..at + 5).collect();
        for room in [30., 50., 100., 150., 300., 600., 1500., 1990., 2000.] {
            for matches in [vec![], vec![0, 1], tail.clone()] {
                let f = fit_label(&label, &matches, room, ten);
                assert!(ten(&f.text) <= room, "room {room}: {:?}", f.text);
                // Every highlight points at the same letter it did before.
                for (&shown, &was) in f.matches.iter().zip(
                    matches
                        .iter()
                        .filter(|m| f.matches.len() == matches.len() || **m < 2 || **m >= at),
                ) {
                    assert_eq!(
                        utf16_of(&f.text, shown),
                        label.chars().nth(was).unwrap(),
                        "room {room}"
                    );
                }
            }
        }
    }

    #[test]
    fn no_room_for_even_an_ellipsis_shows_nothing() {
        let f = fit_label(&long_label(), &[0], 5., ten);
        assert_eq!(f.text, "");
        assert!(f.matches.is_empty());
    }

    #[test]
    fn offsets_follow_utf16_through_the_cut() {
        // A crab is two UTF-16 units; the highlight must keep pointing at the z.
        let label = format!("🦀 {} zoom", "x".repeat(150));
        let at = label.encode_utf16().count() - 4;
        let f = fit_label(&label, &[at], 300., ten);
        assert!(ten(&f.text) <= 300.);
        assert_eq!(f.matches.len(), 1);
        assert_eq!(utf16_of(&f.text, f.matches[0]), 'z');
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
