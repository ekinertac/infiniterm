//! Greedy subsequence scoring and highlight positions for palette search.
//! Port of fuzzy.ts and its tests; palette.rs consumes these scores.
//! Offsets use UTF-16 code units to match the reference, including non-ASCII labels.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub score: f64,
    pub matches: Vec<usize>,
}
pub fn fuzzy_match(query: &str, text: &str) -> Option<Match> {
    let q = query
        .trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
        .to_lowercase();
    if q.is_empty() {
        return Some(Match {
            score: 0.,
            matches: vec![],
        });
    }
    let lower: Vec<u16> = text.to_lowercase().encode_utf16().collect();
    let mut matches = vec![];
    let mut score = 0.;
    let mut at = 0;
    for c in q.chars() {
        let mut units = [0; 2];
        let needle = c.encode_utf16(&mut units);
        let found = at
            + lower
                .get(at..)?
                .windows(needle.len())
                .position(|w| w == needle)?;
        let gap = found - at;
        if gap > 0 {
            score -= gap.min(12) as f64;
        }
        score += 2.;
        if found == 0 {
            score += 16.;
        } else if [32, 46, 45, 95, 47, 58, 43].contains(&lower[found - 1]) {
            score += 12.;
        }
        if matches.last().is_some_and(|last| found == last + 1) {
            score += 9.;
        }
        matches.push(found);
        at = found + 1;
    }
    score -= text.encode_utf16().count().min(60) as f64 / 10.;
    Some(Match { score, matches })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn score(q: &str, t: &str) -> f64 {
        fuzzy_match(q, t).map_or(f64::NEG_INFINITY, |m| m.score)
    }
    #[test]
    fn subsequence_in_order() {
        assert!(fuzzy_match("nwc", "New terminal card").is_some());
        assert!(fuzzy_match("ntc", "New terminal card").is_some());
    }
    #[test]
    fn absent_or_out_of_order_rejected() {
        assert!(fuzzy_match("xyz", "New terminal card").is_none());
        assert!(fuzzy_match("cardnew", "New terminal card").is_none());
    }
    #[test]
    fn case_insensitive() {
        assert!(fuzzy_match("NEW", "new terminal card").is_some());
    }
    #[test]
    fn empty_query_has_no_opinion() {
        for q in ["", "   "] {
            assert_eq!(
                fuzzy_match(q, "anything"),
                Some(Match {
                    score: 0.,
                    matches: vec![]
                })
            );
        }
    }
    #[test]
    fn positions_for_highlighting() {
        assert_eq!(
            fuzzy_match("ntc", "New terminal card").unwrap().matches,
            vec![0, 4, 13]
        );
    }
    #[test]
    fn start_bonus() {
        assert!(score("zoom", "Zoom in") > score("zoom", "Canvas zoom in"));
    }
    #[test]
    fn word_boundary_bonus() {
        assert!(score("ntc", "New terminal card") > score("ntc", "antic"));
    }
    #[test]
    fn consecutive_run_bonus() {
        assert!(score("term", "terminal") > score("term", "the rat moves"));
    }
    #[test]
    fn shorter_text_bonus() {
        assert!(score("zi", "Zoom in") > score("zi", "Zoom in on absolutely everything"));
    }
    #[test]
    fn gap_penalty() {
        assert!(score("ab", "ab") > score("ab", &format!("a{}b", "x".repeat(8))));
    }
    #[test]
    fn gap_penalty_capped() {
        let near = score("ab", &format!("a{}b", "x".repeat(20)));
        let far = score("ab", &format!("a{}b", "x".repeat(200)));
        assert!(near - far < 20.);
    }
    #[test]
    fn empty_text() {
        assert!(fuzzy_match("a", "").is_none());
        assert_eq!(
            fuzzy_match("", ""),
            Some(Match {
                score: 0.,
                matches: vec![]
            })
        );
    }
    #[test]
    fn unicode_positions_match_utf16_reference() {
        assert_eq!(fuzzy_match("z", "🦀 Zoom").unwrap().matches, vec![3]);
        assert_eq!(fuzzy_match("ç", "Çalışma").unwrap().matches, vec![0]);
    }
}
