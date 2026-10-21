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

/// The shortest query the palette holds to a REAL match (#490). Shorter
/// ones are abbreviations (`rgs`, `ntc`) and keep the loose subsequence rule.
pub const STRICT_MIN_CHARS: usize = 4;

/// What a query that contains the text contiguously earns over any other
/// kind of match, so the rows that hold the word come first whatever the
/// length or the position bonuses say. Far below the palette's hint penalty
/// (1000) so a label match still outranks a hint match.
const CONTIGUOUS_TIER: f64 = 200.;

/// The palette's match (#490). For a query of `STRICT_MIN_CHARS` or more
/// letters a row shows only when the query matches as a REAL match:
/// - all the letters in one run anywhere in the text (the word is in the
///   label, even at the end of a long one), or
/// - the letters are initials: each one starts a word, or
/// - at most two runs of two letters or more, each starting a word
///   (`tidy cards` against `tidy the cards`).
///
/// Letters scattered over a long label (`check` in `tidy the cards into a
/// block`) or pieces that start in the middle of a word (`che` + `ck` in
/// `chess game stack`) do not count. A one-run match gets `CONTIGUOUS_TIER`.
/// A shorter query is `fuzzy_match` unchanged. Editor completion and the
/// shortcuts panel keep calling `fuzzy_match`: a prefix like `hmap` for
/// `HashMap` is a fuzzy match there on purpose.
pub fn strict_match(query: &str, text: &str) -> Option<Match> {
    let q: Vec<char> = query
        .trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
        .to_lowercase()
        .chars()
        .collect();
    if q.len() < STRICT_MIN_CHARS {
        return fuzzy_match(query, text);
    }
    let lower: Vec<u16> = text.to_lowercase().encode_utf16().collect();
    let needles: Vec<Vec<u16>> = q
        .iter()
        .map(|c| {
            let mut units = [0; 2];
            c.encode_utf16(&mut units).to_vec()
        })
        .collect();
    let starts: Vec<usize> = lower
        .windows(needles[0].len())
        .enumerate()
        .filter(|(_, w)| *w == needles[0].as_slice())
        .map(|(i, _)| i)
        .collect();
    let mut best: Option<Match> = None;
    for start in starts {
        let Some(mut m) = align_from(&lower, &needles, start) else {
            continue;
        };
        let pieces = pieces(&m.matches);
        if !is_real_match(&lower, &pieces) {
            continue;
        }
        if pieces.len() == 1 {
            m.score += CONTIGUOUS_TIER;
        }
        if best.as_ref().is_none_or(|b| m.score > b.score) {
            best = Some(m);
        }
    }
    best.map(|mut m| {
        m.score -= text.encode_utf16().count().min(60) as f64 / 10.;
        m
    })
}

/// The characters that end a word: space . - _ / : +.
const WORD_SEPARATORS: [u16; 7] = [32, 46, 45, 95, 47, 58, 43];

fn starts_word(lower: &[u16], at: usize) -> bool {
    at == 0 || WORD_SEPARATORS.contains(&lower[at - 1])
}

/// The runs of consecutive positions: (start, length) for each.
fn pieces(matches: &[usize]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = vec![];
    for &at in matches {
        match out.last_mut() {
            Some((start, len)) if *start + *len == at => *len += 1,
            _ => out.push((at, 1)),
        }
    }
    out
}

fn is_real_match(lower: &[u16], pieces: &[(usize, usize)]) -> bool {
    if pieces.len() == 1 {
        return true;
    }
    if !pieces.iter().all(|&(start, _)| starts_word(lower, start)) {
        return false;
    }
    // Initials, or a couple of whole pieces.
    pieces.iter().all(|&(_, len)| len == 1)
        || (pieces.len() <= 2 && pieces.iter().all(|&(_, len)| len >= 2))
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
    // #490: the screenshot's rows for `check`. (label, shown)
    const CHECK_TABLE: [(&str, bool); 7] = [
        ("App: check for updates", true),
        (
            "Card: #27 * The chord sheet is built and the tests pass. I haven't opened it in a browser, so the layout on a phone is unchecked",
            true,
        ),
        ("Canvas: tidy the cards into a block", false),
        ("Card: #8 3D mobile chess game stack · Personal Stuff", false),
        ("Window: change the title bar and Dock colour…", false),
        ("Card: mask with a decoy / unmask", false),
        ("Canvas: fit the selection, the group or the block", false),
    ];

    #[test]
    fn a_word_of_four_letters_finds_only_rows_that_hold_it() {
        for (label, shown) in CHECK_TABLE {
            assert_eq!(
                strict_match("check", label).is_some(),
                shown,
                "check against {label:?}"
            );
        }
    }

    #[test]
    fn the_word_at_the_end_of_a_long_label_is_found() {
        let label = CHECK_TABLE[1].0;
        let m = strict_match("check", label).unwrap();
        let at = label.find("unchecked").unwrap() + 2;
        assert_eq!(m.matches, (at..at + 5).collect::<Vec<_>>());
    }

    #[test]
    fn rows_that_hold_the_word_come_first() {
        let word_start = strict_match("check", "App: check for updates")
            .unwrap()
            .score;
        let mid_word = strict_match("check", "A very unchecked row").unwrap().score;
        // Two whole pieces that start words: real, but not the word itself.
        let pieces = strict_match("tidy cards", "Canvas: tidy the cards into a block").unwrap();
        assert!(word_start > mid_word, "{word_start} vs {mid_word}");
        assert!(mid_word > pieces.score, "{mid_word} vs {}", pieces.score);
    }

    #[test]
    fn two_whole_pieces_that_start_words_count() {
        assert!(strict_match("tidy cards", "Canvas: tidy the cards into a block").is_some());
        // The second piece starts in the middle of a word: no.
        assert!(strict_match("tidy ards", "Canvas: tidy the cards into a block").is_none());
        // Three pieces: no.
        assert!(strict_match("tidy ta bl", "Canvas: tidy the cards into a block").is_none());
    }

    #[test]
    fn four_initials_count() {
        assert!(strict_match("ntce", "New terminal card extra").is_some());
        // Scattered inside words: no.
        assert!(strict_match("ewmr", "New terminal card extra").is_none());
    }

    #[test]
    fn a_short_query_keeps_the_loose_rule() {
        // Three letters and fewer are abbreviations: unchanged.
        for (q, t) in [
            ("rgs", "Group: go to the previous"),
            ("ntc", "New terminal card"),
            ("cke", "Canvas: tidy the cards into a block"),
        ] {
            assert_eq!(strict_match(q, t), fuzzy_match(q, t), "{q} against {t}");
        }
        let run = strict_match("rgs", "Card: #13 CG-RGS · CuriousOS")
            .unwrap()
            .score;
        let scattered = strict_match("rgs", "Group: go to the previous")
            .unwrap()
            .score;
        assert!(run > scattered);
    }

    #[test]
    fn unicode_positions_match_utf16_reference() {
        assert_eq!(fuzzy_match("z", "🦀 Zoom").unwrap().matches, vec![3]);
        assert_eq!(fuzzy_match("ç", "Çalışma").unwrap().matches, vec![0]);
    }
}
