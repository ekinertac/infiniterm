//! The right-click menus as data (#278): for each place you can click, a list
//! of commands with when to show each and when to enable it. Pure: no window,
//! no AppKit. `infiniterm-ui/src/native_menu.rs` presents the rows this builds.
//!
//! Called by `infiniterm-ui` (the right-click listener asks `rows`). Related:
//! `when.rs` (the clause language, the same keys keybindings use),
//! `shortcuts.rs` (a row's shortcut hint), `model/register.rs` (the commands a
//! menu names must exist: a test checks every one). A menu never holds its own
//! behaviour: a row runs a registered command, so the palette, the keymap,
//! `ift` and the menu cannot drift.
//!
//! A row's title is the command's own label with its domain prefix cut
//! ("Card: rename\u{2026}" becomes "Rename\u{2026}"), unless the table gives one,
//! so the ellipsis that marks a command asking for more (#277) carries over.
//!
//! One place is not a table row: `Def::Git`, the terminal's git row (#754),
//! is read when the menu opens (`Source::git`): a Switch Branch submenu of
//! the card's repository, or Git Init where there is none. A branch row's
//! command is `BRANCH_PICK` and the ref name, which `Model::menu_branch_pick`
//! takes apart; it is the one row that is not a registered command, because
//! the branches are not known before the menu opens.

use crate::git_branches::Branch;
use crate::when::{Context, When};

/// The command of a branch row in the Switch Branch submenu, before the full
/// ref name (`git.switchBranch.to:refs/heads/main`).
pub const BRANCH_PICK: &str = "git.switchBranch.to:";

/// The command of the row a terminal outside any repository shows instead.
const GIT_INIT: &str = "git.init";

/// The place that was right-clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    /// The text of a terminal card.
    Terminal,
    /// A card's frame or label chip when the card has no menu of its own
    /// (a diff, a transcript, a page, a browser), and any card of a selection
    /// of several. For a terminal or an editor the frame and label open the
    /// card's own menu (`card_area`): a card is a card (Ekin, #421).
    Frame,
    /// Bare canvas.
    Canvas,
    /// A workspace tab in the title bar.
    Tab,
    /// An editor card's text.
    Editor,
    /// A row of an editor's file tree.
    Tree,
    /// A tab of an editor's or a browser's tab strip, and a browser page:
    /// the page's right-click is this menu with the rows of what it landed
    /// on added (`pageLink` ...), one table for both (#625).
    TabStrip,
}

/// One entry of a menu table.
#[derive(Clone, Copy, Debug)]
pub enum Def {
    Command(CommandDef),
    /// A submenu one level deep; its own entries are commands and separators.
    Submenu {
        title: &'static str,
        items: &'static [Def],
    },
    Separator,
    /// The card's git row, built when the menu opens (`git_rows`).
    Git,
}

#[derive(Clone, Copy, Debug)]
pub struct CommandDef {
    pub id: &'static str,
    /// Replaces the label-derived title.
    pub title: Option<&'static str>,
    /// The row is left out unless this holds (empty: always shown).
    pub show: &'static str,
    /// The row is greyed unless this holds (empty: always enabled).
    pub enable: &'static str,
    /// The shortcut to draw when the keymap has none: a body's own chord
    /// (Cmd+C and Cmd+V in a terminal are the terminal's, not commands).
    pub chord: &'static str,
}

const fn titled(id: &'static str, title: &'static str) -> Def {
    Def::Command(CommandDef {
        id,
        title: Some(title),
        show: "",
        enable: "",
        chord: "",
    })
}

const fn full(
    id: &'static str,
    title: Option<&'static str>,
    show: &'static str,
    enable: &'static str,
    chord: &'static str,
) -> Def {
    Def::Command(CommandDef {
        id,
        title,
        show,
        enable,
        chord,
    })
}

const NEW_CARD: Def = Def::Submenu {
    title: "New",
    items: &[
        titled("card.new.terminal", "Terminal"),
        titled("card.new.editor", "Editor"),
        titled("card.new.browser", "Browser\u{2026}"),
        titled("card.new.claude", "Claude Code"),
    ],
};

/// The size commands act on the focused card alone, so a selection of
/// several has no Size menu (#347).
const SIZE: Def = Def::Submenu {
    title: "Size",
    items: &[
        full("card.size", Some("Fractions\u{2026}"), ONE_CARD, "", ""),
        full(
            "card.size.reset",
            Some("Fill the Free Space"),
            ONE_CARD,
            "",
            "",
        ),
        full("card.maximize.toggle", Some("Maximize"), ONE_CARD, "", ""),
    ],
};

