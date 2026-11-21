//! The settings window's rows and edits, as pure functions (#745 step 2).
//!
//! Where it fits: the window (`ui.settings`, a later step) draws one row
//! per `settings_schema` entry. This file decides what each row says and
//! what an edit writes, so the gpui view is wiring:
//!
//! - `rows`: every settings.json key with the value in force and whether
//!   settings.json sets it (the "set" marker and the reset button);
//! - `write_for`: what an edit writes. A value equal to the default removes
//!   the key instead of writing it, and reset removes it too (#745: "Setting
//!   a value back to its default removes the key rather than writing it");
//! - `apply`: the new settings.json text, patched through `jsonc` so comments
//!   and layout survive, and refused with the reason when `validate_settings`
//!   would refuse the file, so the file is never left in a state it rejects;
//! - `parse_field`: what a text field's text means for its kind;
//! - `matches`: the search field, across every section.
//!
//! What it is not: a second validator. `validate_settings` and
//! `merge_config` stay the judges; `parse_field` only turns typed text into
//! the JSON value the kind holds, and the value in force is what
//! `merge_config` made of the file (clamps included), so the window shows
//! what the app uses.
//!
//! Related: `settings_schema.rs` (kinds and sections), `settings_doc.rs`
//! (the text `matches` searches), `jsonc.rs` (`patch_json_text`,
//! `remove_key`), `model/settings_in.rs` (`validate_settings`).
//!
//! Constraints: pure, no gpui, no file access. The caller reads and writes
//! settings.json.

use crate::config::{flatten, merge_config, Config};
use crate::jsonc::{parse_jsonc, patch_json_text, remove_key};
use crate::settings_doc::SETTINGS_DOC;
use crate::settings_schema::{Kind, Section, SETTINGS_SCHEMA};
use serde_json::{Map, Value};

/// One row of the window.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub key: &'static str,
    pub kind: Kind,
    pub section: Section,
    /// The value the app uses: settings.json over the defaults, as
    /// `merge_config` read it.
    pub value: Value,
    /// settings.json names this key, so the row is marked and can be reset.
    pub set: bool,
}

/// What an edit does to settings.json.
#[derive(Clone, Debug, PartialEq)]
pub enum Write {
    Set(Value),
    Remove,
}

/// Every key of a config as a dotted name and its value, walked as the
/// defaults file is (`notify.ntfy.server` is one field named `ntfy.server`).
fn dotted(config: &Config) -> Map<String, Value> {
    fn walk(value: &Value, prefix: &str, out: &mut Map<String, Value>) {
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
            other => {
                out.insert(prefix.to_string(), other.clone());
            }
        }
    }
    let mut out = Map::new();
    walk(
        &serde_json::to_value(config).unwrap_or_default(),
        "",
        &mut out,
    );
    out
}

/// The keys a settings.json text sets, by dotted name. A text that does not
/// parse sets nothing: the app keeps the settings in force then (#246), and
/// so does the window.
fn set_keys(text: &str) -> Map<String, Value> {
    parse_jsonc(text)
        .ok()
        .and_then(|v| v.as_object().map(flatten))
        .unwrap_or_default()
}

/// The rows for a settings.json text, in schema order.
pub fn rows(settings_text: &str) -> Vec<Row> {
    let user = set_keys(settings_text);
    let in_force = dotted(&merge_config(&Value::Object(user.clone())));
    SETTINGS_SCHEMA
        .iter()
        .map(|&(key, kind, section)| Row {
            key,
            kind,
            section,
            value: in_force.get(key).cloned().unwrap_or(Value::Null),
            set: user.contains_key(key),
        })
        .collect()
}

/// The default of `key`, as the defaults file holds it.
pub fn default_of(key: &str) -> Option<Value> {
    dotted(&crate::config::default_config()).get(key).cloned()
}

/// Equal as settings: numbers by value, so `14` typed matches the default
/// `14.0` the config serialises.
fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// What setting `key` to `value` writes: the value, or the key's removal
/// when the value is the default.
pub fn write_for(key: &str, value: Value) -> Write {
    match default_of(key) {
        Some(d) if same(&d, &value) => Write::Remove,
        _ => Write::Set(value),
    }
}

