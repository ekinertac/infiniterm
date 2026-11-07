//! Find and replace over the buffer's text: what `@codemirror/search` did
//! for the reference, plus the toggles an editor user expects (#602).
//! Matching is literal by default and, as CodeMirror's default,
//! case-insensitive unless the query has an upper-case letter (smart case);
//! `match_case` forces exact case, `whole_word` wraps the pattern in word
//! boundaries and `regex` reads the query as a regular expression, with
//! `$1` / `${name}` and `\n` `\t` in the replacement. A `scope` limits
//! matches to a char range (replace in selection). Every match is listed so
//! the view can light them all and step through the current one. Positions
//! are char indices, the buffer's unit, whatever the regex crate counts in.

use regex::{Regex, RegexBuilder};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Search {
    pub query: String,
    pub replacement: String,
    /// Every match, as char ranges, in order.
    pub matches: Vec<(usize, usize)>,
    /// The match the caret is on.
    pub current: Option<usize>,
    /// The replace row is showing (Cmd+Option+F, or Tab from the query).
    pub replacing: bool,
    /// The query is a regular expression.
    pub regex: bool,
    /// Exact case, instead of smart case.
    pub match_case: bool,
    /// Only whole words match.
    pub whole_word: bool,
    /// Only matches inside this char range count (replace in selection).
    pub scope: Option<(usize, usize)>,
    /// Why the query is not a valid pattern, for the panel.
    pub error: Option<String>,
}

/// The toggles that change what a query means.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Options {
    pub regex: bool,
    pub match_case: bool,
    pub whole_word: bool,
}

/// Smart case: an upper-case letter in the query makes it case-sensitive.
/// In a regex, a letter after a backslash is an escape (`\S`, `\D`), not
/// something the user typed in capitals.
fn smart_case(query: &str, regex: bool) -> bool {
    let mut escaped = false;
    for c in query.chars() {
        if regex && escaped {
            escaped = false;
        } else if regex && c == '\\' {
            escaped = true;
        } else if c.is_uppercase() {
            return true;
        }
    }
    false
}

/// The compiled pattern for `query`, or why it does not compile. Public so
/// a search over many files (`grep.rs`) compiles once, not once per file.
pub fn build(query: &str, o: Options) -> Result<Regex, String> {
    let mut pattern = if o.regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    if o.whole_word {
        pattern = format!(r"\b(?:{pattern})\b");
    }
    RegexBuilder::new(&pattern)
        .case_insensitive(!(o.match_case || smart_case(query, o.regex)))
        // ^ and $ are per line, as in every editor's find.
        .multi_line(true)
        .build()
        .map_err(|e| {
            // The crate's message spans several lines; the panel has one.
            let text = e.to_string();
            text.lines()
                .last()
                .unwrap_or("invalid pattern")
                .trim_start_matches("error: ")
                .to_string()
        })
}

/// Byte offsets to char offsets, walked forward so a list of matches costs
/// one pass over the text, not one per match.
struct CharIndex<'a> {
    text: &'a str,
    byte: usize,
    chars: usize,
}

impl<'a> CharIndex<'a> {
    fn new(text: &'a str) -> Self {
        CharIndex {
            text,
            byte: 0,
            chars: 0,
        }
    }

    /// The char index of `byte`, which must not be before the last one asked.
    fn at(&mut self, byte: usize) -> usize {
        self.chars += self.text[self.byte..byte].chars().count();
        self.byte = byte;
        self.chars
    }
}

fn byte_of(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map_or(text.len(), |(b, _)| b)
}

fn in_scope(scope: Option<(usize, usize)>, a: usize, b: usize) -> bool {
    scope.is_none_or(|(s, e)| a >= s && b <= e)
}

/// All matches of `query` in `text` under `o`, as char ranges. An empty
/// query has none; an invalid pattern is an error, not a panic.
pub fn find_matches(
    text: &str,
    query: &str,
    o: Options,
    scope: Option<(usize, usize)>,
) -> Result<Vec<(usize, usize)>, String> {
    if query.is_empty() {
        return Ok(vec![]);
    }
    let re = build(query, o)?;
    let mut idx = CharIndex::new(text);
    let mut out = vec![];
    for m in re.find_iter(text) {
        // A pattern like `a*` also matches the empty string everywhere;
        // there is nothing to light or replace there.
        if m.is_empty() {
            continue;
        }
        let (a, b) = (idx.at(m.start()), idx.at(m.end()));
        if in_scope(scope, a, b) {
            out.push((a, b));
        }
    }
    Ok(out)
}

/// All literal matches of `query` in `text`, as char ranges.
pub fn find_all(text: &str, query: &str) -> Vec<(usize, usize)> {
    find_matches(text, query, Options::default(), None).unwrap_or_default()
}

