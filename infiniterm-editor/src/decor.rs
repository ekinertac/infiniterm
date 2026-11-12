//! What the editor paints over the text that is not syntax (#778): the
//! indent guides, the other occurrences of the word under the caret, and
//! which symbol headers sticky scroll pins at the top of the card. Every
//! one of them is opt-in (`editor.indentGuides`, `editor.highlightOccurrences`,
//! `editor.stickyScroll`), so a default editor computes none of this.
//!
//! Called by `infiniterm-ui/src/editor_body.rs` while it paints the visible
//! rows. Pure: lines in, columns out. Columns are CHARS, as the painter
//! places everything (`cell_w` per char); a line indented with tabs gets no
//! guides, since a tab is drawn wider than one cell and a guide would sit
//! in the wrong place.
use crate::symbols::Symbol;
use std::ops::Range;

/// Lines looked at to guess a file's indent width: enough for any real file
/// to show its indent, cheap enough to redo on every edit.
pub const UNIT_SAMPLE_LINES: usize = 500;
/// The narrowest and widest indent width taken from a file. One space is a
/// continuation or alignment, never a level; past 8 nothing nests.
const UNIT_MIN: usize = 2;
const UNIT_MAX: usize = 8;
/// How far a blank line looks up and down for the indent of its neighbours.
/// A blank run longer than this gets no guides, which is the safe mistake.
const BLANK_REACH: usize = 200;
/// Longest text the occurrence highlight looks for: a long selection is a
/// passage, not something repeated.
const OCCURRENCE_MAX_CHARS: usize = 200;

/// The leading spaces of `line`, or `None` for a blank line or one whose
/// indent has a tab in it.
pub fn indent_of(line: &str) -> Option<usize> {
    if line.trim().is_empty() {
        return None;
    }
    let lead: String = line.chars().take_while(|c| c.is_whitespace()).collect();
    (!lead.contains('\t')).then(|| lead.chars().count())
}

/// The file's indent width: the smallest nonzero leading-space count in the
/// first lines that is at least `UNIT_MIN`, else `fallback`.
pub fn indent_unit<'a>(lines: impl Iterator<Item = &'a str>, fallback: usize) -> usize {
    lines
        .take(UNIT_SAMPLE_LINES)
        .filter_map(indent_of)
        .filter(|&n| n >= UNIT_MIN)
        .min()
        .map_or(fallback, |n| n.min(UNIT_MAX))
}

/// The indent a line's guides are drawn to. A text line uses its own; a
/// blank line takes the smaller of the nearest text lines above and below,
/// so a guide runs through the blank lines inside a block and stops at its
/// end. `line_at` returns `None` past the end of the file.
pub fn guide_depth(line: usize, line_at: impl Fn(usize) -> Option<String>) -> usize {
    let Some(text) = line_at(line) else { return 0 };
    if let Some(n) = indent_of(&text) {
        return n;
    }
    if !text.trim().is_empty() {
        return 0; // a tab indent
    }
    let near = |step: isize| -> usize {
        let mut l = line as isize;
        for _ in 0..BLANK_REACH {
            l += step;
            if l < 0 {
                return 0;
            }
            match line_at(l as usize) {
                None => return 0,
                Some(t) if t.trim().is_empty() => continue,
                Some(t) => return indent_of(&t).unwrap_or(0),
            }
        }
        0
    };
    near(-1).min(near(1))
}

/// The guide columns for a line at `depth`: one per indent level strictly
/// left of the text (`0`, `unit`, `2 * unit`, ...).
pub fn guide_columns(depth: usize, unit: usize) -> impl Iterator<Item = usize> {
    (0..depth).step_by(unit.max(1))
}