/// The settings.json text after `write`, or why it was refused. `Ok(None)`
/// when the file would not change (the value is already there, or a reset
/// key was never set).
pub fn apply(settings_text: &str, key: &str, write: &Write) -> Result<Option<String>, String> {
    let next = match write {
        Write::Set(value) => patch_json_text(settings_text, key, value),
        Write::Remove => remove_key(settings_text, key),
    };
    let Some(next) = next else {
        return Ok(None);
    };
    let parsed = parse_jsonc(&next).map_err(|e| format!("settings.json would not parse: {e}"))?;
    crate::model::settings_in::validate_settings(&parsed)?;
    Ok(Some(next))
}

/// Past this a float no longer holds every whole number exactly (2^53 is
/// about 9e15), so a whole number this big or bigger stays a float.
const WHOLE_LIMIT: f64 = 1e15;

/// `#rgb`, `#rrggbb` or `#rrggbbaa`.
fn is_color(s: &str) -> bool {
    crate::ui_theme::is_gradient_stop(s)
}

/// The value a text field's `text` stands for, or why it does not. Only the
/// kinds edited as one line of text; the others have their own controls.
pub fn parse_field(kind: Kind, text: &str) -> Result<Value, String> {
    let t = text.trim();
    match kind {
        // A whole number is written whole (`16`, not `16.0`), as a person
        // would type it into the file.
        Kind::Number => t
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(|n| {
                if n.fract() == 0. && n.abs() < WHOLE_LIMIT {
                    Value::from(n as i64)
                } else {
                    Value::from(n)
                }
            })
            .ok_or_else(|| "a number".to_string()),
        Kind::OptionalNumber if t.is_empty() => Ok(Value::Null),
        Kind::OptionalNumber => parse_field(Kind::Number, t),
        Kind::Color if t.is_empty() || is_color(t) => Ok(Value::from(t)),
        Kind::Color => Err("a colour: #rgb, #rrggbb or #rrggbbaa".to_string()),
        // Text keeps its spaces: a shell command or a font list may need them.
        Kind::Text | Kind::Font | Kind::Folder | Kind::TerminalTheme | Kind::UiTheme => {
            Ok(Value::from(text))
        }
        Kind::Bool | Kind::Choice(_) | Kind::TextList | Kind::NumberList | Kind::SearchEngines => {
            Err("not edited as text".to_string())
        }
    }
}

