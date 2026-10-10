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
//! A domain written without a scheme (`github.com/ekinertac`, `example.com`)
//! is a URL too, opened over https (2026-09-25: before, it was offered as a
//! path, the filesystem said no, and Cmd+hover underlined nothing). What
//! makes it a domain rather than a file is its last label being a known
//! top-level domain (`BARE_TLDS`), a list kept to names that are not also
//! common file extensions, so `lib.rs`, `README.md` and `install.sh` stay
//! paths.
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
    /// The program marked this itself (OSC 8), so it is underlined at rest:
    /// text that is a link and does not look like one is only text to a user.
    /// Links found by pattern show only under Cmd+hover.
    pub marked: bool,
}

struct Patterns {
    url: Regex,
    path: Regex,
    trailing: Regex,
    line_suffix: Regex,
    explicit_ref: Regex,
    bare_ref: Regex,
}

/// Top-level domains that make `name.tld` a web address without a scheme.
/// Deliberately short and free of file extensions: `.rs`, `.md`, `.sh`,
/// `.py`, `.js`, `.ts`, `.go`, `.pl` and friends are real TLDs too, and a
/// terminal prints far more file names than bare domains.
const BARE_TLDS: &[&str] = &[
    "com", "org", "net", "io", "dev", "ai", "app", "co", "me", "gov", "edu", "info", "xyz", "so",
    "gg", "tv", "us", "uk", "de", "fr", "nl", "eu", "tr", "jp", "ca", "au", "ly", "to", "fm",
    "page", "site", "blog", "cloud", "tech", "news", "art", "biz", "ee", "it", "es", "se",
];

/// `github.com/ekinertac`, `www.example.org`, `localhost:3000`-free: a host
/// of dot-separated labels, the last one in `BARE_TLDS`, then an optional
/// port and path.
fn is_bare_domain(text: &str) -> bool {
    if text.starts_with(['~', '.', '/']) {
        return false;
    }
    let host = text.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        && labels
            .last()
            .is_some_and(|tld| BARE_TLDS.contains(&tld.to_ascii_lowercase().as_str()))
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
        explicit_ref: Regex::new(r"(?-u)([A-Za-z0-9][A-Za-z0-9_.-]*)/([A-Za-z0-9_.-]+)#(\d{1,7})").unwrap(),
        bare_ref: Regex::new(r"(?-u)#(\d{1,7})").unwrap(),
    })
}

pub fn find_links(line: &str) -> Vec<Found> {
    find_links_in(line, None)
}

/// A character that can be part of the word before or after a reference:
/// `x#12` and `#12ab` are not issue numbers.
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `owner/repo#432` anywhere, and a bare `#432` when the card's directory
/// belongs to a GitHub repository (`repo`): the issue or pull request page
/// (#335). A bare number needs a space or an opening bracket or quote before
/// it and no word character after, so `#!/bin/sh`, `a#1` and `#12ab` are not
/// references. A colour like `#123` is, which only shows as an underline under
/// Cmd.
fn issue_refs(line: &str, repo: Option<&crate::git_repo::Repo>) -> Vec<Found> {
    let p = patterns();
    let mut out = vec![];
    let after_ok = |end: usize| !line[end..].chars().next().is_some_and(is_word);
    for c in p.explicit_ref.captures_iter(line) {
        let all = c.get(0).unwrap();
        let before = line[..all.start()].chars().next_back();
        if before.is_some_and(|b| is_word(b) || matches!(b, '/' | '.' | '-' | '#' | ':'))
            || !after_ok(all.end())
        {
            continue;
        }
        let r = crate::git_repo::Repo {
            owner: c[1].to_string(),
            name: c[2].to_string(),
        };
        out.push(Found {
            kind: LinkKind::Url,
            start: all.start(),
            end: all.end(),
            text: all.as_str().to_string(),
            target: r.issue_url(&c[3]),
            line: None,
            marked: false,
        });
    }
    if let Some(repo) = repo {
        for c in p.bare_ref.captures_iter(line) {
            let all = c.get(0).unwrap();
            let before = line[..all.start()].chars().next_back();
            let opens = before.is_none_or(|b| {
                b.is_whitespace() || matches!(b, '(' | '[' | '{' | '"' | '\'' | '`' | '*' | ',')
            });
            if !opens || !after_ok(all.end()) {
                continue;
            }
            // Inside an explicit `owner/repo#432` already.
            if out
                .iter()
                .any(|f| all.start() >= f.start && all.end() <= f.end)
            {
                continue;
            }
            out.push(Found {
                kind: LinkKind::Url,
                start: all.start(),
                end: all.end(),
                text: all.as_str().to_string(),
                target: repo.issue_url(&c[1]),
                line: None,
                marked: false,
            });
        }
    }
    out
}

