//! The settings schema: for every settings.json key, the kind of value it
//! takes and the section it belongs to (#745).
//!
//! Where it fits: the settings window (`ui.settings`, #745) draws one row
//! per entry here, and the kind picks the control: a checkbox for `Bool`, a
//! list for `Choice`, a text field for `Number` and `Text`, a hex field with
//! a swatch for `Color`, and so on. Nothing reads it yet; it lands first, on
//! its own, so the window that follows is wiring (Ekin, #745: schema with
//! tests, then the checkbox and rows, then the view).
//!
//! What it is not: validation. `validate_settings` and `merge_config` stay
//! the only judges of a value, so the window writes through the same path as
//! a hand edit and a value the file accepts is never refused by the window.
//! A `Choice` lists the words the window offers, not the only words the file
//! may hold.
//!
//! Related: `settings_doc.rs` (what each key is for; the tests here hold
//! that every key has both a doc line and a schema row), `config.rs` (the
//! values and the enums the choices mirror), `ui_theme::Kind` (the same idea
//! for ui theme keys, which the window's theme section will use).
//!
//! Constraints: pure, no gpui, works on every system. A new setting needs a
//! row here or `every_setting_has_exactly_one_row` fails, as it needs a
//! `settings_doc.rs` entry.

/// What a key holds, which is also the control that edits it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Number,
    /// A number, or `null` for unset (it then follows another key, as
    /// `editor.fontSize` follows `terminal.fontSize`).
    OptionalNumber,
    Text,
    /// One of a fixed set of words, offered as a list.
    Choice(&'static [&'static str]),
    /// `#rgb`, `#rrggbb` or `#rrggbbaa`; empty is unset.
    Color,
    /// A font family name; the window offers the installed fonts.
    Font,
    /// A terminal colour scheme from the themes folder (`theme`).
    TerminalTheme,
    /// A file name in `ui-themes/` (`ui.theme`).
    UiTheme,
    /// A folder on disk; empty is unset.
    Folder,
    /// A list of strings, one row each (`terminal.env`).
    TextList,
    /// A list of numbers, one row each (`editor.rulers`).
    NumberList,
    /// `browser.engines`: a list of `{keyword, name, searchUrl}`.
    SearchEngines,
}

/// The window's left menu, in the order it lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    General,
    Terminal,
    Cards,
    Canvas,
    Labels,
    Window,
    Editor,
    Browser,
    Notifications,
    Performance,
}

impl Section {
    pub const ALL: [Section; 10] = [
        Section::General,
        Section::Terminal,
        Section::Cards,
        Section::Canvas,
        Section::Labels,
        Section::Window,
        Section::Editor,
        Section::Browser,
        Section::Notifications,
        Section::Performance,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Terminal => "Terminal",
            Section::Cards => "Cards",
            Section::Canvas => "Canvas",
            Section::Labels => "Labels",
            Section::Window => "Window",
            Section::Editor => "Editor",
            Section::Browser => "Browser",
            Section::Notifications => "Notifications",
            Section::Performance => "Performance",
        }
    }
}

/// The words a `Choice` offers, as `config.rs` spells them in the file.
/// `choices_round_trip_through_the_real_merge` holds each to its enum.
const BACKENDS: &[&str] = &["daemon", "pty", "tmux"];
const CURSOR_STYLES: &[&str] = &["block", "bar", "underline"];
const WRAPS: &[&str] = &["prose", "always", "never"];
const AUTOSAVES: &[&str] = &["off", "focusChange"];
const LABEL_CORNERS: &[&str] = &["top right", "top left", "bottom left", "bottom right"];
const SIDES: &[&str] = &["left", "right"];
const FULLSCREENS: &[&str] = &["cover", "native"];
const COMMAND_MODIFIERS: &[&str] = &["ctrl", "super"];

use Kind::*;
use Section::*;

