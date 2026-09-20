//! Find and replace over the buffer's text: what `@codemirror/search` did
//! for the reference. Matching is literal and, as CodeMirror's default,
//! case-insensitive unless the query has an upper-case letter; every match
//! is listed so the view can light them all and step through the current
//! one. Positions are char indices, the buffer's unit.

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
}

fn smart_case(query: &str) -> bool {
    query.chars().any(char::is_uppercase)
}

/// All literal matches of `query` in `text`, as char ranges.
pub fn find_all(text: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return vec![];
    }
    let sensitive = smart_case(query);
    // One char in, one char out: `to_lowercase` can return two ('İ' gives
    // 'i' plus a combining dot), and a hay that grew would put every
    // match after it at the wrong index. Ekin writes Turkish.
    let fold = |c: char| {
        if sensitive {
            c
        } else {
            c.to_lowercase().next().unwrap_or(c)
        }
    };
    let hay: Vec<char> = text.chars().map(fold).collect();
    let needle: Vec<char> = query.chars().map(fold).collect();
    let mut out = vec![];
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if hay[i..i + needle.len()] == needle[..] {
            out.push((i, i + needle.len()));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

impl Search {
    /// Re-runs the query over `text` and keeps the current match on or after
    /// `from`, the caret.
    pub fn refresh(&mut self, text: &str, from: usize) {
        self.matches = find_all(text, &self.query);
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