/// How a value reads in a row and in a field opened on it: text as it is,
/// whole numbers without a decimal point (`14`, not `14.0`), `null` as
/// empty (an unset optional number), lists and objects as compact JSON.
pub fn display(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0. && f.abs() < WHOLE_LIMIT => format!("{}", f as i64),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

/// Words that keep their own spelling in a title: acronyms, and the ntfy
/// service's lower-case name.
const TITLE_WORDS: &[(&str, &str)] = &[
    ("ui", "UI"),
    ("lsp", "LSP"),
    ("fps", "FPS"),
    ("pwd", "PWD"),
    ("ntfy", "ntfy"),
    ("px", "px"),
];

/// A key as a title for people: the dotted parts and the camelCase words
/// split, each word capitalised (`terminal.fontFamily` reads "Terminal Font
/// Family"). The row shows the key itself beside it, so the title only has
/// to read well, not to be unique.
pub fn title(key: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    for part in key.split('.') {
        let mut word = String::new();
        for c in part.chars() {
            if c.is_uppercase() && !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            word.push(c);
        }
        if !word.is_empty() {
            words.push(word);
        }
    }
    words
        .iter()
        .map(|w| {
            let lower = w.to_lowercase();
            match TITLE_WORDS.iter().find(|(k, _)| *k == lower) {
                Some((_, spelled)) => spelled.to_string(),
                None => {
                    let mut c = lower.chars();
                    c.next()
                        .map(|f| f.to_uppercase().chain(c).collect())
                        .unwrap_or_default()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The first line of a key's doc text, the row's subtitle.
pub fn summary(key: &str) -> &'static str {
    SETTINGS_DOC
        .iter()
        .find(|(k, _)| *k == key)
        .and_then(|(_, lines)| lines.first().copied())
        .unwrap_or("")
}

/// True when `row` answers the search `query`: every word of it appears in
/// the key, the section's title or the key's doc text, ignoring case. An
/// empty query matches every row.
pub fn matches(row: &Row, query: &str) -> bool {
    let doc = SETTINGS_DOC
        .iter()
        .find(|(k, _)| *k == row.key)
        .map(|(_, lines)| lines.join(" "))
        .unwrap_or_default();
    let hay = format!("{} {} {}", row.key, row.section.title(), doc).to_lowercase();
    query
        .split_whitespace()
        .all(|w| hay.contains(&w.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row<'a>(rows: &'a [Row], key: &str) -> &'a Row {
        rows.iter().find(|r| r.key == key).unwrap()
    }

    #[test]
    fn an_empty_file_shows_every_default_unmarked() {
        let rows = rows("{}");
        assert_eq!(rows.len(), SETTINGS_SCHEMA.len());
        assert!(rows.iter().all(|r| !r.set));
        assert!(same(&row(&rows, "terminal.fontSize").value, &json!(14)));
        assert_eq!(row(&rows, "terminal.cursorStyle").value, json!("block"));
    }

    #[test]
    fn a_key_in_the_file_is_marked_with_its_value_flat_or_nested() {
        let rows = rows(
            r#"{
                // mine
                "terminal.fontSize": 16,
                "editor": { "explorer": { "git": true } },
            }"#,
        );
        let size = row(&rows, "terminal.fontSize");
        assert!(size.set);
        assert!(same(&size.value, &json!(16)));
        assert!(row(&rows, "editor.explorer.git").set);
        assert_eq!(row(&rows, "editor.explorer.git").value, json!(true));
        assert!(!row(&rows, "terminal.cursorBlink").set);
    }

    #[test]
    fn the_value_in_force_is_what_the_merge_made_of_it() {
        // ui.panStep is clamped to 20..1000; the window shows what the app uses.
        let rows = rows(r#"{ "ui.panStep": 5 }"#);
        let step = row(&rows, "ui.panStep");
        assert!(step.set);
        assert!(same(&step.value, &json!(crate::config::MIN_PAN_STEP)));
    }

    #[test]
    fn a_file_that_does_not_parse_shows_the_defaults() {
        let rows = rows("{ not json");
        assert!(rows.iter().all(|r| !r.set));
    }

    #[test]
    fn the_default_value_removes_the_key_and_any_other_value_sets_it() {
        assert_eq!(write_for("terminal.fontSize", json!(14)), Write::Remove);
        assert_eq!(write_for("terminal.fontSize", json!(14.0)), Write::Remove);
        assert_eq!(
            write_for("terminal.fontSize", json!(15)),
            Write::Set(json!(15))
        );
        assert_eq!(
            write_for("terminal.cursorBlink", json!(true)),
            Write::Remove
        );
        assert_eq!(write_for("editor.fontSize", Value::Null), Write::Remove);
        assert_eq!(
            write_for("editor.fontSize", json!(13)),
            Write::Set(json!(13))
        );
    }

    #[test]
    fn apply_patches_the_text_and_keeps_comments() {
        let text = "{\n  // my font\n  \"terminal.fontSize\": 16\n}\n";
        let next = apply(text, "terminal.fontSize", &Write::Set(json!(18)))
            .unwrap()
            .unwrap();
        assert!(next.contains("// my font"));
        assert!(next.contains("\"terminal.fontSize\": 18"));
        let added = apply(text, "terminal.cursorBlink", &Write::Set(json!(false)))
            .unwrap()
            .unwrap();
        assert!(added.contains("\"terminal.cursorBlink\": false"));
        assert!(added.contains("// my font"));
    }

    #[test]
    fn reset_removes_the_key_and_a_key_never_set_changes_nothing() {
        let text = "{\n  \"terminal.fontSize\": 16,\n  \"theme\": \"Dracula\"\n}\n";
        let next = apply(text, "terminal.fontSize", &Write::Remove)
            .unwrap()
            .unwrap();
        assert!(!next.contains("fontSize"));
        assert!(next.contains("Dracula"));
        assert_eq!(
            apply(text, "terminal.cursorBlink", &Write::Remove),
            Ok(None)
        );
    }

    #[test]
    fn writing_the_value_already_there_changes_nothing() {
        let text = "{ \"terminal.fontSize\": 16 }";
        assert_eq!(
            apply(text, "terminal.fontSize", &Write::Set(json!(16))),
            Ok(None)
        );
    }

    #[test]
    fn a_value_the_file_would_refuse_is_refused_with_the_reason() {
        let err = apply("{}", "terminal.fontSize", &Write::Set(json!("big"))).unwrap_err();
        assert!(err.contains("terminal.fontSize"), "{err}");
    }

    #[test]
    fn a_field_reads_numbers_colours_and_text_by_kind() {
        assert_eq!(
            parse_field(Kind::Number, " 15 "),
            Ok(json!(15)),
            "whole, as typed"
        );
        assert_eq!(parse_field(Kind::Number, "1.25"), Ok(json!(1.25)));
        assert!(parse_field(Kind::Number, "big").is_err());
        assert!(parse_field(Kind::Number, "inf").is_err());
        assert_eq!(parse_field(Kind::OptionalNumber, ""), Ok(Value::Null));
        assert_eq!(parse_field(Kind::OptionalNumber, "13"), Ok(json!(13)));
        assert_eq!(
            parse_field(Kind::Color, "#ff000080"),
            Ok(json!("#ff000080"))
        );
        assert_eq!(parse_field(Kind::Color, ""), Ok(json!("")));
        assert!(parse_field(Kind::Color, "red").is_err());
        assert_eq!(
            parse_field(Kind::Text, " /bin/zsh -l"),
            Ok(json!(" /bin/zsh -l"))
        );
        assert!(parse_field(Kind::Bool, "true").is_err());
    }

    #[test]
    fn a_key_reads_as_a_title() {
        assert_eq!(title("terminal.fontFamily"), "Terminal Font Family");
        assert_eq!(title("theme"), "Theme");
        assert_eq!(title("ui.hideRemoteBranches"), "UI Hide Remote Branches");
        assert_eq!(title("editor.lsp.enabled"), "Editor LSP Enabled");
        assert_eq!(title("notify.ntfy.server"), "Notify ntfy Server");
        assert_eq!(title("ui.minTextPx"), "UI Min Text px");
        assert_eq!(title("ui.statusBarPwd"), "UI Status Bar PWD");
        // Every key gets a title with no empty word in it.
        for (key, _, _) in SETTINGS_SCHEMA {
            let t = title(key);
            assert!(!t.is_empty() && !t.contains("  "), "{key}: {t:?}");
        }
    }

    #[test]
    fn values_read_plainly() {
        assert_eq!(display(&json!(14.0)), "14");
        assert_eq!(display(&json!(1.2)), "1.2");
        assert_eq!(display(&json!("block")), "block");
        assert_eq!(display(&Value::Null), "");
        assert_eq!(display(&json!(["A=1"])), "[\"A=1\"]");
        assert_eq!(
            summary("terminal.cursorBlink"),
            "Whether the cursor blinks."
        );
        assert_eq!(summary("nope"), "");
    }

    #[test]
    fn search_finds_by_key_section_and_doc_text_across_sections() {
        let rows = rows("{}");
        let hits = |q: &str| -> Vec<&str> {
            rows.iter()
                .filter(|r| matches(r, q))
                .map(|r| r.key)
                .collect()
        };
        assert_eq!(hits("").len(), rows.len());
        assert!(hits("fontsize").contains(&"terminal.fontSize"));
        assert!(hits("fontsize").contains(&"editor.fontSize"));
        assert!(hits("FONT editor").iter().all(|k| k.starts_with("editor.")));
        // From the doc line, not the key: "Whether the cursor blinks."
        assert!(hits("blinks").contains(&"terminal.cursorBlink"));
        assert!(hits("notifications").contains(&"notify.done"));
        assert!(hits("zzzz nothing").is_empty());
    }
}