/// A `show` clause: the row is for one card, not a selection of several.
const ONE_CARD: &str = "!multiSelection";

/// How to look at the canvas, in the menu of a card: the keys Cmd+1, Cmd+2, a
/// selection's fit, Cmd+0, Tidy and Cmd+A. Fit Card is for one card, Fit
/// Selection for several (#419).
const CANVAS_SUB: Def = Def::Submenu {
    title: "Canvas",
    items: &[
        full("canvas.zoom.fitCard", Some("Fit Card"), ONE_CARD, "", ""),
        full(
            "canvas.zoom.fitGroup",
            Some("Fit Selection"),
            "multiSelection",
            "",
            "",
        ),
        titled("canvas.zoom.fitAll", "Fit All Cards"),
        titled("canvas.zoom.actual", "Actual Size"),
        Def::Separator,
        titled("canvas.tidy", "Tidy into a Block"),
        titled("focus.selectAll", "Select All Cards"),
    ],
};

/// What every card's menu ends with, in this order, whatever the card shows:
/// make another card, change this one's place and size, look at the canvas,
/// name and protect it, close it. The menus of a terminal's text, an
/// editor's text and a card's frame differ only above it (#419); before, each
/// had its own subset and the frame menu was the only one with Group.
const CARD_SECTION: &[Def] = &[
    Def::Separator,
    full("card.split.right", Some("Split Right"), ONE_CARD, "", ""),
    full("card.split.down", Some("Split Down"), ONE_CARD, "", ""),
    NEW_CARD,
    titled("group.new", "Group\u{2026}"),
    Def::Separator,
    SIZE,
    titled("card.moveToWorkspace", "Move to Workspace\u{2026}"),
    CANVAS_SUB,
    Def::Separator,
    full("card.rename", Some("Rename\u{2026}"), ONE_CARD, "", ""),
    titled("card.protect", "Protect"),
    full(
        "card.mask",
        Some("Mask"),
        "terminalFocus && !multiSelection",
        "",
        "",
    ),
    titled("card.clearState", "Clear State Colour"),
    Def::Separator,
    titled("card.close", "Close"),
];

const TERMINAL_TOP: &[Def] = &[
    full(
        "terminal.copy",
        Some("Copy"),
        "",
        "terminalHasSelection",
        "cmd+c",
    ),
    full("terminal.paste", Some("Paste"), "", "", "cmd+v"),
    Def::Separator,
    titled("card.find", "Find\u{2026}"),
    full(
        "terminal.searchSelection",
        Some("Search the Web for Selection"),
        "",
        "terminalHasSelection",
        "",
    ),
    Def::Separator,
    titled("card.clear", "Clear Buffer"),
    titled("terminal.redraw", "Redraw"),
    titled("card.transcript", "Show Transcript"),
    Def::Separator,
    Def::Git,
];

/// A card's frame or label has nothing of its own above the card section.
const FRAME_TOP: &[Def] = &[];

const CANVAS: &[Def] = &[
    NEW_CARD,
    Def::Separator,
    titled("canvas.zoom.fitAll", "Fit All Cards"),
    titled("canvas.zoom.actual", "Actual Size"),
    titled("canvas.tidy", "Tidy into a Block"),
    titled("focus.selectAll", "Select All Cards"),
    Def::Separator,
    titled("layout.undo", "Undo Layout"),
    titled("layout.redo", "Redo Layout"),
    titled("card.reopen", "Reopen Closed Card"),
    Def::Separator,
    titled("app.palette", "Command Palette\u{2026}"),
    titled("app.settings", "Settings"),
    titled("app.shortcuts", "Keyboard Shortcuts"),
];

const TAB: &[Def] = &[
    titled("workspace.rename", "Rename…"),
    titled("workspace.new", "New Workspace"),
    Def::Separator,
    titled("workspace.reorder.left", "Move Left"),
    titled("workspace.reorder.right", "Move Right"),
    Def::Separator,
    titled("workspace.close", "Close Workspace"),
];

