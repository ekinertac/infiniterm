//! Greedy subsequence scoring and highlight positions for palette search.
//! Port of fuzzy.ts and its tests; palette.rs consumes these scores.
//! Offsets use UTF-16 code units to match the reference, including non-ASCII labels.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub score: f64,
    pub matches: Vec<usize>,
}
pub fn fuzzy_match(query: &str, text: &str) -> Option<Match> {
    let q: Vec<char> = query
        .trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
        .to_lowercase()
        .chars()
        .collect();
    if q.is_empty() {
        return Some(Match {
            score: 0.,
            matches: vec![],
        });
    }
    let lower: Vec<u16> = text.to_lowercase().encode_utf16().collect();
    let needles: Vec<Vec<u16>> = q
        .iter()
        .map(|c| {
            let mut units = [0; 2];
            c.encode_utf16(&mut units).to_vec()
        })
        .collect();
    // Taking the FIRST occurrence of each character was greedy in the
    // wrong way: "rgs" against "Card: #13 CG-RGS" took the r of "Card"
    // and then hunted forward, and the contiguous RGS lost to scattered
    // letters in "Group: go to the previous". Every occurrence of the
    // first character is tried as a start; from each, a character that
    // continues the run is preferred to one further on; the best wins.
    let starts: Vec<usize> = lower
        .windows(needles[0].len())
        .enumerate()
        .filter(|(_, w)| *w == needles[0].as_slice())
        .map(|(i, _)| i)
        .collect();
    let mut best: Option<Match> = None;
    for start in starts {
        let Some(m) = align_from(&lower, &needles, start) else {
            continue;
        };
        if best.as_ref().is_none_or(|b| m.score > b.score) {
            best = Some(m);
        }
    }
    best.map(|mut m| {
        m.score -= text.encode_utf16().count().min(60) as f64 / 10.;
        m
    })
}

/// One alignment: the first character at `start`, each next one at the
/// position right after the last when it is there, else the first
/// occurrence further on. Scored as the greedy walk always was.
fn align_from(lower: &[u16], needles: &[Vec<u16>], start: usize) -> Option<Match> {
    let mut matches = Vec::with_capacity(needles.len());
    let mut score = 0.;
    let mut at = start;
    for (i, needle) in needles.iter().enumerate() {
        let found = if i == 0 {
            start
        } else {
            let continues = lower
                .get(at..at + needle.len())
                .is_some_and(|w| w == needle.as_slice());
            if continues {
                at
            } else {
                at + lower
                    .get(at..)?
                    .windows(needle.len())
                    .position(|w| w == needle.as_slice())?
            }
        };
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
    Some(Match { score, matches })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn score(q: &str, t: &str) -> f64 {
        fuzzy_match(q, t).map_or(f64::NEG_INFINITY, |m| m.score)
    }
    // The contiguous run wins over scattered letters, wherever it sits.
    #[test]
    fn a_contiguous_run_outranks_scattered_letters() {
        let run = score("rgs", "Card: #13 CG-RGS · CuriousOS");
        let scattered = score("rgs", "Group: go to the previous");
        assert!(run > scattered, "{run} vs {scattered}");
        let m = fuzzy_match("rgs", "Card: #13 CG-RGS · CuriousOS").unwrap();
        assert_eq!(m.matches, vec![13, 14, 15]);
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
