//! What a card calls itself. Port of cardLabel.ts and its tests.
//!
//! Three sources in priority order, because a label should answer "what is
//! this card doing right now?" and only the first is a claim the user made:
//! a name they set (or an OSC title the shell wrote, which is how Claude
//! Code's /rename arrives); the foreground process; the directory with the
//! home shortened to `~`. Editors are their file, browsers their host,
//! transcripts the first eight characters of the session id. State (dirty,
//! language, read-only) is a BADGE beside the label in the card frame, never
//! part of the name: the name says which card, the badges say what it is.
//!
//! The bug this replaces in the reference's history: the name was taken from
//! the directory once at creation, so every card opened in the home
//! directory was permanently called `ekinertac`. `inspect.rs` supplies the
//! live process and directory; the card frame calls `card_label` each paint.
use crate::saved_layout::CardKind;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Labelled<'a> {
    /// A name the user or the shell set. Empty when never set.
    pub title: &'a str,
    /// The name an AGENT in this card gave its session, through the
    /// terminal's title escape. Only ever set while an agent is running
    /// there, so a shell's own title never reaches a label.
    pub session: Option<&'a str>,
    /// The foreground process, from the process table.
    pub proc: Option<&'a str>,
    pub cwd: &'a str,
    /// An editor card's file, a transcript card's session file.
    pub path: Option<&'a str>,
    pub kind: Option<CardKind>,
    /// A browser card's page; the label is its host.
    pub url: Option<&'a str>,
    /// An editor's explorer root; the label when there is no file yet.
    pub root: Option<&'a str>,
}