/// `find_links` plus the references to issues and pull requests of `repo`.
pub fn find_links_in(line: &str, repo: Option<&crate::git_repo::Repo>) -> Vec<Found> {
    let refs = issue_refs(line, repo);
    let mut found = find_links_plain(line);
    // A reference wins over the path or address it overlaps: `o/r#432` also
    // reads as the path `o/r` and `#432` as the tail of an address.
    found.retain(|f| !refs.iter().any(|r| f.start < r.end && r.start < f.end));
    found.extend(refs);
    found.sort_by_key(|f| f.start);
    found
}

/// `found` plus the OSC 8 hyperlinks of the row (`spans`: start column, end
/// column not included, address), the program's own word for where a link is.
/// A span wins over anything it overlaps. Only web addresses are offered:
/// the text of a link is whatever the program printed, and a Cmd+click must
/// not run `file:` or an application's own scheme. `line` has one character
/// per column, as the terminal's rows do.
pub fn merge_hyperlinks(
    mut found: Vec<Found>,
    line: &str,
    spans: &[(usize, usize, String)],
) -> Vec<Found> {
    let byte_of = |col: usize| line.char_indices().nth(col).map_or(line.len(), |(i, _)| i);
    let mut linked = vec![];
    for (start, end, uri) in spans {
        let lower = uri.to_ascii_lowercase();
        let web = lower.starts_with("http://") || lower.starts_with("https://");
        if !web || uri.len() > 2048 || uri.chars().any(|c| c.is_control() || c == ' ') {
            continue;
        }
        let (s, e) = (byte_of(*start), byte_of(*end));
        let text = line[s..e.max(s)].trim_end();
        if text.is_empty() {
            continue;
        }
        linked.push(Found {
            kind: LinkKind::Url,
            start: s,
            end: s + text.len(),
            text: text.to_string(),
            target: uri.clone(),
            line: None,
            marked: true,
        });
    }
    if linked.is_empty() {
        return found;
    }
    found.retain(|f| !linked.iter().any(|l| f.start < l.end && l.start < f.end));
    found.extend(linked);
    found.sort_by_key(|f| f.start);
    found
}