const EDITOR_TOP: &[Def] = &[
    titled("card.save", "Save"),
    titled("card.find", "Find\u{2026}"),
    titled("editor.goToSymbol", "Go to Symbol\u{2026}"),
    titled("editor.goToFile", "Go to File\u{2026}"),
    titled("editor.goToLine", "Go to Line\u{2026}"),
    Def::Separator,
    Def::Submenu {
        title: "Transform",
        items: &[
            titled("editor.transform.upper", "UPPERCASE"),
            titled("editor.transform.lower", "lowercase"),
            titled("editor.transform.title", "Title Case"),
            titled("editor.transform.camel", "camelCase"),
            titled("editor.transform.snake", "snake_case"),
            titled("editor.transform.kebab", "kebab-case"),
            Def::Separator,
            titled("editor.transform.sortLines", "Sort Lines"),
            titled("editor.transform.uniqueLines", "Unique Lines"),
            titled("editor.transform.reverseLines", "Reverse Lines"),
            titled(
                "editor.transform.trimTrailingWhitespace",
                "Trim Trailing Whitespace",
            ),
            titled("editor.transform.removeBlankLines", "Remove Blank Lines"),
            titled("editor.transform.stripLineNumbers", "Strip Line Numbers"),
            Def::Separator,
            titled(
                "editor.transform.indentTabsToSpaces",
                "Indentation to Spaces",
            ),
            titled("editor.transform.indentSpacesToTabs", "Indentation to Tabs"),
        ],
    },
    titled("editor.explorer", "Show File Tree"),
    titled("editor.blame", "Blame"),
];

const TREE: &[Def] = &[
    titled("editor.tree.open", "Open"),
    Def::Separator,
    titled("editor.tree.reveal", "Reveal in Finder"),
    titled("editor.tree.copyPath", "Copy Path"),
    titled("editor.tree.copyRelativePath", "Copy Relative Path"),
];

/// A `show` clause: a browser card, and a right-click that is not on a link,
/// a field or selected text. From the tab strip all three are false, so the
/// strip shows the page-wide rows a plain click on the page shows (#625).
const PLAIN_PAGE: &str = "browserFocus && !pageLink && !pageEditable && !pageSelection";

/// The one menu of a browser card, for its tab strip and its page (#625).
/// The rows above the tab rows are the page's own: they follow what the
/// right-click landed on (`pageLink`, `pageEditable`, `pageSelection`, set
/// only while that menu is open), so the strip, which sets none of them,
/// shows the same menu without them. The editor's tab strip shares the
/// table; its rows are gated by `editorFocus`.
const TAB_STRIP: &[Def] = &[
    full("browser.back", Some("Back"), PLAIN_PAGE, "canGoBack", ""),
    full(
        "browser.forward",
        Some("Forward"),
        PLAIN_PAGE,
        "canGoForward",
        "",
    ),
    full("browser.reload", Some("Reload"), PLAIN_PAGE, "", ""),
    Def::Separator,
    full("browser.cut", Some("Cut"), "pageEditable", "", "cmd+x"),
    full(
        "browser.copy",
        Some("Copy"),
        "pageEditable || pageSelection",
        "",
        "cmd+c",
    ),
    full("browser.paste", Some("Paste"), "pageEditable", "", "cmd+v"),
    Def::Separator,
    full(
        "browser.link.newTab",
        Some("Open Link in New Tab"),
        "pageLink",
        "",
        "",
    ),
    full(
        "browser.link.newCard",
        Some("Open Link in New Card"),
        "pageLink",
        "",
        "",
    ),
    full(
        "browser.link.copy",
        Some("Copy Link Address"),
        "pageLink",
        "",
        "",
    ),
    full(
        "browser.link.external",
        Some("Open Link in System Browser"),
        "pageLink",
        "",
        "",
    ),
    Def::Separator,
    full(
        "editor.tab.new",
        Some("New Tab"),
        "editorFocus",
        "",
        "cmd+t",
    ),
    full(
        "browser.tab.new",
        Some("New Tab"),
        "browserFocus",
        "",
        "cmd+t",
    ),
    Def::Separator,
    full(
        "editor.tab.close",
        Some("Close Tab"),
        "editorFocus",
        "",
        "cmd+w",
    ),
    full(
        "browser.tab.close",
        Some("Close Tab"),
        "browserFocus",
        "",
        "cmd+w",
    ),
    full(
        "editor.tab.reopenClosed",
        Some("Reopen Closed Tab"),
        "editorFocus",
        "",
        "cmd+shift+t",
    ),
    full(
        "browser.tab.reopenClosed",
        Some("Reopen Closed Tab"),
        "browserFocus",
        "",
        "cmd+shift+t",
    ),
    full(
        "editor.tab.moveToCard",
        Some("Move Tab to Card…"),
        "editorFocus",
        "",
        "",
    ),
    full(
        "editor.card.moveIntoCard",
        Some("Move Card into Card…"),
        "editorFocus",
        "",
        "",
    ),
    Def::Separator,
    full(
        "browser.copyUrl",
        Some("Copy Address"),
        "browserFocus",
        "",
        "",
    ),
    full(
        "browser.external",
        Some("Open in System Browser"),
        "browserFocus && !pageLink",
        "",
        "",
    ),
];

