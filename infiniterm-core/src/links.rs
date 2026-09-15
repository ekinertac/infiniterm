//! Finding the things in a line of terminal output that can be opened.
//! Port of links.ts and its tests.
//!
//! Two kinds: URLs, and paths. A path is anything that LOOKS like one
//! (`src/lib/foo.ts`, `~/Code`, `./build`, `README.md:42`); whether it IS
//! one is decided later by asking the filesystem (`paths_exist` in the
//! backend), relative to the card's directory. That check keeps `node.js` in
//! a sentence and `1.2.3` in a version from being underlined, so matching
//! generously here is fine. A `:line:col` suffix is underlined but not
//! opened; the punctuation a sentence leaves on a path is neither.
//!
//! Offsets are BYTE offsets into `line`; the terminal element maps them to
//! cells (the reference's are UTF-16 units for xterm, the same idea). The
//! regexes are the reference's with `\w`, `\b` and `\d` pinned to ASCII, as
//! JavaScript's are.
use regex::Regex;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Url,
    Path,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub kind: LinkKind,
    /// Byte offsets in the line, end exclusive.
    pub start: usize,
    pub end: usize,
    /// The text as written, suffix and all: what is underlined.
    pub text: String,
    /// What to open: the URL, or the path without a :line:col suffix.
    pub target: String,
    /// The line from a `:42` or `:42:7` suffix, for opening an editor there.
    pub line: Option<u64>,
}

struct Patterns {
    url: Regex,
    path: Regex,
    trailing: Regex,
    line_suffix: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| Patterns {
        url: Regex::new(r#"https?://[^\s'"<>()\[\]]+"#).unwrap(),
        // Something with a slash in it, or a tilde/dot start, or a bare
        // filename with an extension. Trailing punctuation trimmed after.
        path: Regex::new(
            r"(?-u)(?:~|\.{1,2})?/[\w.@%+=:,\-~/]+|[\w.@%+\-~]+/[\w.@%+=:,\-~/]*|\b[\w\-]+(?:\.[\w\-]+)+(?::\d+(?::\d+)?)?",
        )
        .unwrap(),
        trailing: Regex::new(r#"[.,;:)\]'"]+$"#).unwrap(),
        line_suffix: Regex::new(r"(?-u):(\d+)(?::\d+)?$").unwrap(),
    })
}

pub fn find_links(line: &str) -> Vec<Found> {
    let p = patterns();
    let mut found: Vec<Found> = vec![];
    for m in p.url.find_iter(line) {
        let text = p.trailing.replace(m.as_str(), "").into_owned();
        found.push(Found {
            kind: LinkKind::Url,
            start: m.start(),
            end: m.start() + text.len(),
            target: text.clone(),
            text,
            line: None,
        });
    }
    for m in p.path.find_iter(line) {
        let text = p.trailing.replace(m.as_str(), "").into_owned();
        if text.is_empty() || text == "/" || text == "." || text == ".." {
            continue;
        }
        let (start, end) = (m.start(), m.start() + text.len());
        // Inside a URL already found: the URL wins.
        if found
            .iter()
            .any(|f| f.kind == LinkKind::Url && start >= f.start && end <= f.end)
        {
            continue;
        }
        let line_no = p
            .line_suffix
            .captures(&text)
            .and_then(|c| c[1].parse().ok());
        found.push(Found {
            kind: LinkKind::Path,
            start,
            end,
            target: p.line_suffix.replace(&text, "").into_owned(),
            text,
            line: line_no,
        });
    }
    found.sort_by_key(|f| f.start);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: &str, text: &str) -> Option<Found> {
        find_links(line).into_iter().find(|f| f.text == text)
    }

    #[test]
    fn finds_a_url_and_trims_the_sentence_punctuation_after_it() {
        let f = at(
            "see https://example.com/a/b. now",
            "https://example.com/a/b",
        )
        .unwrap();
        assert_eq!(f.kind, LinkKind::Url);
        assert_eq!(f.target, "https://example.com/a/b");
        assert_eq!(f.start, 4);
    }

    #[test]
    fn finds_paths_with_slashes_tildes_and_dots() {
        assert_eq!(
            at("edit src/lib/foo.ts please", "src/lib/foo.ts")
                .unwrap()
                .kind,
            LinkKind::Path
        );
        assert_eq!(
            at("cd ~/Code/infiniterm", "~/Code/infiniterm")
                .unwrap()
                .target,
            "~/Code/infiniterm"
        );
        assert_eq!(at("ls ./build", "./build").unwrap().kind, LinkKind::Path);
        assert_eq!(
            at("in /usr/local/bin:", "/usr/local/bin").unwrap().target,
            "/usr/local/bin"
        );
    }

    // The underline covers the suffix; the open does not.
    #[test]
    fn keeps_a_line_col_suffix_in_the_text_but_not_the_target() {
        let f = at("error at src/App.svelte:42:7", "src/App.svelte:42:7").unwrap();
        assert_eq!(f.target, "src/App.svelte");
        assert_eq!(f.line, Some(42));
        assert_eq!(
            at("see src/lib/foo.ts", "src/lib/foo.ts").unwrap().line,
            None
        );
    }

    #[test]
    fn offers_a_bare_filename_with_an_extension_to_be_checked_later() {
        assert_eq!(
            at("read README.md first", "README.md").unwrap().kind,
            LinkKind::Path
        );
    }

    #[test]
    fn does_not_double_offer_the_path_inside_a_url() {
        let all = find_links("https://github.com/ekinertac/infiniterm/blob/master/README.md");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].kind, LinkKind::Url);
    }

    #[test]
    fn trims_closing_brackets_and_quotes() {
        assert_eq!(
            at("in 'src/lib/split.ts')", "src/lib/split.ts")
                .unwrap()
                .target,
            "src/lib/split.ts"
        );
    }

    #[test]
    fn ignores_lone_slashes_and_dots() {
        assert!(find_links("a / b . c")
            .iter()
            .all(|f| f.kind != LinkKind::Path));
    }
}