/// Every settings.json key, its kind and its section, in the order the
/// window lists them within a section (the defaults file's order).
pub const SETTINGS_SCHEMA: &[(&str, Kind, Section)] = &[
    ("theme", TerminalTheme, General),
    ("ui.theme", UiTheme, General),
    ("startingDir", Folder, General),
    (
        "keyboard.commandModifier",
        Choice(COMMAND_MODIFIERS),
        General,
    ),
    ("join.enabled", Bool, General),
    ("ui.hideRemoteBranches", Bool, General),
    ("ui.switcherStateColors", Bool, General),
    ("ui.showWhatsNew", Bool, General),
    ("terminal.shell", Text, Terminal),
    ("terminal.backend", Choice(BACKENDS), Terminal),
    ("terminal.cursorStyle", Choice(CURSOR_STYLES), Terminal),
    ("terminal.cursorBlink", Bool, Terminal),
    ("terminal.cursorColor", Color, Terminal),
    ("terminal.fontFamily", Font, Terminal),
    ("terminal.fontSize", Number, Terminal),
    ("terminal.fontWeight", Text, Terminal),
    ("terminal.fontWeightBold", Text, Terminal),
    ("terminal.letterSpacing", Number, Terminal),
    ("terminal.lineHeight", Number, Terminal),
    ("terminal.scrollback", Number, Terminal),
    ("terminal.sessionBuffer", Number, Terminal),
    ("terminal.decoyCommand", Text, Terminal),
    ("terminal.copyOnSelect", Bool, Terminal),
    ("terminal.reverseLinkClick", Bool, Terminal),
    ("terminal.scrollMultiplier", Number, Terminal),
    ("terminal.padding", Number, Terminal),
    ("terminal.env", TextList, Terminal),
    ("cards.width", Number, Cards),
    ("cards.height", Number, Cards),
    ("cards.shape", Text, Cards),
    ("cards.inheritDirectory", Bool, Cards),
    ("cards.newCardInheritsDirectory", Bool, Cards),
    ("cards.gap", Number, Cards),
    ("cards.savedWorkspaceKeepsAgentSessions", Bool, Cards),
    ("canvas.zoomSensitivity", Number, Canvas),
    ("canvas.momentum", Bool, Canvas),
    ("ui.fitSplitSlot", Bool, Canvas),
    ("ui.fitMagnify", Bool, Canvas),
    ("ui.fitPadding", Number, Canvas),
    ("ui.animations", Bool, Canvas),
    ("ui.panStep", Number, Canvas),
    ("ui.workspaceIsolation", Bool, Canvas),
    ("ui.cardZoom", Number, Canvas),
    ("ui.cardLabelSize", Number, Labels),
    ("ui.cardLabelPosition", Choice(LABEL_CORNERS), Labels),
    ("ui.hideLabelWhenMaximised", Bool, Labels),
    ("ui.groupLabelSize", Number, Labels),
    ("ui.fullscreen", Choice(FULLSCREENS), Window),
    ("ui.statusBarPwd", Bool, Window),
    ("ui.statusBarPwdSide", Choice(SIDES), Window),
    ("ui.statusBarSize", Number, Window),
    ("ui.hideVersion", Bool, Window),
    ("ui.hideBuildNumber", Bool, Window),
    ("editor.highlightLine", Bool, Editor),
    ("editor.completeWords", Bool, Editor),
    ("editor.folderScratchTab", Bool, Editor),
    ("editor.explorer.icons", Bool, Editor),
    ("editor.explorer.git", Bool, Editor),
    ("editor.fontFamily", Font, Editor),
    ("editor.fontSize", OptionalNumber, Editor),
    ("editor.fontWeight", Text, Editor),
    ("editor.fontWeightBold", Text, Editor),
    ("editor.formatOnSave", Bool, Editor),
    ("editor.lsp.enabled", Bool, Editor),
    ("editor.selectionColor", Color, Editor),
    ("editor.selectionTextColor", Color, Editor),
    ("editor.cursorColor", Color, Editor),
    ("editor.wrap", Choice(WRAPS), Editor),
    ("editor.highlightOccurrences", Bool, Editor),
    ("editor.indentGuides", Bool, Editor),
    ("editor.smoothCaret", Bool, Editor),
    ("editor.rulers", NumberList, Editor),
    ("editor.whitespaceInSelection", Bool, Editor),
    ("editor.stickyScroll", Bool, Editor),
    ("editor.autosave", Choice(AUTOSAVES), Editor),
    ("editor.trimTrailingWhitespaceOnSave", Bool, Editor),
    ("editor.insertFinalNewlineOnSave", Bool, Editor),
    ("editor.spellcheck", Bool, Editor),
    ("browser.zoom", Number, Browser),
    ("browser.searchEngine", Text, Browser),
    ("browser.suggestions", Bool, Browser),
    ("browser.devtools", Bool, Browser),
    ("browser.engines", SearchEngines, Browser),
    ("notify.ntfy.server", Text, Notifications),
    ("notify.waiting", Bool, Notifications),
    ("notify.failed", Bool, Notifications),
    ("notify.done", Bool, Notifications),
    ("notify.skipWhenFocused", Bool, Notifications),
    ("ui.showFps", Bool, Performance),
    ("ui.glyphBudget", Number, Performance),
    ("ui.minTextPx", Number, Performance),
    ("ui.textAsBars", Bool, Performance),
];

/// The kind of `key`, if it is a settings key.
pub fn kind_of(key: &str) -> Option<Kind> {
    SETTINGS_SCHEMA
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, kind, _)| *kind)
}