fn find_links_plain(line: &str) -> Vec<Found> {
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
            marked: false,
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
        if is_bare_domain(&text) {
            found.push(Found {
                kind: LinkKind::Url,
                start,
                end,
                target: format!("https://{text}"),
                text,
                line: None,
                marked: false,
            });
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
            marked: false,
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
    fn a_domain_without_a_scheme_is_a_url_opened_over_https() {
        let f = at(
            "see github.com/ekinertac/infiniterm.",
            "github.com/ekinertac/infiniterm",
        )
        .unwrap();
        assert_eq!(f.kind, LinkKind::Url);
        assert_eq!(f.target, "https://github.com/ekinertac/infiniterm");
        let f = at("go to example.com, then", "example.com").unwrap();
        assert_eq!(
            (f.kind, f.target.as_str()),
            (LinkKind::Url, "https://example.com")
        );
        // A port is the host's, not a line number. (A port AND a path is
        // cut at the port by the path pattern; rare enough to leave.)
        let f = at("on www.acme.dev:8443", "www.acme.dev:8443").unwrap();
        assert_eq!(
            (f.target.as_str(), f.line),
            ("https://www.acme.dev:8443", None)
        );
        assert_eq!(
            at("site.com.tr", "site.com.tr").unwrap().kind,
            LinkKind::Url
        );
    }

    #[test]
    fn file_names_stay_paths_even_where_the_extension_is_a_tld() {
        for name in [
            "lib.rs",
            "README.md",
            "install.sh",
            "run.py",
            "index.ts",
            "main.go",
            "Cargo.toml",
            "node.js",
            "src/app.rs",
        ] {
            let f = find_links(name);
            assert!(f.iter().all(|f| f.kind == LinkKind::Path), "{name}: {f:?}");
        }
        assert!(find_links("~/Code/foo.com")
            .iter()
            .all(|f| f.kind == LinkKind::Path));
        assert!(find_links("1.2.3").iter().all(|f| f.kind == LinkKind::Path));
    }

    #[test]
    fn ignores_lone_slashes_and_dots() {
        assert!(find_links("a / b . c")
            .iter()
            .all(|f| f.kind != LinkKind::Path));
    }

    fn repo() -> crate::git_repo::Repo {
        crate::git_repo::Repo {
            owner: "ekinertac".into(),
            name: "infiniterm".into(),
        }
    }

    fn texts(line: &str, repo: Option<&crate::git_repo::Repo>) -> Vec<(String, String)> {
        find_links_in(line, repo)
            .into_iter()
            .map(|f| (f.text, f.target))
            .collect()
    }

    // The case that started it (#335): an agent's summary line.
    #[test]
    fn the_numbers_in_a_summary_line_are_links_inside_a_repo() {
        let line =
            "Four issues are fixed and merged in PR #349 (#334, #336, #340, #346, all closed)";
        let found = texts(line, Some(&repo()));
        let numbers: Vec<&str> = found.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(numbers, ["#349", "#334", "#336", "#340", "#346"]);
        assert_eq!(
            found[0].1,
            "https://github.com/ekinertac/infiniterm/issues/349"
        );
        // Outside a repo a bare number is plain text.
        assert!(texts(line, None).is_empty());
    }

    #[test]
    fn a_bare_number_needs_a_clean_edge_on_both_sides() {
        let r = repo();
        for plain in [
            "#!/bin/sh",
            "a#12",
            "x#1",
            "#12ab",
            "#",
            "# 12",
            "color #12_x",
        ] {
            assert!(
                texts(plain, Some(&r)).iter().all(|(t, _)| !t.contains('#')),
                "{plain}"
            );
        }
        for (line, want) in [
            ("#12", "#12"),
            ("see #12.", "#12"),
            ("(#12)", "#12"),
            ("[#12]", "#12"),
            ("\"#12\"", "#12"),
            ("`#12`", "#12"),
            ("fixes #12, #13", "#12"),
        ] {
            assert_eq!(texts(line, Some(&r))[0].0, want, "{line}");
        }
    }

    #[test]
    fn owner_repo_number_is_a_link_anywhere_and_beats_the_path_it_contains() {
        let found = texts("see curiousgamesdev/CG-RGS#432 now", None);
        assert_eq!(
            found,
            vec![(
                "curiousgamesdev/CG-RGS#432".to_string(),
                "https://github.com/curiousgamesdev/CG-RGS/issues/432".to_string()
            )]
        );
        // With a repo of its own, the explicit one is not re-read as a bare number.
        let found = texts("o/r#5 and #6", Some(&repo()));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].1, "https://github.com/o/r/issues/5");
        assert_eq!(
            found[1].1,
            "https://github.com/ekinertac/infiniterm/issues/6"
        );
        // A path with a hash in it stays a path.
        assert!(texts("src/lib.rs#12x", None)
            .iter()
            .all(|(t, _)| !t.contains('#')));
    }

    #[test]
    fn a_reference_beside_a_url_leaves_the_url_alone() {
        let found = texts("https://github.com/a/b/issues/7 and #8", Some(&repo()));
        assert_eq!(found[0].0, "https://github.com/a/b/issues/7");
        assert_eq!(found[1].0, "#8");
    }

    #[test]
    fn an_osc8_span_becomes_a_link_over_whatever_the_text_says() {
        let line = "see issue 432 now";
        let spans = vec![(4, 13, "https://github.com/o/r/issues/432".to_string())];
        let found = merge_hyperlinks(find_links(line), line, &spans);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "issue 432");
        assert!(found[0].marked);
        assert!(find_links("https://example.com").iter().all(|f| !f.marked));
        assert_eq!(found[0].target, "https://github.com/o/r/issues/432");
        assert_eq!((found[0].start, found[0].end), (4, 13));
        // A span over text that already read as a link replaces it.
        let line = "go to github.com/a/b please";
        let spans = vec![(6, 20, "https://example.org/x".to_string())];
        let found = merge_hyperlinks(find_links(line), line, &spans);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].target, "https://example.org/x");
    }

    #[test]
    fn only_web_addresses_are_opened_and_columns_count_characters() {
        let line = "\u{4e2d}\u{6587} link";
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "vscode://x/y",
            "https://a b",
            "https://a\x1bb",
        ] {
            let spans = vec![(3, 7, bad.to_string())];
            assert!(
                merge_hyperlinks(vec![], line, &spans).is_empty(),
                "{bad} must not be a link"
            );
        }
        // Two wide characters then a space: the span starts at column 3.
        let spans = vec![(3, 7, "HTTP://example.com".to_string())];
        let found = merge_hyperlinks(vec![], line, &spans);
        assert_eq!(found[0].text, "link");
        // No spans: the links are returned untouched.
        let plain = find_links("https://example.com");
        assert_eq!(
            merge_hyperlinks(plain.clone(), "https://example.com", &[]),
            plain
        );
    }
}