/// `\n`, `\t` and `\\` in a regex replacement, as the characters they name.
/// Done before `$` expansion, which leaves backslashes alone.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// What each match becomes, as `(start, end, new text)` in char indices,
/// for replace all. In regex mode `$1`, `${name}` and `\n` expand; a
/// literal search inserts the replacement as typed.
pub fn replace_edits(
    text: &str,
    query: &str,
    replacement: &str,
    o: Options,
    scope: Option<(usize, usize)>,
) -> Result<Vec<(usize, usize, String)>, String> {
    if query.is_empty() {
        return Ok(vec![]);
    }
    let re = build(query, o)?;
    let template = o.regex.then(|| unescape(replacement));
    let mut idx = CharIndex::new(text);
    let mut out = vec![];
    for caps in re.captures_iter(text) {
        let m = caps.get(0).expect("group 0 is the whole match");
        if m.is_empty() {
            continue;
        }
        let (a, b) = (idx.at(m.start()), idx.at(m.end()));
        if !in_scope(scope, a, b) {
            continue;
        }
        let new = match &template {
            Some(t) => {
                let mut dst = String::new();
                caps.expand(t, &mut dst);
                dst
            }
            None => replacement.to_string(),
        };
        out.push((a, b, new));
    }
    Ok(out)
}

/// What the single match at `range` becomes: the same expansion as
/// `replace_edits`, for the one the caret is on.
pub fn replacement_at(
    text: &str,
    query: &str,
    replacement: &str,
    o: Options,
    range: (usize, usize),
) -> Result<String, String> {
    if !o.regex {
        return Ok(replacement.to_string());
    }
    let re = build(query, o)?;
    let start = byte_of(text, range.0);
    let template = unescape(replacement);
    // The first match at or after the start is this one: matches are
    // leftmost and the list was made by the same pattern.
    let Some(caps) = re.captures_at(text, start).filter(|c| {
        c.get(0)
            .is_some_and(|m| m.start() == start && !m.is_empty())
    }) else {
        return Ok(template);
    };
    let mut dst = String::new();
    caps.expand(&template, &mut dst);
    Ok(dst)
}

impl Search {
    pub fn options(&self) -> Options {
        Options {
            regex: self.regex,
            match_case: self.match_case,
            whole_word: self.whole_word,
        }
    }

    /// Re-runs the query over `text` and keeps the current match on or after
    /// `from`, the caret.
    pub fn refresh(&mut self, text: &str, from: usize) {
        match find_matches(text, &self.query, self.options(), self.scope) {
            Ok(matches) => {
                self.matches = matches;
                self.error = None;
            }
            Err(e) => {
                self.matches = vec![];
                self.error = Some(e);
            }
        }
        self.current = if self.matches.is_empty() {
            None
        } else {
            Some(
                self.matches
                    .iter()
                    .position(|(s, _)| *s >= from)
                    .unwrap_or(0),
            )
        };
    }

    /// Every match with what it becomes, for replace all.
    pub fn edits(&self, text: &str) -> Vec<(usize, usize, String)> {
        replace_edits(
            text,
            &self.query,
            &self.replacement,
            self.options(),
            self.scope,
        )
        .unwrap_or_default()
    }

    /// The current match with what it becomes, for replace.
    pub fn current_edit(&self, text: &str) -> Option<(usize, usize, String)> {
        let (a, b) = self.current_range()?;
        let new =
            replacement_at(text, &self.query, &self.replacement, self.options(), (a, b)).ok()?;
        Some((a, b, new))
    }

    pub fn next(&mut self) {
        if let (Some(i), n) = (self.current, self.matches.len()) {
            self.current = Some((i + 1) % n);
        }
    }

    pub fn prev(&mut self) {
        if let (Some(i), n) = (self.current, self.matches.len()) {
            self.current = Some((i + n - 1) % n);
        }
    }

