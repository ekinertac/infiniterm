//! Completion from the words already in the text: the simple kind that
//! editors had before LSP (Vim's Ctrl-N, Emacs `dabbrev`, Sublime's
//! default). No language knowledge beyond one hint: when the file has a
//! grammar, the editor passes the byte ranges of its comments and strings
//! (`skip`, from the tree-sitter spans), and words inside them are not
//! offered, so the list holds names, not the prose of a comment.
//!
//! Candidates are the words of the current text, nearest to the caret
//! first, then words from the card's other tabs, marked with the tab's
//! name. A prefix match ranks above letters-in-order (`gtl` finds
//! `getTotalLength`), which is only tried from `FUZZY_FROM` letters so two
//! letters do not match half the file.
//!
//! Pure over text. `complete.rs` holds the popup's types; the editor calls
//! `complete_words` when no schema provider applies
//! (`infiniterm-ui/src/editor_body.rs`).
use crate::complete::{Completion, Offer, MAX_ITEMS};
use crate::fuzzy::fuzzy_match;
use std::collections::HashMap;

/// Letters typed before the popup opens.
pub const MIN_PREFIX: usize = 2;
/// Letters from which in-order matching is tried as well as prefixes.
pub const FUZZY_FROM: usize = 3;
/// The shortest word offered: shorter ones are noise.
pub const MIN_WORD: usize = 3;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The words of `text` as (char offset, word): at least `MIN_WORD` chars,
/// not starting with a digit, not starting inside a `skip` byte range
/// (sorted by start).
pub fn words_in(text: &str, skip: &[(usize, usize)]) -> Vec<(usize, String)> {
    let mut out = vec![];
    let mut si = 0;
    let mut chars = text.char_indices().enumerate().peekable();
    while let Some((ci, (bi, c))) = chars.next() {
        if !is_word(c) {
            continue;
        }
        let mut word = String::from(c);
        while let Some((_, (_, n))) = chars.peek() {
            if !is_word(*n) {
                break;
            }
            word.push(*n);
            chars.next();
        }
        while si < skip.len() && skip[si].1 <= bi {
            si += 1;
        }
        let skipped = si < skip.len() && skip[si].0 <= bi;
        if !skipped && word.chars().count() >= MIN_WORD && !c.is_ascii_digit() {
            out.push((ci, word));
        }
    }
    out
}

/// Each distinct word once, for the word list a tab keeps for its siblings.
pub fn unique_words(text: &str, skip: &[(usize, usize)]) -> Vec<String> {
    let mut words: Vec<String> = words_in(text, skip).into_iter().map(|(_, w)| w).collect();
    words.sort_unstable();
    words.dedup();
    words
}

/// Another tab's words and the name to show beside them.
pub struct Other<'a> {
    pub name: &'a str,
    pub words: &'a [String],
}

