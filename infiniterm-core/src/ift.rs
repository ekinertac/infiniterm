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
    /// A Markdown document as a Page card (page.rs).
    Page {
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

/// How a script names a card (#127): the number on its label, `7` or `#7`
/// (the same after a reboot, unlike an id), else the id itself.
#[derive(Debug, PartialEq, Eq)]
pub enum CardRef {
    Number(u32),
    Id(String),
}

pub fn parse_card_ref(arg: &str) -> Option<CardRef> {
    let arg = arg.trim();
    if arg.is_empty() {
        return None;
    }
    match arg.strip_prefix('#').unwrap_or(arg).parse::<u32>() {
        Ok(n) => Some(CardRef::Number(n)),
        Err(_) => Some(CardRef::Id(arg.to_string())),
    }
}

/// The bytes a named key sends, for `ift send --key`. Names a person would
/// type; `ctrl-c` twice is how Claude Code is told to leave.
pub fn key_bytes(name: &str) -> Option<&'static [u8]> {
    Some(match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => b"\r",
        "esc" | "escape" => b"\x1b",
        "tab" => b"\t",
        "backspace" => b"\x7f",
        "ctrl-c" => b"\x03",
        "ctrl-d" => b"\x04",
        "ctrl-l" => b"\x0c",
        "ctrl-z" => b"\x1a",
        "up" => b"\x1b[A",
        "down" => b"\x1b[B",
        "right" => b"\x1b[C",
        "left" => b"\x1b[D",
        _ => return None,
    })
}

/// `ift send <card> [text...] [--enter] [--key NAME]...`: the card, then the
/// bytes to type, the text first and the keys after it in the order given.
pub fn parse_send(args: &[String]) -> Result<(String, Vec<u8>), String> {
    let card = args.first().ok_or("send takes a card and what to type")?;
    let mut text: Vec<&str> = vec![];
    let mut keys: Vec<u8> = vec![];
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--enter" => keys.extend_from_slice(b"\r"),
            "--key" => {
                let name = it.next().ok_or("--key takes a key name")?;
                keys.extend_from_slice(key_bytes(name).ok_or(format!("no key called {name}"))?);
            }
            other => text.push(other),
        }
    }
    let mut bytes = text.join(" ").into_bytes();
    bytes.extend(keys);
    if bytes.is_empty() {
        return Err("nothing to send".into());
    }
    Ok((card.clone(), bytes))
}

