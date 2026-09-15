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
    let hay: Vec<char> = if sensitive {
        text.chars().collect()
    } else {
        text.chars().flat_map(|c| c.to_lowercase()).collect()
    };
    let needle: Vec<char> = if sensitive {
        query.chars().collect()
    } else {
        query.chars().flat_map(|c| c.to_lowercase()).collect()
    };
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
        assert_eq!(find_all("Foo foo FOO", "foo"), vec![(0, 3), (4, 7), (8, 11)]);
        assert_eq!(find_all("Foo foo FOO", "Foo"), vec![(0, 3)]);
        assert!(find_all("abc", "").is_empty());
    }

    #[test]
    fn positions_are_chars_and_matches_do_not_overlap() {
        assert_eq!(find_all("héé aa aaa", "aa"), vec![(4, 6), (7, 9)]);
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