pub fn complete_words(
    doc: &str,
    caret: usize,
    skip: &[(usize, usize)],
    others: &[Other],
) -> Option<Offer> {
    let chars: Vec<char> = doc.chars().collect();
    if chars.get(caret).is_some_and(|c| is_word(*c)) {
        return None;
    }
    let mut start = caret.min(chars.len());
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let typed = caret.min(chars.len()) - start;
    if typed < MIN_PREFIX || chars[start].is_ascii_digit() {
        return None;
    }
    let prefix: String = chars[start..caret].iter().collect();
    let lower = prefix.to_lowercase();

    // word -> (distance from the caret, the tab it came from if not this text)
    let mut found: HashMap<String, (usize, Option<&str>)> = HashMap::new();
    for (off, word) in words_in(doc, skip) {
        if off == start || word == prefix {
            continue;
        }
        let d = off.abs_diff(caret);
        found
            .entry(word)
            .and_modify(|e| e.0 = e.0.min(d))
            .or_insert((d, None));
    }
    for o in others {
        for w in o.words {
            if *w != prefix {
                found.entry(w.clone()).or_insert((usize::MAX, Some(o.name)));
            }
        }
    }

    // (group, fuzzy score, distance, word, tab): prefix matches first.
    let mut ranked: Vec<(u8, f64, usize, String, Option<&str>)> = found
        .into_iter()
        .filter_map(|(w, (d, tab))| {
            if w.to_lowercase().starts_with(&lower) {
                Some((0, 0., d, w, tab))
            } else if typed >= FUZZY_FROM {
                fuzzy_match(&prefix, &w).map(|m| (1, m.score, d, w, tab))
            } else {
                None
            }
        })
        .collect();
    ranked.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(b.1.total_cmp(&a.1))
            .then(a.2.cmp(&b.2))
            .then(a.3.cmp(&b.3))
    });
    let items: Vec<Completion> = ranked
        .into_iter()
        .take(MAX_ITEMS)
        .map(|(_, _, _, w, tab)| Completion {
            insert: w.clone(),
            label: w,
            detail: tab.unwrap_or("").to_string(),
            doc: String::new(),
        })
        .collect();
    (!items.is_empty()).then_some(Offer {
        typed,
        items,
        enter_accepts: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_end(doc: &str, others: &[Other]) -> Option<Offer> {
        complete_words(doc, doc.chars().count(), &[], others)
    }

    fn labels(o: &Offer) -> Vec<&str> {
        o.items.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    fn words_are_found_with_their_char_offsets_and_short_or_numeric_ones_are_not() {
        let w = words_in("ab total_sum 42x foo.bar", &[]);
        assert_eq!(
            w,
            vec![
                (3, "total_sum".into()),
                (17, "foo".into()),
                (21, "bar".into())
            ]
        );
        // Turkish letters are letters.
        assert_eq!(words_in("şeker ağaç", &[]).len(), 2);
    }

    #[test]
    fn words_inside_skipped_ranges_are_left_out() {
        let text = "let alpha = 1; // beta gamma\nlet delta";
        let start = text.find("//").unwrap();
        let end = text.find('\n').unwrap();
        let w: Vec<String> = words_in(text, &[(start, end)])
            .into_iter()
            .map(|(_, w)| w)
            .collect();
        assert_eq!(w, vec!["let", "alpha", "let", "delta"]);
    }

    #[test]
    fn nothing_before_two_letters_in_the_middle_of_a_word_or_after_a_digit_start() {
        assert!(at_end("total\nt", &[]).is_none());
        assert!(
            complete_words("total totalx", 8, &[], &[]).is_none(),
            "caret inside a word"
        );
        assert!(at_end("total\n9t", &[]).is_none());
    }

    #[test]
    fn prefix_matches_come_nearest_first_and_the_word_being_typed_is_not_offered() {
        let doc = "total_far\n\n\n\n\n\n\n\ntotal_near\nuse total";
        let o = at_end(doc, &[]).unwrap();
        assert_eq!(o.typed, 5);
        assert_eq!(labels(&o), vec!["total_near", "total_far"]);
        assert!(!o.enter_accepts);
    }

    #[test]
    fn case_does_not_matter_for_a_prefix_and_an_exact_repeat_is_not_offered() {
        let o = at_end("TotalSum\ntotal", &[]).unwrap();
        assert_eq!(labels(&o), vec!["TotalSum"]);
        assert!(at_end("total\ntotal", &[]).is_none());
    }

    #[test]
    fn letters_in_order_match_from_three_letters_and_rank_after_prefixes() {
        let doc = "getTotalLength\ngtlast\ngtl";
        let o = at_end(doc, &[]).unwrap();
        assert_eq!(labels(&o), vec!["gtlast", "getTotalLength"]);
        // Two letters are a prefix only.
        assert!(at_end("getTotalLength\ngt", &[]).is_none());
    }

    #[test]
    fn other_tabs_add_their_words_with_the_tab_name_and_never_repeat_one() {
        let other = vec!["configure".to_string(), "context".to_string()];
        let others = [Other {
            name: "main.rs",
            words: &other,
        }];
        let o = at_end("contents\ncon", &others).unwrap();
        // This text's word first, then the other tab's, alphabetical.
        assert_eq!(labels(&o), vec!["contents", "configure", "context"]);
        assert_eq!(o.items[0].detail, "");
        assert_eq!(o.items[1].detail, "main.rs");
        let dup = vec!["contents".to_string()];
        let o = at_end(
            "contents\ncon",
            &[Other {
                name: "x",
                words: &dup,
            }],
        )
        .unwrap();
        assert_eq!(o.items.len(), 1);
        assert_eq!(o.items[0].detail, "");
    }

    #[test]
    fn unique_words_are_sorted_and_deduplicated() {
        assert_eq!(
            unique_words("beta alpha beta gamma", &[]),
            vec!["alpha", "beta", "gamma"]
        );
    }
}