/// The menu a click on a card's frame or label chip opens: the same one its
/// body opens, so where on the card you click does not change what you are
/// offered. A kind with no menu of its own gets the generic one.
pub fn card_area(kind: crate::saved_layout::CardKind) -> Area {
    use crate::saved_layout::CardKind;
    match kind {
        CardKind::Terminal => Area::Terminal,
        CardKind::Editor => Area::Editor,
        _ => Area::Frame,
    }
}

pub fn table(area: Area) -> Vec<Def> {
    match area {
        Area::Terminal => [TERMINAL_TOP, CARD_SECTION].concat(),
        Area::Frame => [FRAME_TOP, CARD_SECTION].concat(),
        Area::Canvas => CANVAS.to_vec(),
        Area::Tab => TAB.to_vec(),
        Area::Editor => [EDITOR_TOP, CARD_SECTION].concat(),
        Area::Tree => TREE.to_vec(),
        Area::TabStrip => TAB_STRIP.to_vec(),
    }
}

/// A row of a built menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Item {
        command: String,
        title: String,
        /// The chord to draw, as the keymap spells it.
        chord: Option<String>,
        enabled: bool,
    },
    Submenu {
        title: String,
        rows: Vec<Row>,
    },
    Separator,
}

/// What a menu needs from the app to be built.
pub struct Source<'a> {
    pub context: &'a Context,
    /// The command's label, `None` for an unknown id (the row is dropped).
    pub label: &'a dyn Fn(&str) -> Option<String>,
    /// The first chord the keymap binds to a command, as the platform shows
    /// it.
    pub chord: &'a dyn Fn(&str) -> Option<String>,
    /// A row's own chord from the table (a body's, like the terminal's copy)
    /// as the platform shows it: `Model::shown_chord`, so off the Mac the
    /// terminal's Cmd+C is drawn Ctrl+Shift+C (#464).
    pub shown: &'a dyn Fn(&str) -> Option<String>,
    /// The branches of the focused card's repository, asked only by a menu
    /// with a `Def::Git` row; `None`: the card's folder is in no repository.
    pub git: &'a dyn Fn() -> Option<Vec<Branch>>,
}

/// "Card: rename\u{2026}" to "Rename\u{2026}": the part after the domain, its first
/// letter in capitals.
pub fn short_title(label: &str) -> String {
    let rest = label.split_once(": ").map_or(label, |(_, r)| r);
    let mut chars = rest.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn holds(clause: &str, ctx: &Context) -> bool {
    clause.is_empty() || When::parse(clause).is_ok_and(|w| w.eval(ctx))
}

fn build(defs: &[Def], src: &Source) -> Vec<Row> {
    let mut rows = vec![];
    for def in defs {
        match def {
            Def::Separator => rows.push(Row::Separator),
            Def::Command(c) => {
                if !holds(c.show, src.context) {
                    continue;
                }
                let Some(label) = (src.label)(c.id) else {
                    continue;
                };
                let chord = (src.chord)(c.id).or_else(|| {
                    (!c.chord.is_empty())
                        .then(|| (src.shown)(c.chord))
                        .flatten()
                });
                rows.push(Row::Item {
                    command: c.id.into(),
                    title: c.title.map_or_else(|| short_title(&label), String::from),
                    chord,
                    enabled: holds(c.enable, src.context),
                });
            }
            Def::Git => rows.extend(git_rows((src.git)().as_deref())),
            Def::Submenu { title, items } => {
                let inner = tidy(build(items, src));
                if !inner.is_empty() {
                    rows.push(Row::Submenu {
                        title: (*title).into(),
                        rows: inner,
                    });
                }
            }
        }
    }
    tidy(rows)
}

/// The git row (#754): Switch Branch with the local branches, the current
/// one greyed, then the remote ones with no local branch (`git_branches::
/// parse` already orders and filters them); Git Init outside a repository. A
/// repository with no commit yet has no branch to switch to and no need of
/// an init, so it gets no row.
pub fn git_rows(branches: Option<&[Branch]>) -> Vec<Row> {
    let Some(branches) = branches else {
        return vec![Row::Item {
            command: GIT_INIT.into(),
            title: "Git Init".into(),
            chord: None,
            enabled: true,
        }];
    };
    let item = |b: &Branch| Row::Item {
        command: format!("{BRANCH_PICK}{}", b.refname),
        title: if b.current {
            format!("{} (current)", b.name)
        } else {
            b.name.clone()
        },
        chord: None,
        enabled: !b.current,
    };
    let mut rows: Vec<Row> = branches.iter().filter(|b| !b.remote).map(item).collect();
    rows.push(Row::Separator);
    rows.extend(branches.iter().filter(|b| b.remote).map(item));
    let rows = tidy(rows);
    if rows.is_empty() {
        return vec![];
    }
    vec![Row::Submenu {
        title: "Switch Branch".into(),
        rows,
    }]
}

/// No separator first, last or twice in a row: hiding a row can leave one
/// stranded.
fn tidy(rows: Vec<Row>) -> Vec<Row> {
    let mut out: Vec<Row> = vec![];
    for row in rows {
        if row == Row::Separator && matches!(out.last(), None | Some(Row::Separator)) {
            continue;
        }
        out.push(row);
    }
    while out.last() == Some(&Row::Separator) {
        out.pop();
    }
    out
}

/// The menu for `area`, as the app is now.
pub fn rows(area: Area, src: &Source) -> Vec<Row> {
    build(&table(area), src)
}

/// Every command id a table names, submenus included: for the test that
/// checks they are all registered.
pub fn command_ids(area: Area) -> Vec<&'static str> {
    fn walk(defs: &[Def], out: &mut Vec<&'static str>) {
        for d in defs {
            match d {
                Def::Command(c) => out.push(c.id),
                Def::Submenu { items, .. } => walk(items, out),
                Def::Git => out.push(GIT_INIT),
                Def::Separator => {}
            }
        }
    }
    let mut out = vec![];
    walk(&table(area), &mut out);
    out
}