/// The keys of one section, in the window's order.
pub fn keys_in(section: Section) -> impl Iterator<Item = &'static str> {
    SETTINGS_SCHEMA
        .iter()
        .filter(move |(_, _, s)| *s == section)
        .map(|(k, _, _)| *k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{default_config, merge_config};
    use crate::settings_doc::SETTINGS_DOC;
    use serde_json::{json, Value};

    /// Every settings.json key of the defaults as a dotted key and its
    /// default value, walked the way `settings_doc::undocumented` walks
    /// them. The look keys are `config::Ui` fields too, but they live in the
    /// ui theme file (#511) and `ui_theme::LOOK_KEYS` is their schema.
    fn leaves() -> Vec<(String, Value)> {
        let mut out = all_leaves();
        out.retain(|(k, _)| !crate::ui_theme::LOOK_KEYS.iter().any(|(l, _)| l == k));
        out
    }

    fn all_leaves() -> Vec<(String, Value)> {
        fn walk(value: &Value, prefix: &str, out: &mut Vec<(String, Value)>) {
            match value {
                Value::Object(map) => {
                    for (k, v) in map {
                        let key = if prefix.is_empty() {
                            k.clone()
                        } else {
                            format!("{prefix}.{k}")
                        };
                        walk(v, &key, out);
                    }
                }
                other => out.push((prefix.to_string(), other.clone())),
            }
        }
        let mut out = Vec::new();
        walk(
            &serde_json::to_value(default_config()).unwrap(),
            "",
            &mut out,
        );
        out
    }

    /// The value of a dotted key in a config, read from its JSON.
    fn value_at(config: &crate::config::Config, key: &str) -> Value {
        let mut v = serde_json::to_value(config).unwrap();
        // `notify.ntfy.server` is one field named `ntfy.server`.
        let mut rest = key;
        while let Some(obj) = v.as_object() {
            if let Some(found) = obj.get(rest) {
                return found.clone();
            }
            let (head, tail) = rest.split_once('.').expect("key is in the config");
            v = obj.get(head).expect("group is in the config").clone();
            rest = tail;
        }
        v
    }

    #[test]
    fn every_setting_has_exactly_one_row() {
        let leaves: Vec<String> = leaves().into_iter().map(|(k, _)| k).collect();
        let missing: Vec<&String> = leaves.iter().filter(|k| kind_of(k).is_none()).collect();
        assert!(missing.is_empty(), "keys with no schema row: {missing:?}");
        for (key, _, _) in SETTINGS_SCHEMA {
            assert!(
                leaves.iter().any(|l| l == key),
                "a row for a key the config does not have: {key}"
            );
            let rows = SETTINGS_SCHEMA.iter().filter(|(k, _, _)| k == key).count();
            assert_eq!(rows, 1, "{key} has {rows} rows");
        }
    }

    #[test]
    fn every_row_has_a_doc_line() {
        for (key, _, _) in SETTINGS_SCHEMA {
            assert!(
                SETTINGS_DOC.iter().any(|(k, _)| k == key),
                "{key} has a schema row but no settings_doc entry"
            );
        }
    }

    #[test]
    fn each_kind_matches_its_default() {
        for (key, default) in leaves() {
            let kind = kind_of(&key).unwrap();
            let ok = match kind {
                Bool => default.is_boolean(),
                Number => default.is_number(),
                OptionalNumber => default.is_null() || default.is_number(),
                Text | Font | TerminalTheme | UiTheme | Folder | Choice(_) => default.is_string(),
                Color => default
                    .as_str()
                    .is_some_and(|s| s.is_empty() || crate::ui_theme::is_gradient_stop(s)),
                TextList => default
                    .as_array()
                    .is_some_and(|a| a.iter().all(Value::is_string)),
                NumberList => default
                    .as_array()
                    .is_some_and(|a| a.iter().all(Value::is_number)),
                SearchEngines => default.is_array(),
            };
            assert!(ok, "{key} is {kind:?} but defaults to {default}");
        }
    }

    #[test]
    fn a_choice_offers_its_default() {
        for (key, default) in leaves() {
            if let Some(Choice(words)) = kind_of(&key) {
                let d = default.as_str().unwrap();
                assert!(
                    words.contains(&d),
                    "{key} defaults to {d:?}, not in {words:?}"
                );
            }
        }
    }

    /// Each word a choice offers is one the config reads as itself, so a
    /// renamed enum variant cannot leave the window offering a dead word.
    #[test]
    fn choices_round_trip_through_the_real_merge() {
        for (key, kind, _) in SETTINGS_SCHEMA {
            let Choice(words) = kind else { continue };
            for word in *words {
                let config = merge_config(&json!({ *key: word }));
                assert_eq!(
                    value_at(&config, key),
                    json!(word),
                    "{key}: {word:?} did not stick"
                );
            }
        }
    }

    #[test]
    fn every_section_has_keys_and_sections_come_in_menu_order() {
        for section in Section::ALL {
            assert!(
                keys_in(section).next().is_some(),
                "{} is empty",
                section.title()
            );
        }
        let mut sorted = Section::ALL;
        sorted.sort();
        assert_eq!(sorted, Section::ALL);
    }

    #[test]
    fn a_key_that_is_not_a_setting_has_no_kind() {
        assert_eq!(kind_of("terminal.fontSize"), Some(Number));
        assert_eq!(
            kind_of("ui.windowOpacity"),
            None,
            "a ui theme key, not a settings key"
        );
        assert_eq!(kind_of("nope"), None);
    }
}
