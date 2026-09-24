//! Formatting `ift` answers and deciding what a path opens as. Port of
//! ift.ts and its tests.
//!
//! `ift ls` output is API the moment anything pipes it (Hyrum's Law): the
//! column ORDER, the separator and the placeholder for an absent value are
//! promises. New information goes in a new trailing column, never by
//! reordering; a richer shape goes behind `--json`. Tab separated with no
//! header, because that is what `cut -f2` and `awk` expect.
//!
//! `OpenPlan` is the ONE path for `ift <path>`, Cmd+click on a path and the
//! typed-path prompt, so the three cannot drift. `ift` itself decides
//! nothing: it resolves paths (it stands in the directory) and the app does
//! the rest. The commands layer turns a plan into a card beside the asker.
use crate::card_label::tilde_path;

pub struct ListedCard<'a> {
    pub id: &'a str,
    pub cwd: &'a str,
    pub group_id: Option<&'a str>,
    /// The agent state's kind: `none`, `working`, `idle`.
    pub agent: &'a str,
    pub remote: Option<&'a str>,
    /// The card's `#7`, as shown on its label.
    pub number: u32,
}

/// The stand-in for a column with nothing in it, so field counts never vary.
pub const EMPTY: &str = "-";

/// One row per card: id, group, directory, agent state, remote, number.
/// The id first because it is what every other verb takes, so `ift ls |
/// cut -f1` is the list of things you can act on. Paths keep `~`: read by a
/// person as often as by a script, and a script can expand one prefix. The
/// number came last, as a trailing column: the five before it are what
/// scripts already cut.
pub fn format_card_list(
    cards: &[ListedCard],
    group_name: impl Fn(&str) -> Option<String>,
    home: &str,
) -> String {
    cards
        .iter()
        .map(|c| {
            let group = c.group_id.and_then(&group_name).filter(|n| !n.is_empty());
            let dir = tilde_path(c.cwd, home);
            [
                c.id.to_string(),
                group.unwrap_or_else(|| EMPTY.to_string()),
                if dir.is_empty() {
                    EMPTY.to_string()
                } else {
                    dir
                },
                c.agent.to_string(),
                c.remote
                    .filter(|r| !r.is_empty())
                    .unwrap_or(EMPTY)
                    .to_string(),
                if c.number > 0 {
                    c.number.to_string()
                } else {
                    EMPTY.to_string()
                },
            ]
            .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    Directory,
    File,
}

/// What a path should open as. A directory becomes an editor with the
/// explorer rooted there (the tree is the way to a file); a file becomes an
/// editor whose `cwd` is the file's directory, so "beside" lands right.
#[derive(Clone, Debug, PartialEq)]
pub enum OpenPlan {
    Editor {
        cwd: String,
        path: Option<String>,
        root: Option<String>,
        line: Option<u64>,
    },
    Diff {
        cwd: String,
        path: Option<String>,
        root: String,
    },
    Browser {
        cwd: String,
        url: String,
    },
    Transcript {
        cwd: String,
        path: String,
    },
    Refused {
        text: String,
    },
}

/// A URL, from Cmd+click on one in a terminal: a browser card. `cwd` is the
/// clicking card's directory, kept only so "beside" and the next Cmd+T work.
pub fn url_plan(url: &str, cwd: &str) -> OpenPlan {
    OpenPlan::Browser {
        cwd: cwd.into(),
        url: url.into(),
    }
}

/// An agent's session file, as a transcript card beside the agent's card.
pub fn transcript_plan(path: &str, cwd: &str) -> OpenPlan {
    OpenPlan::Transcript {
        cwd: cwd.into(),
        path: path.into(),
    }
}

/// The directory of an absolute path, its root for a top-level file;
/// `None` for a relative one, which is what every caller turns into a
/// refusal. `ift` resolves a path before sending it, so a relative one
/// arriving means the caller did not.
///
/// `paths::is_rooted` rather than a leading `/`, because `C:\Users\PC` is
/// rooted and has no leading slash: `ift <a windows path>` was refused as
/// relative until this asked the right question.
fn parent(path: &str) -> Option<String> {
    crate::paths::is_rooted(path).then(|| crate::paths::parent_dir(path))
}

/// `ift diff <path>`: a directory shows every change beneath it, a file
/// only itself, and that file is shown at once.
pub fn diff_plan(path: &str, kind: PathKind) -> OpenPlan {
    if kind == PathKind::Directory {
        return OpenPlan::Diff {
            cwd: path.into(),
            path: None,
            root: path.into(),
        };
    }
    match parent(path) {
        Some(cwd) => OpenPlan::Diff {
            cwd,
            path: Some(path.into()),
            root: path.into(),
        },
        None => OpenPlan::Refused {
            text: format!("{path}: not an absolute path"),
        },
    }
}

/// `line` is where the editor opens, from `file:42` or a `path:42:7` link.
pub fn open_plan(path: &str, kind: PathKind, line: Option<f64>) -> OpenPlan {
    if kind == PathKind::Directory {
        return OpenPlan::Editor {
            cwd: path.into(),
            path: None,
            root: Some(path.into()),
            line: None,
        };
    }
    match parent(path) {
        Some(cwd) => OpenPlan::Editor {
            cwd,
            path: Some(path.into()),
            root: None,
            line: line.filter(|l| *l > 0.).map(|l| l.floor() as u64),
        },
        None => OpenPlan::Refused {
            text: format!("{path}: not an absolute path"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card<'a>() -> ListedCard<'a> {
        ListedCard {
            id: "c1",
            cwd: "/Users/me/Code/api",
            group_id: None,
            agent: "none",
            remote: None,
            number: 7,
        }
    }

    fn names(id: &str) -> Option<String> {
        (id == "g1").then(|| "humbl.ai".to_string())
    }

    fn row(text: &str) -> Vec<&str> {
        text.split('\t').collect()
    }

    #[test]
    fn is_tab_separated_with_the_id_first() {
        let r = format_card_list(&[card()], names, "/Users/me");
        assert_eq!(row(&r)[0], "c1");
        assert_eq!(row(&r)[2], "~/Code/api");
    }

    #[test]
    fn names_the_group_rather_than_its_id() {
        assert!(format_card_list(
            &[ListedCard {
                group_id: Some("g1"),
                ..card()
            }],
            names,
            ""
        )
        .contains("humbl.ai"));
    }

    // Field counts must never vary, or cut -f4 reads a different column per row.
    #[test]
    fn fills_an_absent_value_rather_than_leaving_it_blank() {
        let r = format_card_list(&[card()], names, "");
        assert_eq!(row(&r).len(), 6);
        assert_eq!(row(&r)[1], EMPTY);
        assert_eq!(row(&r)[4], EMPTY);
        assert_eq!(row(&r)[5], "7");
        let unnumbered = format_card_list(
            &[ListedCard {
                number: 0,
                ..card()
            }],
            names,
            "",
        );
        assert_eq!(row(&unnumbered)[5], EMPTY);
    }

    #[test]
    fn keeps_every_row_the_same_width_whatever_is_set() {
        let full = ListedCard {
            id: "c2",
            group_id: Some("g1"),
            remote: Some("ssh box"),
            agent: "working",
            ..card()
        };
        let rows = format_card_list(&[card(), full], names, "");
        assert_eq!(
            rows.lines().map(|r| row(r).len()).collect::<Vec<_>>(),
            [6, 6]
        );
    }

    #[test]
    fn is_one_row_per_card_and_empty_for_none() {
        assert_eq!(
            format_card_list(&[card(), ListedCard { id: "c2", ..card() }], names, "")
                .lines()
                .count(),
            2
        );
        assert_eq!(format_card_list(&[], names, ""), "");
    }

    // A group id that no longer resolves must not print a placeholder of its own.
    #[test]
    fn survives_a_group_that_has_gone() {
        let r = format_card_list(
            &[ListedCard {
                group_id: Some("gone"),
                ..card()
            }],
            names,
            "",
        );
        assert_eq!(row(&r)[1], EMPTY);
    }

    // The tree is the way to a file: a directory is an explorer, not a shell.
    #[test]
    fn opens_a_directory_as_an_editor_card_with_the_explorer_rooted_there() {
        assert_eq!(
            open_plan("/Users/me/Code", PathKind::Directory, None),
            OpenPlan::Editor {
                cwd: "/Users/me/Code".into(),
                path: None,
                root: Some("/Users/me/Code".into()),
                line: None
            }
        );
    }

    // The editor's cwd is the file's directory, so "beside" still means something.
    #[test]
    fn opens_a_file_as_an_editor_card_in_its_directory() {
        let plan = |line| open_plan("/Users/me/readme.md", PathKind::File, line);
        assert_eq!(
            plan(None),
            OpenPlan::Editor {
                cwd: "/Users/me".into(),
                path: Some("/Users/me/readme.md".into()),
                root: None,
                line: None
            }
        );
        assert!(matches!(
            plan(Some(42.)),
            OpenPlan::Editor { line: Some(42), .. }
        ));
        assert!(matches!(
            plan(Some(0.)),
            OpenPlan::Editor { line: None, .. }
        ));
        assert!(
            matches!(open_plan("/top.txt", PathKind::File, None), OpenPlan::Editor { cwd, .. } if cwd == "/")
        );
    }

    #[test]
    fn refuses_a_relative_file_path() {
        assert!(matches!(
            open_plan("readme.md", PathKind::File, None),
            OpenPlan::Refused { .. }
        ));
    }

    #[test]
    fn roots_a_diff_at_the_directory_or_at_the_file_with_the_file_shown() {
        assert_eq!(
            diff_plan("/r", PathKind::Directory),
            OpenPlan::Diff {
                cwd: "/r".into(),
                path: None,
                root: "/r".into()
            }
        );
        assert_eq!(
            diff_plan("/r/a.ts", PathKind::File),
            OpenPlan::Diff {
                cwd: "/r".into(),
                path: Some("/r/a.ts".into()),
                root: "/r/a.ts".into()
            }
        );
    }

    #[test]
    fn a_url_is_a_browser_card_standing_in_the_clicking_cards_directory() {
        assert_eq!(
            url_plan("https://example.com/a", "/x"),
            OpenPlan::Browser {
                cwd: "/x".into(),
                url: "https://example.com/a".into()
            }
        );
    }
}