/// The caret's guide: the level its block hangs from, and the lines it
/// runs over, so the painter can draw that one brighter. Starting from the
/// caret's line (or, on a blank line, its depth), the block is every line
/// around it that is blank or indented at least as deep.
pub fn active_guide(
    caret_line: usize,
    unit: usize,
    line_count: usize,
    line_at: impl Fn(usize) -> Option<String> + Copy,
) -> Option<(usize, Range<usize>)> {
    let depth = guide_depth(caret_line, line_at);
    if depth == 0 || unit == 0 {
        return None;
    }
    let col = (depth - 1) / unit * unit;
    let inside = |l: usize| guide_depth(l, line_at) > col;
    let mut start = caret_line;
    while start > 0 && inside(start - 1) {
        start -= 1;
    }
    let mut end = caret_line + 1;
    while end < line_count && inside(end) {
        end += 1;
    }
    Some((col, start..end))
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The word the caret touches in `line` at char column `col` (inside it, or
/// just after its last letter), as a char range.
pub fn word_at(line: &str, col: usize) -> Option<Range<usize>> {
    let chars: Vec<char> = line.chars().collect();
    let at = |i: usize| chars.get(i).copied().is_some_and(is_word);
    let mid = if at(col) {
        col
    } else if col > 0 && at(col - 1) {
        col - 1
    } else {
        return None;
    };
    let mut a = mid;
    while a > 0 && at(a - 1) {
        a -= 1;
    }
    let mut b = mid + 1;
    while at(b) {
        b += 1;
    }
    Some(a..b)
}

/// Where `needle` starts in `line`, as char columns. `whole_word` skips a
/// match with a word character on either side, which is what the word
/// under the caret wants; a selected passage matches anywhere.
pub fn occurrences(line: &str, needle: &str, whole_word: bool) -> Vec<usize> {
    let n = needle.chars().count();
    if n == 0 || n > OCCURRENCE_MAX_CHARS || needle.trim().is_empty() {
        return vec![];
    }
    let hay: Vec<char> = line.chars().collect();
    let pat: Vec<char> = needle.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i + n <= hay.len() {
        if hay[i..i + n] == pat[..] {
            let bounded = !whole_word
                || ((i == 0 || !is_word(hay[i - 1]))
                    && hay.get(i + n).is_none_or(|c| !is_word(*c)));
            if bounded {
                out.push(i);
                i += n;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The header lines (0-based) sticky scroll pins over the top of the text
/// when `first` is the first line shown: every symbol that starts above it
/// and is still open at it, outermost first, at most `max` of them (the
/// innermost kept). A symbol with no end (`end_line` 0) is skipped.
pub fn sticky_headers(symbols: &[Symbol], first: usize, max: usize) -> Vec<usize> {
    let mut lines: Vec<usize> = symbols
        .iter()
        .filter(|s| s.end_line > 0)
        .map(|s| (s.line - 1, s.end_line - 1))
        .filter(|&(start, end)| start < first && end > first)
        .map(|(start, _)| start)
        .collect();
    lines.sort_unstable();
    lines.dedup();
    let skip = lines.len().saturating_sub(max);
    lines.split_off(skip)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at<'a>(lines: &'a [&'a str]) -> impl Fn(usize) -> Option<String> + Copy + 'a {
        move |i| lines.get(i).map(|s| s.to_string())
    }

    #[test]
    fn indent_ignores_blank_and_tab_lines() {
        assert_eq!(indent_of("    x"), Some(4));
        assert_eq!(indent_of("x"), Some(0));
        assert_eq!(indent_of("   "), None);
        assert_eq!(indent_of("\tx"), None);
    }

    #[test]
    fn unit_is_the_smallest_real_indent() {
        let src = ["fn a() {", "    if x {", "        y", "    }", "}"];
        assert_eq!(indent_unit(src.iter().copied(), 2), 4);
        let two = ["a:", "  b:", "    c: 1"];
        assert_eq!(indent_unit(two.iter().copied(), 4), 2);
        // A one-space continuation is not a level.
        let odd = ["/**", " * doc", " */"];
        assert_eq!(indent_unit(odd.iter().copied(), 2), 2);
    }

    #[test]
    fn a_blank_line_takes_the_shallower_neighbour() {
        let src = ["fn a() {", "    x;", "", "    y;", "}", "", "fn b() {}"];
        assert_eq!(guide_depth(2, at(&src)), 4);
        assert_eq!(guide_depth(5, at(&src)), 0);
        assert_eq!(
            guide_columns(8, 4).collect::<Vec<_>>(),
            vec![0, 4],
            "a guide per level, left of the text"
        );
    }

    #[test]
    fn the_active_guide_spans_the_carets_block() {
        let src = [
            "fn a() {",
            "    if x {",
            "        y;",
            "",
            "        z;",
            "    }",
            "}",
        ];
        assert_eq!(active_guide(2, 4, src.len(), at(&src)), Some((4, 2..5)));
        assert_eq!(active_guide(1, 4, src.len(), at(&src)), Some((0, 1..6)));
        assert_eq!(active_guide(0, 4, src.len(), at(&src)), None);
    }

    #[test]
    fn the_word_under_the_caret_includes_just_after_its_end() {
        assert_eq!(word_at("let foo_bar = 1", 5), Some(4..11));
        assert_eq!(word_at("let foo_bar = 1", 11), Some(4..11));
        assert_eq!(word_at("let foo_bar = 1", 12), None);
        assert_eq!(word_at("", 0), None);
    }

    #[test]
    fn occurrences_respect_word_bounds_only_when_asked() {
        assert_eq!(occurrences("foo foobar foo", "foo", true), vec![0, 11]);
        assert_eq!(occurrences("foo foobar foo", "foo", false), vec![0, 4, 11]);
        assert_eq!(occurrences("aaa", "aa", false), vec![0]);
        assert!(occurrences("a b", " ", false).is_empty());
    }

    fn sym(line: usize, end_line: usize) -> Symbol {
        Symbol {
            name: String::new(),
            kind: "fn",
            line,
            end_line,
            depth: 0,
        }
    }

    #[test]
    fn sticky_headers_are_the_open_symbols_above_the_top_line() {
        // impl at lines 1..20, fn at 3..10, fn at 12..18 (1-based).
        let s = [sym(1, 20), sym(3, 10), sym(12, 18)];
        assert_eq!(sticky_headers(&s, 5, 3), vec![0, 2]);
        assert_eq!(sticky_headers(&s, 14, 3), vec![0, 11]);
        assert_eq!(sticky_headers(&s, 0, 3), Vec::<usize>::new());
        assert_eq!(sticky_headers(&s, 5, 1), vec![2], "the innermost kept");
        // The last line of a symbol has nothing left to pin it for.
        assert_eq!(sticky_headers(&s, 9, 3), vec![0]);
    }
}
