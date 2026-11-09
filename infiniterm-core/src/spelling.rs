//! British and American spellings, folded to the American one for matching
//! (#679): the palette, the shortcuts panel and the settings completion find
//! "colour" when you type "color" and the other way round.
//!
//! Where it fits: `fuzzy.rs` runs `fold_map` over the text and `fold` over the
//! query when either holds one of the stems below, and keeps the better of the
//! two matches. A stem, not a whole word, so "colours", "maximised" and the
//! camelCase flattened inside a setting key ("hidelabelwhenmaximised") fold
//! too. Only the spellings this app's labels, keys and docs use are listed;
//! the scan test in `commands`' registry (`labels_hold_no_unfolded_british_spelling`)
//! fails when a new label brings another.
//!
//! No gpui, no state.

/// `(British stem, American stem)`, lowercase. A stem matches anywhere in the
/// text. Add a pair when the scan test names a word.
const STEMS: &[(&str, &str)] = &[
    ("colour", "color"),
    ("maximis", "maximiz"),
    ("minimis", "minimiz"),
    ("customis", "customiz"),
    ("organis", "organiz"),
    ("normalis", "normaliz"),
    ("initialis", "initializ"),
    ("serialis", "serializ"),
    ("synchronis", "synchroniz"),
    ("recognis", "recogniz"),
    ("capitalis", "capitaliz"),
    ("visualis", "visualiz"),
    ("centre", "center"),
    ("grey", "gray"),
    ("behaviour", "behavior"),
    ("favourite", "favorite"),
    ("licence", "license"),
    ("analyse", "analyze"),
];

/// Whether `lower` holds a British stem.
fn has_british_stem(lower: &str) -> bool {
    STEMS.iter().any(|(british, _)| lower.contains(british))
}

/// `lower` (already lowercase) with every British stem replaced by the
/// American one, and for each UTF-16 unit of the result the UTF-16 index of
/// the unit it came from in `lower`, so a match found on the folded text can
/// be highlighted on the original. A replaced stem maps position by position
/// (the American form is never longer), the rest of the text maps one to one.
pub fn fold_map(lower: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(lower.len());
    let mut map: Vec<usize> = Vec::with_capacity(lower.len());
    let mut at = 0usize;
    let mut rest = lower;
    while let Some(c) = rest.chars().next() {
        if let Some((british, american)) = STEMS.iter().find(|(b, _)| rest.starts_with(b)) {
            let last = british.len() - 1;
            let n = american.chars().count();
            for (k, ch) in american.chars().enumerate() {
                out.push(ch);
                // The last letter points at the last letter of the British
                // stem (colo-u-r: the r), the others one to one.
                map.push(at + if k + 1 == n { last } else { k.min(last) });
            }
            at += british.len();
            rest = &rest[british.len()..];
        } else {
            out.push(c);
            for j in 0..c.len_utf16() {
                map.push(at + j);
            }
            at += c.len_utf16();
            rest = &rest[c.len_utf8()..];
        }
    }
    (out, map)
}

/// The American spelling of `s`, lowercase.
pub fn fold(s: &str) -> String {
    fold_map(&s.to_lowercase()).0
}

/// Whether folding could change a match between `query` and `text` at all:
/// one of them holds a British stem. The quick exit for the common case.
pub fn may_differ(query: &str, text: &str) -> bool {
    has_british_stem(&query.to_lowercase()) || has_british_stem(&text.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn british_stems_fold_to_the_american_spelling() {
        assert_eq!(
            fold("Card: clear the state colour"),
            "card: clear the state color"
        );
        assert_eq!(fold("Maximise / restore"), "maximize / restore");
        assert_eq!(
            fold("ui.hideLabelWhenMaximised"),
            "ui.hidelabelwhenmaximized"
        );
        assert_eq!(fold("Centre, grey, behaviours"), "center, gray, behaviors");
        assert_eq!(fold("ift licence"), "ift license");
        // American text and plain text are left alone.
        assert_eq!(fold("terminal.cursorColor"), "terminal.cursorcolor");
        assert_eq!(fold("Exercise the promise"), "exercise the promise");
    }

    #[test]
    fn a_folded_position_points_back_into_the_original_text() {
        let (folded, map) = fold_map("window colour");
        assert_eq!(folded, "window color");
        assert_eq!(map.len(), folded.encode_utf16().count());
        // "color" starts at 7 in both; its last letter, r, is at 11 folded and
        // at 12 in the original (the u is skipped).
        assert_eq!(map[7], 7);
        assert_eq!(map[11], 12, "the last letter of the stem lines up");
        let (folded, map) = fold_map("colours");
        assert_eq!(folded, "colors");
        assert_eq!(
            map[5], 6,
            "the s after the stem lines up with the original s"
        );
    }

    #[test]
    fn only_a_text_with_a_stem_can_differ() {
        assert!(may_differ("color", "Window: change the colour"));
        assert!(may_differ("colour", "Window: change the color"));
        assert!(!may_differ("color", "Window: change the color"));
    }
}