/// A path with the home directory written as `~`. The full path rather than
/// the last segment, because half the directories anyone works in are
/// called `src`; the home prefix is the only part that is the same on every
/// card. Matches on a segment boundary, so `/Users/ekinertacular` is not a
/// path inside `/Users/ekinertac`.
pub fn tilde_path(path: &str, home: &str) -> String {
    let h = home.trim_end_matches('/');
    if h.is_empty() {
        return path.to_string();
    }
    if path == h {
        return "~".to_string();
    }
    match path.strip_prefix(h) {
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// `en.wikipedia.org` for a page there; the whole string when it is not a
/// URL. The WHATWG `host` (with port, without credentials), found by hand:
/// a string without `://` is not a URL with a host and comes back whole.
fn host_of(url: &str) -> &str {
    let Some((_, rest)) = url.split_once("://") else {
        return url;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    if host.is_empty() {
        url
    } else {
        host
    }
}

fn last_segment(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or("");
    if name.is_empty() {
        path
    } else {
        name
    }
}

pub fn card_label(card: &Labelled, home: &str) -> String {
    let title = card.title.trim();
    if !title.is_empty() {
        return title.to_string();
    }
    if card.kind == Some(CardKind::Browser) {
        return card.url.map_or("browser", host_of).to_string();
    }
    // A session file is named by a uuid (Claude Code) or `<timestamp>_<uuid>`
    // (Pi); the uuid's first eight characters are how Claude Code itself
    // refers to a session, and the timestamp would make every Pi card read
    // `2026-09-`.
    if card.kind == Some(CardKind::Transcript) {
        let Some(path) = card.path else {
            return "transcript".to_string();
        };
        let name = path.rsplit('/').next().unwrap_or("");
        let start = name.rfind('_').map_or(0, |i| i + 1);
        return name[start..].chars().take(8).collect();
    }
    // A page is named by its document, without the `.md`: "Start here".
    if card.kind == Some(CardKind::Page) {
        if let Some(path) = card.path {
            let name = last_segment(path);
            return name.strip_suffix(".md").unwrap_or(name).to_string();
        }
    }
    if let Some(path) = card.path {
        return last_segment(path).to_string();
    }
    let file_kind = matches!(card.kind, Some(CardKind::Editor | CardKind::Diff));
    if let (true, Some(root)) = (file_kind, card.root) {
        return format!("{}/", last_segment(root));
    }
    match card.kind {
        Some(CardKind::Editor) => return "untitled".to_string(),
        Some(CardKind::Diff) => return "diff".to_string(),
        _ => {}
    }
    // Above the process, because "claude" on six cards says less than the
    // six things those sessions are called; below a chosen name, because
    // renaming a card is a decision and this is not.
    if let Some(session) = card.session.map(str::trim).filter(|s| !s.is_empty()) {
        return session.to_string();
    }
    if let Some(proc) = card.proc.map(str::trim).filter(|p| !p.is_empty()) {
        return proc.to_string();
    }
    tilde_path(card.cwd, home)
}

/// Splits a label so the last path segment can be kept when the card is too
/// narrow for the whole thing: the head ellipsises, the tail never does.
/// Exists because the obvious CSS for a left-hand ellipsis (`direction:
/// rtl`) reordered `~/Code/acme.dev` into `Code/acme.dev/~`; the native
/// frame lays out the two halves and needs no bidi either. A label with no
/// slash (a process name) is all tail.
pub fn split_label(label: &str) -> (&str, &str) {
    match label.rfind('/') {
        Some(cut) if cut > 0 => label.split_at(cut),
        _ => ("", label),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/Users/ekinertac";

    fn card<'a>() -> Labelled<'a> {
        Labelled {
            cwd: "/Users/ekinertac/Code/api",
            ..Default::default()
        }
    }

    // An agent's session name beats the process it is running under, and
    // loses to a name somebody chose. Claude Code renames the session (and
    // so the terminal's title) as the work changes, and `/rename` is the
    // same thing done by hand.
    #[test]
    fn an_agents_session_name_outranks_its_process_and_nothing_else() {
        let session = Labelled {
            session: Some("porting the omnibox"),
            proc: Some("claude"),
            ..card()
        };
        assert_eq!(card_label(&session, HOME), "porting the omnibox");

        let renamed = Labelled {
            title: "the one I named",
            ..session.clone()
        };
        assert_eq!(card_label(&renamed, HOME), "the one I named");

        // Nothing to say is not something to say.
        let blank = Labelled {
            session: Some("   "),
            ..session.clone()
        };
        assert_eq!(card_label(&blank, HOME), "claude");

        let no_agent = Labelled {
            session: None,
            ..session
        };
        assert_eq!(card_label(&no_agent, HOME), "claude");
    }

    fn kind<'a>(kind: CardKind, path: Option<&'a str>) -> Labelled<'a> {
        Labelled {
            cwd: "/x",
            kind: Some(kind),
            path,
            ..Default::default()
        }
    }

    #[test]
    fn a_name_you_set_wins_over_everything() {
        assert_eq!(
            card_label(
                &Labelled {
                    title: "deploy",
                    proc: Some("vim"),
                    ..card()
                },
                ""
            ),
            "deploy"
        );
    }

    #[test]
    fn a_running_process_names_the_card_while_it_runs() {
        assert_eq!(
            card_label(
                &Labelled {
                    proc: Some("vim"),
                    ..card()
                },
                ""
            ),
            "vim"
        );
    }

    #[test]
    fn an_idle_card_is_named_for_its_directory_home_shortened_to_tilde() {
        assert_eq!(card_label(&card(), HOME), "~/Code/api");
    }

    // The name was once taken from the directory at creation, so every card
    // opened in the home directory was permanently called `ekinertac`.
    #[test]
    fn the_directory_is_read_live_not_frozen_at_creation() {
        assert_eq!(
            card_label(
                &Labelled {
                    cwd: HOME,
                    ..card()
                },
                HOME
            ),
            "~"
        );
        assert_eq!(
            card_label(
                &Labelled {
                    cwd: "/Users/ekinertac/Code/infiniterm",
                    ..card()
                },
                HOME
            ),
            "~/Code/infiniterm"
        );
    }

    #[test]
    fn whitespace_does_not_count_as_a_name() {
        assert_eq!(
            card_label(
                &Labelled {
                    title: "   ",
                    proc: Some("vim"),
                    ..card()
                },
                ""
            ),
            "vim"
        );
        assert_eq!(
            card_label(
                &Labelled {
                    proc: Some("  "),
                    ..card()
                },
                HOME
            ),
            "~/Code/api"
        );
    }

    // The full path, because half the directories anyone works in are called `src`.
    #[test]
    fn tilde_path_shortens_only_the_home_prefix() {
        assert_eq!(tilde_path("/Users/ekinertac/Code/api", HOME), "~/Code/api");
        assert_eq!(tilde_path(HOME, HOME), "~");
        assert_eq!(tilde_path("/etc/hosts", HOME), "/etc/hosts");
    }

    // A segment boundary, so a sibling with a longer name is not swallowed.
    #[test]
    fn tilde_path_does_not_match_a_partial_segment() {
        assert_eq!(
            tilde_path("/Users/ekinertacular/x", HOME),
            "/Users/ekinertacular/x"
        );
    }

    #[test]
    fn tilde_path_tolerates_a_trailing_slash_on_home_and_no_home_at_all() {
        assert_eq!(tilde_path("/Users/me/Code", "/Users/me/"), "~/Code");
        assert_eq!(tilde_path("/Users/me/Code", ""), "/Users/me/Code");
    }

    #[test]
    fn split_label_keeps_the_last_segment_whole() {
        assert_eq!(
            split_label("~/Code/acme.dev/gunicorn_workers"),
            ("~/Code/acme.dev", "/gunicorn_workers")
        );
    }

    #[test]
    fn split_label_leaves_a_label_with_no_path_all_tail() {
        assert_eq!(split_label("vim"), ("", "vim"));
        assert_eq!(split_label("~"), ("", "~"));
    }

    #[test]
    fn split_label_does_not_strand_a_leading_slash_on_its_own() {
        assert_eq!(split_label("/etc"), ("", "/etc"));
    }

    #[test]
    fn an_editor_card_is_labelled_by_its_file_state_is_a_badge_not_part_of_the_name() {
        let ed = Labelled {
            cwd: "/x",
            path: Some("/x/App.svelte"),
            ..Default::default()
        };
        assert_eq!(card_label(&ed, ""), "App.svelte");
        // A chosen name still wins.
        assert_eq!(
            card_label(
                &Labelled {
                    title: "notes",
                    path: Some("/x/a.md"),
                    ..ed
                },
                ""
            ),
            "notes"
        );
    }

    #[test]
    fn an_untitled_editor_says_so() {
        assert_eq!(card_label(&kind(CardKind::Editor, None), ""), "untitled");
    }

    #[test]
    fn an_editor_on_a_directory_is_labelled_by_the_directory() {
        let ed = Labelled {
            cwd: "/x/proj",
            root: Some("/x/proj"),
            kind: Some(CardKind::Editor),
            ..Default::default()
        };
        assert_eq!(card_label(&ed, ""), "proj/");
    }

    #[test]
    fn a_browser_card_is_its_host_and_browser_before_it_has_one() {
        let web = |url| Labelled {
            url,
            ..kind(CardKind::Browser, None)
        };
        assert_eq!(
            card_label(&web(Some("https://en.wikipedia.org/wiki/Main_Page")), ""),
            "en.wikipedia.org"
        );
        assert_eq!(
            card_label(&web(Some("localhost:1420")), ""),
            "localhost:1420"
        );
        assert_eq!(card_label(&web(None), ""), "browser");
    }

    #[test]
    fn a_transcript_card_is_the_first_eight_characters_of_its_session_id() {
        assert_eq!(
            card_label(
                &kind(CardKind::Transcript, Some("/s/a840a9f8-4efe-4b76.jsonl")),
                ""
            ),
            "a840a9f8"
        );
        assert_eq!(
            card_label(
                &kind(
                    CardKind::Transcript,
                    Some("/s/2026-09-08T14-50-29-889Z_01a0817f-c681.jsonl")
                ),
                ""
            ),
            "01a0817f"
        );
        assert_eq!(
            card_label(&kind(CardKind::Transcript, None), ""),
            "transcript"
        );
    }

    // Native check: the hand-rolled host matches the WHATWG `host` for the
    // shapes a browser card sees.
    #[test]
    fn host_of_matches_the_url_host() {
        assert_eq!(host_of("https://user:pw@host:8080/x?y#z"), "host:8080");
        assert_eq!(host_of("http://localhost:1420"), "localhost:1420");
        assert_eq!(host_of("file:///tmp/x.html"), "file:///tmp/x.html");
    }
}
