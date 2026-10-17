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

use crate::when::{Context, When};

/// The place that was right-clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    /// The text of a terminal card.
    Terminal,
    /// A card's frame or its label chip.
    Frame,
    /// Bare canvas.
    Canvas,
    /// A workspace tab in the title bar.
    Tab,
    /// An editor card's text.
    Editor,
    /// A row of an editor's file tree.
    Tree,
    /// A tab of an editor's or a browser's tab strip.
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
    titled("card.transcript", "Show Transcript"),
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

const TAB_STRIP: &[Def] = &[
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
    Def::Separator,
    full("browser.reload", Some("Reload"), "browserFocus", "", ""),
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
        "browserFocus",
        "",
        "",
    ),
];

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
    /// The first chord the keymap binds to a command.
    pub chord: &'a dyn Fn(&str) -> Option<String>,
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
                let chord =
                    (src.chord)(c.id).or_else(|| (!c.chord.is_empty()).then(|| c.chord.into()));
                rows.push(Row::Item {
                    command: c.id.into(),
                    title: c.title.map_or_else(|| short_title(&label), String::from),
                    chord,
                    enabled: holds(c.enable, src.context),
                });
            }
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
                Def::Separator => {}
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
            },
        )
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
            },
        );
        assert!(rows.is_empty(), "{rows:?}");
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
}