    pub fn current_range(&self) -> Option<(usize, usize)> {
        self.matches.get(self.current?).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RE: Options = Options {
        regex: true,
        match_case: false,
        whole_word: false,
    };

    #[test]
    fn lower_case_queries_ignore_case_and_upper_case_ones_do_not() {
        assert_eq!(
            find_all("Foo foo FOO", "foo"),
            vec![(0, 3), (4, 7), (8, 11)]
        );
        assert_eq!(find_all("Foo foo FOO", "Foo"), vec![(0, 3)]);
        assert!(find_all("abc", "").is_empty());
    }

    #[test]
    fn positions_are_chars_and_matches_do_not_overlap() {
        assert_eq!(find_all("héé aa aaa", "aa"), vec![(4, 6), (7, 9)]);
    }

    // A capital dotted I lowercases to two chars; a match after one must
    // still be reported where it is in the text.
    #[test]
    fn a_turkish_capital_before_the_match_does_not_shift_it() {
        assert_eq!(find_all("İstanbul ve ankara", "ankara"), vec![(12, 18)]);
        assert_eq!(find_all("İİİ x", "x"), vec![(4, 5)]);
    }

    #[test]
    fn a_literal_query_is_not_a_pattern() {
        assert_eq!(find_all("a.c abc", "a.c"), vec![(0, 3)]);
        assert_eq!(find_all("f(x) f(y)", "f(x)"), vec![(0, 4)]);
    }

    #[test]
    fn match_case_makes_a_lower_case_query_exact() {
        let o = Options {
            match_case: true,
            ..Default::default()
        };
        assert_eq!(
            find_matches("Foo foo", "foo", o, None).unwrap(),
            vec![(4, 7)]
        );
    }

    #[test]
    fn whole_word_skips_matches_inside_other_words() {
        let o = Options {
            whole_word: true,
            ..Default::default()
        };
        assert_eq!(
            find_matches("cat concat cat.", "cat", o, None).unwrap(),
            vec![(0, 3), (11, 14)]
        );
        // The word toggle applies to a literal with punctuation too.
        assert_eq!(
            find_matches("a.b a.bc", "a.b", o, None).unwrap(),
            vec![(0, 3)]
        );
    }

    #[test]
    fn a_regex_matches_per_line_and_counts_chars() {
        assert_eq!(
            find_matches("é1\n22\nx", r"^\d+$", RE, None).unwrap(),
            vec![(3, 5)]
        );
        assert_eq!(
            find_matches("é12 3", r"\d+", RE, None).unwrap(),
            vec![(1, 3), (4, 5)]
        );
    }

    // `\S` is an escape, not a capital the user typed, so it must not turn
    // smart case on.
    #[test]
    fn an_escape_letter_does_not_make_a_regex_case_sensitive() {
        assert_eq!(
            find_matches("ab AB", r"\Sb", RE, None).unwrap(),
            vec![(0, 2), (3, 5)]
        );
        assert_eq!(find_matches("ab AB", "AB", RE, None).unwrap(), vec![(3, 5)]);
    }

    #[test]
    fn empty_matches_are_not_listed_and_a_bad_pattern_is_an_error() {
        assert_eq!(find_matches("baab", "a*", RE, None).unwrap(), vec![(1, 3)]);
        assert!(find_matches("x", "(", RE, None).is_err());
        let mut s = Search {
            query: "(".into(),
            regex: true,
            ..Default::default()
        };
        s.refresh("x", 0);
        assert!(s.error.is_some() && s.matches.is_empty());
        s.regex = false;
        s.refresh("(", 0);
        assert!(s.error.is_none());
        assert_eq!(s.matches, vec![(0, 1)]);
    }

    #[test]
    fn a_scope_keeps_only_matches_inside_it() {
        assert_eq!(
            find_matches("x x x x", "x", Options::default(), Some((2, 5))).unwrap(),
            vec![(2, 3), (4, 5)]
        );
    }

    #[test]
    fn regex_replacements_expand_groups_and_escapes() {
        let edits =
            replace_edits("2024-11-06", r"(\d+)-(\d+)-(\d+)", "$3/$2/$1", RE, None).unwrap();
        assert_eq!(edits, vec![(0, 10, "06/11/2024".to_string())]);
        let edits = replace_edits("a, b", ", ", r"\n", RE, None).unwrap();
        assert_eq!(edits, vec![(1, 3, "\n".to_string())]);
        // A literal search inserts what was typed, `$1` and all.
        let edits = replace_edits("a b", "a", "$1\\n", Options::default(), None).unwrap();
        assert_eq!(edits, vec![(0, 1, "$1\\n".to_string())]);
    }

    #[test]
    fn replace_edits_respect_the_scope_and_char_positions() {
        let edits = replace_edits("é a a a", "a", "b", Options::default(), Some((2, 5))).unwrap();
        assert_eq!(
            edits,
            vec![(2, 3, "b".to_string()), (4, 5, "b".to_string())]
        );
    }

    #[test]
    fn the_current_match_expands_its_own_groups() {
        let mut s = Search {
            query: r"(\w)=(\d)".into(),
            replacement: "$2=$1".into(),
            regex: true,
            ..Default::default()
        };
        let text = "a=1 b=2";
        s.refresh(text, 4);
        assert_eq!(s.current_edit(text), Some((4, 7, "2=b".to_string())));
        assert_eq!(s.edits(text).len(), 2);
    }

    #[test]
    fn the_current_match_starts_at_the_caret_and_wraps() {
        let mut s = Search {
            query: "x".into(),
            ..Default::default()
        };
        s.refresh("x.x.x", 2);
        assert_eq!(s.current_range(), Some((2, 3)));
        s.next();
        s.next();
        assert_eq!(s.current_range(), Some((0, 1)));
        s.prev();
        assert_eq!(s.current_range(), Some((4, 5)));
        s.refresh("x.x.x", 99);
        assert_eq!(s.current_range(), Some((0, 1)));
        s.refresh("none", 0);
        assert_eq!(s.current, None);
    }
}