/// Every `show` and `enable` clause a table uses, to check they parse.
pub fn clauses(area: Area) -> Vec<&'static str> {
    fn walk(defs: &[Def], out: &mut Vec<&'static str>) {
        for d in defs {
            match d {
                Def::Command(c) => {
                    out.extend([c.show, c.enable].into_iter().filter(|s| !s.is_empty()));
                }
                Def::Submenu { items, .. } => walk(items, out),
                Def::Separator | Def::Git => {}
            }
        }
    }
    let mut out = vec![];
    walk(&table(area), &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        Context::terminal_focused()
    }

    fn built(area: Area, ctx: &Context) -> Vec<Row> {
        rows(
            area,
            &Source {
                context: ctx,
                label: &|id| Some(format!("Card: {id}")),
                chord: &|id| (id == "card.close").then(|| "cmd+w".to_string()),
                shown: &|c| Some(c.to_string()),
                git: &|| None,
            },
        )
    }

    // A body chord in the table is drawn as the platform shows it, through
    // `shown`; one with no key there is drawn without a chord.
    #[test]
    fn a_table_chord_goes_through_the_platform_view() {
        let ctx = ctx();
        let built = rows(
            Area::Terminal,
            &Source {
                context: &ctx,
                label: &|id| Some(format!("Card: {id}")),
                chord: &|_| None,
                shown: &|c| (c == "cmd+v").then(|| "ctrl+shift+v".to_string()),
                git: &|| None,
            },
        );
        let chord_of = |id: &str| {
            built.iter().find_map(|r| match r {
                Row::Item { command, chord, .. } if command == id => Some(chord.clone()),
                _ => None,
            })
        };
        assert_eq!(
            chord_of("terminal.paste"),
            Some(Some("ctrl+shift+v".into()))
        );
        assert_eq!(chord_of("terminal.copy"), Some(None));
    }

    #[test]
    fn a_title_is_the_label_without_its_domain_in_capitals() {
        assert_eq!(short_title("Card: rename\u{2026}"), "Rename\u{2026}");
        assert_eq!(
            short_title("Run a command\u{2026}"),
            "Run a command\u{2026}"
        );
        assert_eq!(short_title("Terminal: new card"), "New card");
        assert_eq!(short_title(""), "");
    }

    #[test]
    fn every_clause_in_every_table_parses() {
        for area in [
            Area::Terminal,
            Area::Frame,
            Area::Canvas,
            Area::Tab,
            Area::Editor,
            Area::Tree,
            Area::TabStrip,
        ] {
            for c in clauses(area) {
                assert!(When::parse(c).is_ok(), "{area:?}: {c}");
            }
        }
    }

    #[test]
    fn a_disabled_row_is_kept_greyed_and_the_keymap_chord_wins_over_the_table() {
        let rows = built(Area::Terminal, &ctx());
        let copy = rows
            .iter()
            .find_map(|r| match r {
                Row::Item {
                    command,
                    enabled,
                    chord,
                    ..
                } if command == "terminal.copy" => Some((*enabled, chord.clone())),
                _ => None,
            })
            .expect("copy is in a terminal's menu");
        assert_eq!(
            copy,
            (false, Some("cmd+c".into())),
            "no selection: greyed, hint from the table"
        );
        let with = Context {
            terminal_has_selection: true,
            ..ctx()
        };
        let rows = built(Area::Terminal, &with);
        assert!(rows.iter().any(
            |r| matches!(r, Row::Item { command, enabled: true, .. } if command == "terminal.copy")
        ));
        // the close row takes the keymap's chord
        assert!(rows.iter().any(|r| matches!(r,
            Row::Item { command, chord: Some(c), .. } if command == "card.close" && c == "cmd+w")));
    }

    #[test]
    fn a_menu_never_starts_or_ends_with_a_separator_or_doubles_one() {
        for area in [
            Area::Terminal,
            Area::Frame,
            Area::Canvas,
            Area::Tab,
            Area::Editor,
            Area::Tree,
            Area::TabStrip,
        ] {
            // an editor card focused: the tab strip's rows are per card kind
            let editor = Context {
                card_kind: "editor",
                ..ctx()
            };
            let rows = built(area, &editor);
            assert!(!rows.is_empty(), "{area:?}");
            assert_ne!(rows.first(), Some(&Row::Separator));
            assert_ne!(rows.last(), Some(&Row::Separator));
            assert!(rows
                .windows(2)
                .all(|w| w != [Row::Separator, Row::Separator]));
        }
    }

    #[test]
    fn an_unknown_command_is_dropped_and_an_emptied_submenu_goes_with_it() {
        let rows = rows(
            Area::Canvas,
            &Source {
                context: &ctx(),
                label: &|_| None,
                chord: &|_| None,
                shown: &|_| None,
                git: &|| None,
            },
        );
        assert!(rows.is_empty(), "{rows:?}");
    }

    fn branch(refname: &str, current: bool) -> Branch {
        Branch {
            refname: refname.into(),
            name: crate::git_branches::short_name(refname).into(),
            remote: refname.starts_with("refs/remotes/"),
            current,
        }
    }

    // #754: a terminal in a repository gets Switch Branch, the current branch
    // greyed and the remotes after a separator, each row's command carrying
    // the full ref; outside a repository, Git Init; with no commit, nothing.
    #[test]
    fn the_terminal_menu_switches_branch_or_inits_a_repository() {
        let ctx = ctx();
        let branches = vec![
            branch("refs/heads/main", true),
            branch("refs/heads/dev", false),
            branch("refs/remotes/origin/x", false),
        ];
        let built = rows(
            Area::Terminal,
            &Source {
                context: &ctx,
                label: &|id| Some(format!("Card: {id}")),
                chord: &|_| None,
                shown: &|_| None,
                git: &|| Some(branches.clone()),
            },
        );
        let sub = built
            .iter()
            .find_map(|r| match r {
                Row::Submenu { title, rows } if title == "Switch Branch" => Some(rows.clone()),
                _ => None,
            })
            .expect("a Switch Branch submenu");
        let item = |command: &str, title: &str, enabled: bool| Row::Item {
            command: command.into(),
            title: title.into(),
            chord: None,
            enabled,
        };
        assert_eq!(
            sub,
            [
                item(
                    "git.switchBranch.to:refs/heads/main",
                    "main (current)",
                    false
                ),
                item("git.switchBranch.to:refs/heads/dev", "dev", true),
                Row::Separator,
                item(
                    "git.switchBranch.to:refs/remotes/origin/x",
                    "origin/x",
                    true
                ),
            ]
        );
        assert!(!built
            .iter()
            .any(|r| matches!(r, Row::Item { command, .. } if command == "git.init")));
        // Not a repository: Git Init in its place.
        assert!(built_without_repo()
            .iter()
            .any(|r| matches!(r, Row::Item { command, title, .. }
                if command == "git.init" && title == "Git Init")));
        assert_eq!(git_rows(Some(&[])), vec![]);
        // Only local branches: no trailing separator.
        assert_eq!(
            git_rows(Some(&[branch("refs/heads/main", true)])),
            [Row::Submenu {
                title: "Switch Branch".into(),
                rows: vec![item(
                    "git.switchBranch.to:refs/heads/main",
                    "main (current)",
                    false
                )],
            }]
        );
        // An editor's menu has no git row.
        assert!(!command_ids(Area::Editor).contains(&"git.init"));
    }

    fn built_without_repo() -> Vec<Row> {
        built(Area::Terminal, &ctx())
    }

    #[test]
    fn submenus_carry_their_own_titles_and_one_level_of_rows() {
        let rows = built(Area::Terminal, &ctx());
        let new = rows
            .iter()
            .find_map(|r| match r {
                Row::Submenu { title, rows } if title == "New" => Some(rows),
                _ => None,
            })
            .expect("a New submenu");
        let titles: Vec<&str> = new
            .iter()
            .filter_map(|r| match r {
                Row::Item { title, .. } => Some(title.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            titles,
            ["Terminal", "Editor", "Browser\u{2026}", "Claude Code"]
        );
    }

    // #419: a terminal's text, an editor's text and a card's frame end with the
    // same section, so a row cannot be in one card menu and missing in another.
    // #421: a click on a terminal's or an editor's frame or label opens the
    // card's own menu, not a different one.
    #[test]
    fn the_frame_of_a_terminal_or_an_editor_opens_that_cards_menu() {
        use crate::saved_layout::CardKind;
        assert_eq!(card_area(CardKind::Terminal), Area::Terminal);
        assert_eq!(card_area(CardKind::Editor), Area::Editor);
        for kind in [
            CardKind::Browser,
            CardKind::Diff,
            CardKind::Transcript,
            CardKind::Page,
        ] {
            assert_eq!(card_area(kind), Area::Frame, "{kind:?}");
        }
    }

    #[test]
    fn the_card_menus_end_with_the_same_section() {
        let c = ctx();
        let from_split = |area: Area| -> Vec<Row> {
            let rows = built(area, &c);
            let at = rows
                .iter()
                .position(
                    |r| matches!(r, Row::Item { command, .. } if command == "card.split.right"),
                )
                .expect("every card menu has Split Right");
            rows[at..].to_vec()
        };
        let frame = from_split(Area::Frame);
        assert_eq!(from_split(Area::Terminal), frame);
        assert_eq!(from_split(Area::Editor), frame);
        // And the section holds what was missing: Fit and Tidy and Select All
        // in a Canvas submenu, Group, Move to Workspace and Protect.
        let canvas = frame
            .iter()
            .find_map(|r| match r {
                Row::Submenu { title, rows } if title == "Canvas" => Some(rows),
                _ => None,
            })
            .expect("a Canvas submenu");
        let ids: Vec<&str> = canvas
            .iter()
            .filter_map(|r| match r {
                Row::Item { command, .. } => Some(command.as_str()),
                _ => None,
            })
            .collect();
        for want in [
            "canvas.zoom.fitCard",
            "canvas.zoom.fitAll",
            "canvas.zoom.actual",
            "canvas.tidy",
            "focus.selectAll",
        ] {
            assert!(
                ids.contains(&want),
                "{want} is in the Canvas submenu: {ids:?}"
            );
        }
        let top: Vec<&str> = frame
            .iter()
            .filter_map(|r| match r {
                Row::Item { command, .. } => Some(command.as_str()),
                _ => None,
            })
            .collect();
        for want in [
            "group.new",
            "card.moveToWorkspace",
            "card.protect",
            "card.close",
        ] {
            assert!(top.contains(&want), "{want}: {top:?}");
        }
    }

    // The bare canvas menu keeps the view rows at the top level and gains the
    // layout's undo and redo and the closed card's return.
    #[test]
    fn the_canvas_menu_has_fit_tidy_select_all_and_the_undo_rows() {
        let rows = built(Area::Canvas, &ctx());
        let ids: Vec<String> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Item { command, .. } => Some(command.clone()),
                _ => None,
            })
            .collect();
        for want in [
            "canvas.zoom.fitAll",
            "canvas.zoom.actual",
            "canvas.tidy",
            "focus.selectAll",
            "layout.undo",
            "layout.redo",
            "card.reopen",
        ] {
            assert!(ids.iter().any(|i| i == want), "{want}: {ids:?}");
        }
    }

    #[test]
    fn a_selection_of_several_cards_gets_the_rows_that_act_on_all_of_them() {
        let ids = |rows: &[Row]| -> Vec<String> {
            fn walk(rows: &[Row], out: &mut Vec<String>) {
                for r in rows {
                    match r {
                        Row::Item { command, .. } => out.push(command.clone()),
                        Row::Submenu { rows, .. } => walk(rows, out),
                        Row::Separator => {}
                    }
                }
            }
            let mut out = vec![];
            walk(rows, &mut out);
            out
        };
        let one = ids(&built(Area::Frame, &ctx()));
        let many = ids(&built(
            Area::Frame,
            &Context {
                multi_selection: true,
                ..ctx()
            },
        ));
        for id in [
            "card.rename",
            "card.split.right",
            "card.size",
            "card.maximize.toggle",
            "card.mask",
            "canvas.zoom.fitCard",
        ] {
            assert!(one.iter().any(|c| c == id), "{id} for one card");
            assert!(!many.iter().any(|c| c == id), "{id} hidden for several");
        }
        assert!(!one.iter().any(|c| c == "canvas.zoom.fitGroup"));
        for id in [
            "canvas.zoom.fitGroup",
            "card.moveToWorkspace",
            "group.new",
            "card.protect",
            "card.clearState",
            "card.close",
        ] {
            assert!(many.iter().any(|c| c == id), "{id} for several");
        }
    }

    fn browser() -> Context {
        Context {
            card_kind: "browser",
            ..ctx()
        }
    }

    fn commands(rows: &[Row]) -> Vec<(String, bool)> {
        rows.iter()
            .filter_map(|r| match r {
                Row::Item {
                    command, enabled, ..
                } => Some((command.clone(), *enabled)),
                _ => None,
            })
            .collect()
    }

    // #625: the tab strip and the page are one menu. A plain click on the page
    // is the strip's menu; a click on something adds rows over it and never
    // takes the strip's away.
    #[test]
    fn a_plain_click_on_the_page_is_the_tab_strips_menu() {
        let strip = built(Area::TabStrip, &browser());
        let ids: Vec<_> = commands(&strip).into_iter().map(|(c, _)| c).collect();
        for id in [
            "browser.back",
            "browser.forward",
            "browser.reload",
            "browser.tab.new",
            "browser.tab.close",
            "browser.tab.reopenClosed",
            "browser.copyUrl",
            "browser.external",
        ] {
            assert!(ids.iter().any(|c| c == id), "{id} in the strip's menu");
        }
        for id in ["browser.cut", "browser.link.copy", "browser.link.newTab"] {
            assert!(!ids.iter().any(|c| c == id), "{id} is for a click on it");
        }
    }

    #[test]
    fn history_rows_follow_the_pages_history() {
        let none = commands(&built(Area::TabStrip, &browser()));
        assert!(none.contains(&("browser.back".into(), false)));
        assert!(none.contains(&("browser.forward".into(), false)));
        let some = commands(&built(
            Area::TabStrip,
            &Context {
                can_go_back: true,
                ..browser()
            },
        ));
        assert!(some.contains(&("browser.back".into(), true)));
        assert!(some.contains(&("browser.forward".into(), false)));
    }

    #[test]
    fn a_link_adds_its_rows_and_swaps_the_page_rows_for_them() {
        let on_link = commands(&built(
            Area::TabStrip,
            &Context {
                page_link: true,
                ..browser()
            },
        ));
        let ids: Vec<_> = on_link.into_iter().map(|(c, _)| c).collect();
        for id in [
            "browser.link.newTab",
            "browser.link.newCard",
            "browser.link.copy",
            "browser.link.external",
            "browser.tab.close",
            "browser.copyUrl",
        ] {
            assert!(ids.iter().any(|c| c == id), "{id} on a link");
        }
        for id in ["browser.back", "browser.reload", "browser.external"] {
            assert!(!ids.iter().any(|c| c == id), "{id} is not for a link");
        }
    }

    #[test]
    fn a_field_gets_cut_copy_paste_and_a_selection_only_copy() {
        let ids = |ctx: Context| -> Vec<String> {
            commands(&built(Area::TabStrip, &ctx))
                .into_iter()
                .map(|(c, _)| c)
                .collect()
        };
        let field = ids(Context {
            page_editable: true,
            ..browser()
        });
        for id in ["browser.cut", "browser.copy", "browser.paste"] {
            assert!(field.iter().any(|c| c == id), "{id} in a field");
        }
        let text = ids(Context {
            page_selection: true,
            ..browser()
        });
        assert!(text.iter().any(|c| c == "browser.copy"));
        assert!(!text
            .iter()
            .any(|c| c == "browser.cut" || c == "browser.paste"));
    }

    #[test]
    fn the_editors_strip_has_none_of_the_pages_rows() {
        let editor = Context {
            card_kind: "editor",
            ..ctx()
        };
        let ids: Vec<_> = commands(&built(Area::TabStrip, &editor))
            .into_iter()
            .map(|(c, _)| c)
            .collect();
        assert!(!ids.iter().any(|c| c.starts_with("browser.")));
    }
}