/// `ift read <card> [--lines N] [--all]`: the card, how many of the last
/// lines (none: the whole screen), and whether to reach into the history.
pub fn parse_read(args: &[String]) -> Result<(String, Option<usize>, bool), String> {
    let card = args.first().ok_or("read takes a card")?;
    let mut last = None;
    let mut scrollback = false;
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--all" => scrollback = true,
            "--lines" => {
                let n = it.next().ok_or("--lines takes a number")?;
                last = Some(n.parse().map_err(|_| format!("not a number: {n}"))?);
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok((card.clone(), last, scrollback))
}

/// `ift ls --agents`: one line per card that has an agent, for scripts that
/// restart them: number, agent, session id, state, directory, tab separated.
pub fn format_agents(rows: &[(u32, &str, &str, &str, &str)]) -> String {
    rows.iter()
        .map(|(n, kind, session, state, cwd)| format!("{n}\t{kind}\t{session}\t{state}\t{cwd}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One row per workspace, in tab order: id, name, cards, active. The active
/// column is `*` for the workspace on screen and `-` otherwise, the same
/// placeholder as `ift ls`, so a shell `read` never loses a column. A name is
/// free text: tabs and newlines become spaces so one row stays one row.
pub fn format_workspace_list(rows: &[(&str, &str, usize, bool)]) -> String {
    rows.iter()
        .map(|(id, name, cards, active)| {
            let name: String = name
                .chars()
                .map(|c| {
                    if c == '\t' || c == '\n' || c == '\r' {
                        ' '
                    } else {
                        c
                    }
                })
                .collect();
            let name = if name.trim().is_empty() {
                EMPTY.to_string()
            } else {
                name
            };
            format!(
                "{id}\t{name}\t{cards}\t{}",
                if *active { "*" } else { EMPTY }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Which workspace `ift terminal` opens its cards on.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkspaceTarget {
    /// An id or an exact name, resolved against the live workspaces.
    Existing(String),
    /// `--workspace new`: make one first.
    New,
}

/// `ift terminal`'s arguments: the flags in front, then the directories.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerminalArgs<'a> {
    pub workspace: Option<&'a str>,
    pub name: Option<&'a str>,
    pub dirs: Vec<&'a str>,
}

/// Splits `[--workspace V] [--name V] [--] <dir>...`. Flags only lead; `--`
/// ends them, so a directory called `--workspace` still opens. A word that
/// starts with `--` and is not one of ours stops the flags too and is taken
/// as a directory, which is what an app before this flag did with it.
pub fn parse_terminal_args(args: &[String]) -> Result<TerminalArgs<'_>, String> {
    let mut out = TerminalArgs::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--" => {
                i += 1;
                break;
            }
            flag @ ("--workspace" | "--name") => {
                let Some(value) = args.get(i + 1) else {
                    return Err(format!("{flag} takes a value"));
                };
                let slot = if flag == "--workspace" {
                    &mut out.workspace
                } else {
                    &mut out.name
                };
                if slot.replace(value.as_str()).is_some() {
                    return Err(format!("{flag} given twice"));
                }
                i += 2;
            }
            _ => break,
        }
    }
    out.dirs = args[i..].iter().map(String::as_str).collect();
    Ok(out)
}

/// What `--workspace <value>` names: an id, else an exact name (case
/// sensitive), else the word `new`. A name two workspaces share is an error,
/// not first-wins: the launcher should send the id. A workspace that happens
/// to be called "new" is reachable by its id.
pub fn resolve_workspace(
    workspaces: &[(&str, &str)],
    value: &str,
) -> Result<WorkspaceTarget, String> {
    if let Some((id, _)) = workspaces.iter().find(|(id, _)| *id == value) {
        return Ok(WorkspaceTarget::Existing((*id).to_string()));
    }
    let named: Vec<&str> = workspaces
        .iter()
        .filter(|(_, name)| *name == value)
        .map(|(id, _)| *id)
        .collect();
    match named.as_slice() {
        [id] => Ok(WorkspaceTarget::Existing((*id).to_string())),
        [] if value == "new" => Ok(WorkspaceTarget::New),
        [] => Err(format!("no workspace {value}")),
        _ => Err("workspace name is ambiguous: use the id".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn workspace_list_is_one_tab_separated_row_each() {
        let text = format_workspace_list(&[("w1", "api", 3, true), ("w2", "docs", 0, false)]);
        assert_eq!(text, "w1\tapi\t3\t*\nw2\tdocs\t0\t-");
        assert_eq!(format_workspace_list(&[]), "");
    }

    #[test]
    fn workspace_names_cannot_break_a_row() {
        let text = format_workspace_list(&[("w1", "a\tb\nc", 1, false), ("w2", " ", 0, false)]);
        let rows: Vec<Vec<&str>> = text.lines().map(|l| l.split('\t').collect()).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], ["w1", "a b c", "1", "-"]);
        assert_eq!(rows[1][1], "-");
    }

    #[test]
    fn terminal_args_split_flags_from_directories() {
        let a = strings(&["--workspace", "new", "--name", "demo", "--", "/a", "/b"]);
        let p = parse_terminal_args(&a).unwrap();
        assert_eq!(p.workspace, Some("new"));
        assert_eq!(p.name, Some("demo"));
        assert_eq!(p.dirs, ["/a", "/b"]);
        let only = strings(&["/a"]);
        let p = parse_terminal_args(&only).unwrap();
        assert_eq!(
            p,
            TerminalArgs {
                dirs: vec!["/a"],
                ..Default::default()
            }
        );
    }

    #[test]
    fn terminal_args_double_dash_protects_a_directory_named_like_a_flag() {
        let a = strings(&["--", "--workspace", "x"]);
        let p = parse_terminal_args(&a).unwrap();
        assert_eq!(p.workspace, None);
        assert_eq!(p.dirs, ["--workspace", "x"]);
        // Not ours: stays a directory, as before the flags existed.
        let unknown = strings(&["--other", "/a"]);
        let p = parse_terminal_args(&unknown).unwrap();
        assert_eq!(p.dirs, ["--other", "/a"]);
    }

    #[test]
    fn terminal_args_refuse_a_missing_or_repeated_value() {
        assert!(parse_terminal_args(&strings(&["--workspace"])).is_err());
        assert!(parse_terminal_args(&strings(&["--name"])).is_err());
        let twice = strings(&["--workspace", "a", "--workspace", "b", "/d"]);
        assert!(parse_terminal_args(&twice).is_err());
    }

    #[test]
    fn workspace_resolves_by_id_then_name_then_new() {
        let ws = [("w1", "api"), ("w2", "docs"), ("w3", "docs"), ("w4", "w1")];
        let existing = |id: &str| Ok(WorkspaceTarget::Existing(id.to_string()));
        assert_eq!(resolve_workspace(&ws, "w2"), existing("w2"));
        assert_eq!(resolve_workspace(&ws, "api"), existing("w1"));
        // An id beats a name that spells it.
        assert_eq!(resolve_workspace(&ws, "w1"), existing("w1"));
        assert_eq!(resolve_workspace(&ws, "new"), Ok(WorkspaceTarget::New));
        let e = resolve_workspace(&ws, "docs").unwrap_err();
        assert!(e.contains("ambiguous"), "{e}");
        let e = resolve_workspace(&ws, "API").unwrap_err();
        assert_eq!(e, "no workspace API");
    }

    #[test]
    fn a_workspace_called_new_wins_over_the_word() {
        let ws = [("w1", "new")];
        assert_eq!(
            resolve_workspace(&ws, "new"),
            Ok(WorkspaceTarget::Existing("w1".into()))
        );
    }

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
        (id == "g1").then(|| "acme.dev".to_string())
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
        .contains("acme.dev"));
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

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_card_is_named_by_number_or_id() {
        assert_eq!(parse_card_ref("7"), Some(CardRef::Number(7)));
        assert_eq!(parse_card_ref("#7"), Some(CardRef::Number(7)));
        assert_eq!(
            parse_card_ref("5949c24e"),
            Some(CardRef::Id("5949c24e".into()))
        );
        assert_eq!(parse_card_ref("  "), None);
    }

    #[test]
    fn send_types_the_text_then_the_keys_in_order() {
        assert_eq!(
            parse_send(&v(&["#7", "/exit", "--enter"])).unwrap(),
            ("#7".to_string(), b"/exit\r".to_vec())
        );
        assert_eq!(
            parse_send(&v(&["7", "--key", "ctrl-c", "--key", "ctrl-c"]))
                .unwrap()
                .1,
            b"\x03\x03"
        );
        assert_eq!(
            parse_send(&v(&["7", "claude", "--resume", "abc", "--enter"]))
                .unwrap()
                .1,
            b"claude --resume abc\r"
        );
        assert!(parse_send(&v(&[])).is_err());
        assert!(parse_send(&v(&["7"])).is_err());
        assert!(parse_send(&v(&["7", "--key", "nope"])).is_err());
        assert!(parse_send(&v(&["7", "--key"])).is_err());
    }

    #[test]
    fn read_takes_a_card_a_line_count_and_all() {
        assert_eq!(parse_read(&v(&["3"])).unwrap(), ("3".into(), None, false));
        assert_eq!(
            parse_read(&v(&["3", "--lines", "20", "--all"])).unwrap(),
            ("3".into(), Some(20), true)
        );
        assert!(parse_read(&v(&[])).is_err());
        assert!(parse_read(&v(&["3", "--lines", "x"])).is_err());
        assert!(parse_read(&v(&["3", "--what"])).is_err());
    }

    #[test]
    fn the_agents_list_is_one_tab_separated_line_per_card() {
        assert_eq!(
            format_agents(&[
                (7, "claude", "abc-123", "done", "/a"),
                (9, "codex", "z", "working", "/b")
            ]),
            "7\tclaude\tabc-123\tdone\t/a\n9\tcodex\tz\tworking\t/b"
        );
        assert_eq!(format_agents(&[]), "");
    }
}
