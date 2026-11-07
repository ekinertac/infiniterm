//! Find and replace across files (#602): the text side of Cmd+Shift+F. One
//! compiled pattern (`search::build`, so smart case, whole word and regex
//! mean here what they mean in the find panel) runs over each file's text and
//! lists the matching lines, or rewrites the file's text.
//!
//! Called by `infiniterm-ui/src/find_in_files.rs`, which lists the files and
//! reads and writes them through a `FileSource`. Pure: no disk, so the tests
//! need none. Lines are 1-based, as the editor's go-to-line and a terminal's
//! `path:42` are.

use crate::search::{self, Options};
use regex::Regex;

/// The most hits one search lists: a palette of more is not read, and a
/// pattern that matches every line of a project has told you what it means.
pub const MAX_HITS: usize = 1_000;

/// The longest a hit's line is shown; a minified file is one huge line.
const MAX_LINE_CHARS: usize = 200;

/// A file this big is not searched: it is a log or a dump, not source.
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

/// One matching line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// The file, relative to the searched folder.
    pub path: String,
    pub line: usize,
    /// The line, trimmed and cut for a list.
    pub text: String,
}

/// Whether `bytes` look like a binary file: a NUL in the first 8 KiB, which
/// is what git and grep go by.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

fn shown(line: &str) -> String {
    let line = line.trim();
    if line.chars().count() <= MAX_LINE_CHARS {
        line.to_string()
    } else {
        let cut: String = line.chars().take(MAX_LINE_CHARS).collect();
        format!("{cut}…")
    }
}

/// The lines of `text` that `re` matches, appended to `out` until it holds
/// `MAX_HITS`; true when the cap was reached.
pub fn search_text(re: &Regex, path: &str, text: &str, out: &mut Vec<Hit>) -> bool {
    for (i, line) in text.lines().enumerate() {
        // `^` and `$` are per line in the pattern; a line is searched alone
        // so a match can never run across a newline into the next one. An
        // empty match (`a*`) is not a hit, but a later real one on the same
        // line is.
        if re.find_iter(line).any(|m| !m.is_empty()) {
            out.push(Hit {
                path: path.to_string(),
                line: i + 1,
                text: shown(line),
            });
            if out.len() >= MAX_HITS {
                return true;
            }
        }
    }
    false
}

/// `text` with every match of `query` replaced, and how many there were;
/// `None` when nothing matched. The replacement means what it means in the
/// find panel (`$1` and `\n` in regex mode, as typed otherwise).
pub fn replace_text(
    text: &str,
    query: &str,
    replacement: &str,
    o: Options,
) -> Result<Option<(String, usize)>, String> {
    let edits = search::replace_edits(text, query, replacement, o, None)?;
    if edits.is_empty() {
        return Ok(None);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    let mut at = 0;
    for (a, b, new) in &edits {
        out.extend(chars.by_ref().take(a - at));
        out.push_str(new);
        // Skip what the match covered.
        chars.by_ref().take(b - a).for_each(drop);
        at = *b;
    }
    out.extend(chars);
    Ok(Some((out, edits.len())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn re(q: &str, regex: bool) -> Regex {
        search::build(
            q,
            Options {
                regex,
                ..Default::default()
            },
        )
        .unwrap()
    }

    #[test]
    fn lines_are_listed_one_based_once_each_with_the_text_trimmed() {
        let mut hits = vec![];
        let text = "  foo bar foo\nnothing\nFoo\r\nlast foo";
        assert!(!search_text(&re("foo", false), "a.rs", text, &mut hits));
        assert_eq!(
            hits.iter()
                .map(|h| (h.line, h.text.as_str()))
                .collect::<Vec<_>>(),
            vec![(1, "foo bar foo"), (3, "Foo"), (4, "last foo")]
        );
        assert_eq!(hits[0].path, "a.rs");
    }

    #[test]
    fn a_regex_is_per_line_and_an_empty_match_is_not_a_hit() {
        let mut hits = vec![];
        search_text(&re(r"^\d+$", true), "n", "1\nx2\n33", &mut hits);
        assert_eq!(hits.iter().map(|h| h.line).collect::<Vec<_>>(), vec![1, 3]);
        hits.clear();
        search_text(&re("x*", true), "n", "abc", &mut hits);
        assert!(hits.is_empty());
    }

    #[test]
    fn the_hit_cap_stops_the_search_and_says_so() {
        let text = "a\n".repeat(MAX_HITS + 5);
        let mut hits = vec![];
        assert!(search_text(&re("a", false), "a", &text, &mut hits));
        assert_eq!(hits.len(), MAX_HITS);
    }

    #[test]
    fn a_long_line_is_cut_for_the_list() {
        let mut hits = vec![];
        let text = format!("x{}", "y".repeat(500));
        search_text(&re("x", false), "m", &text, &mut hits);
        assert_eq!(hits[0].text.chars().count(), MAX_LINE_CHARS + 1);
        assert!(hits[0].text.ends_with('…'));
    }

    #[test]
    fn replacing_rewrites_the_text_and_counts_chars_not_bytes() {
        let o = Options::default();
        let (new, n) = replace_text("é a\nb a", "a", "XX", o).unwrap().unwrap();
        assert_eq!((new.as_str(), n), ("é XX\nb XX", 2));
        assert_eq!(replace_text("nothing", "zzz", "q", o).unwrap(), None);
        let regex = Options {
            regex: true,
            ..Default::default()
        };
        let (new, n) = replace_text("k=1 j=2", r"(\w)=(\d)", "$2=$1", regex)
            .unwrap()
            .unwrap();
        assert_eq!((new.as_str(), n), ("1=k 2=j", 2));
        assert!(replace_text("x", "(", "y", regex).is_err());
    }

    #[test]
    fn a_nul_in_the_head_makes_a_file_binary() {
        assert!(is_binary(b"ab\0cd"));
        assert!(!is_binary("plain é text".as_bytes()));
    }
}
