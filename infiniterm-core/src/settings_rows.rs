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
use crate::settings_schema::{Kind, Section, ThemeGroup, SETTINGS_SCHEMA, THEME_SCHEMA};
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
    /// What settings.json itself holds for the key, when it names it. A
    /// list row edits this rather than `value`: the merge drops a search
    /// engine that is not complete yet, and the person is still typing it.
    pub raw: Option<Value>,
    /// A ui theme key's group on the UI Theme page; `None` for a
    /// settings.json key. A theme row's `section` means nothing.
    pub theme: Option<ThemeGroup>,
}

impl Row {
    /// The list a list row edits: the file's own when it has one, else the
    /// value in force.
    pub fn list(&self) -> Vec<Value> {
        match self.raw.as_ref().unwrap_or(&self.value) {
            Value::Array(items) => items.clone(),
            // `ui.backgroundImage` may be one picture rather than a list.
            Value::String(one) if !one.is_empty() => vec![Value::from(one.clone())],
            _ => vec![],
        }
    }
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
            raw: user.get(key).cloned(),
            theme: None,
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
        // A colour or one of the theme's words; `ui_theme::parse` judges it
        // when the theme is written, with its own message.
        Kind::ColorWord => Ok(Value::from(t)),
        Kind::Bool
        | Kind::Choice(_)
        | Kind::TextList
        | Kind::NumberList
        | Kind::SearchEngines
        | Kind::ColorPair
        | Kind::PathList
        | Kind::IconMap
        | Kind::NameList(_) => Err("not edited as text".to_string()),
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

/// The first sentence of a key's doc text, the row's subtitle. The doc is
/// stored as lines wrapped for the defaults file, so the first LINE stops
/// mid-sentence ("by file name without the extension, from"); the lines
/// are joined and cut after the first full stop that ends a sentence.
pub fn summary(key: &str) -> String {
    let text = doc_text(key);
    match text.find(". ") {
        Some(end) => text[..=end].to_string(),
        None => text,
    }
}

/// `list` with item `i` replaced, or `item` added at the end when `i` is
/// past the last (the row's "Add" opens a field one past the end).
pub fn list_set(list: &[Value], i: usize, item: Value) -> Value {
    let mut out = list.to_vec();
    match out.get_mut(i) {
        Some(slot) => *slot = item,
        None => out.push(item),
    }
    Value::Array(out)
}

/// `list` without item `i`.
pub fn list_remove(list: &[Value], i: usize) -> Value {
    Value::Array(
        list.iter()
            .enumerate()
            .filter(|(n, _)| *n != i)
            .map(|(_, v)| v.clone())
            .collect(),
    )
}

/// The three parts of a `browser.engines` entry, in the order the row
/// shows them.
pub const ENGINE_FIELDS: [&str; 3] = ["keyword", "name", "searchUrl"];

/// Engine `i` with one of its parts set, the others kept (a new engine
/// starts with all three empty).
pub fn engine_set(list: &[Value], i: usize, part: &str, text: &str) -> Value {
    let mut engine = list
        .get(i)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(|| {
            ENGINE_FIELDS
                .iter()
                .map(|f| (f.to_string(), Value::from("")))
                .collect()
        });
    engine.insert(part.to_string(), Value::from(text));
    list_set(list, i, Value::Object(engine))
}

/// Why an engine is not used yet, as `merge_config` decides it: a part is
/// missing, or the search address has no `%s` for the words.
pub fn engine_problem(engine: &Value) -> Option<&'static str> {
    let text = |f: &str| engine.get(f).and_then(Value::as_str).unwrap_or("");
    if text("keyword").is_empty() || text("name").is_empty() {
        Some("not used yet: it needs a keyword and a name")
    } else if !text("searchUrl").contains("%s") {
        Some("not used yet: the search address needs %s where the words go")
    } else {
        None
    }
}

/// The items of a picker that answer what was typed, best first; all of
/// them in their own order when nothing was typed.
pub fn pick_matches<'a>(items: &'a [String], query: &str) -> Vec<&'a str> {
    let q = query.trim();
    if q.is_empty() {
        return items.iter().map(String::as_str).collect();
    }
    let mut scored: Vec<(f64, &str)> = items
        .iter()
        .filter_map(|i| crate::fuzzy::fuzzy_match(q, i).map(|m| (m.score, i.as_str())))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().map(|(_, i)| i).collect()
}

/// A key's doc text, lines joined: `SETTINGS_DOC` holds the settings keys
/// and the look keys, `ui_theme::BAR_KEYS` the other theme keys.
fn doc_text(key: &str) -> String {
    SETTINGS_DOC
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, lines)| lines.join(" "))
        .or_else(|| {
            crate::ui_theme::BAR_KEYS
                .iter()
                .find(|(k, _, _)| *k == key)
                .map(|(_, lines, _)| lines.join(" "))
        })
        .unwrap_or_default()
}

/// The theme a fresh file starts from when the theme's file is empty or
/// missing: an empty object, which `patch_json_text` can add to.
const THEME_EMPTY: &str = "{\n}\n";

/// The built-in value of a look key (the defaults' `ui` section); `None`
/// for a bar key, whose built-in look has no value to show.
pub fn theme_builtin(key: &str) -> Option<Value> {
    let short = key.strip_prefix("ui.")?;
    let ui = serde_json::to_value(crate::config::default_config().ui).ok()?;
    ui.get(short).cloned()
}

/// The UI Theme page's rows for a theme file's text: the file's value, else
/// the built-in one (`Null` for a bar key the file does not set).
pub fn theme_rows(theme_text: &str) -> Vec<Row> {
    let file = parse_jsonc(theme_text)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    THEME_SCHEMA
        .iter()
        .map(|&(key, kind, group)| {
            let raw = file.get(key).cloned();
            Row {
                key,
                kind,
                section: Section::General,
                value: raw
                    .clone()
                    .or_else(|| theme_builtin(key))
                    .unwrap_or(Value::Null),
                set: raw.is_some(),
                raw,
                theme: Some(group),
            }
        })
        .collect()
}

/// `write_for` for a theme key: a look key's built-in value removes it.
pub fn theme_write_for(key: &str, value: Value) -> Write {
    match theme_builtin(key) {
        Some(d) if same(&d, &value) => Write::Remove,
        _ => Write::Set(value),
    }
}

/// `apply` for a ui theme file: patched the same way, and refused with
/// `ui_theme::parse`'s reason when the theme would not load.
pub fn theme_apply(theme_text: &str, key: &str, write: &Write) -> Result<Option<String>, String> {
    let base = if theme_text.trim().is_empty() {
        THEME_EMPTY
    } else {
        theme_text
    };
    let next = match write {
        Write::Set(value) => patch_json_text(base, key, value),
        Write::Remove => remove_key(base, key),
    };
    let Some(next) = next else {
        return Ok(None);
    };
    crate::ui_theme::parse(&next)?;
    Ok(Some(next))
}

/// `tab.iconMap`'s entries in the file's order, as program and icon.
pub fn map_entries(map: &Value) -> Vec<(String, String)> {
    map.as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// The map with entry `i` set from `program=icon` text (added at the end
/// when `i` is past the last), or why the text is not an entry. One cell
/// for both halves, because an entry with an empty icon is refused.
pub fn map_set(map: &Value, i: usize, text: &str) -> Result<Value, String> {
    let (program, icon) = text
        .split_once('=')
        .map(|(p, v)| (p.trim(), v.trim()))
        .filter(|(p, v)| !p.is_empty() && !v.is_empty())
        .ok_or("program=icon, for example claude=\u{f544}")?;
    let mut entries = map_entries(map);
    match entries.get_mut(i) {
        Some(slot) => *slot = (program.to_string(), icon.to_string()),
        None => entries.push((program.to_string(), icon.to_string())),
    }
    Ok(Value::Object(
        entries
            .into_iter()
            .map(|(k, v)| (k, Value::from(v)))
            .collect(),
    ))
}

/// The map without entry `i`.
pub fn map_remove(map: &Value, i: usize) -> Value {
    Value::Object(
        map_entries(map)
            .into_iter()
            .enumerate()
            .filter(|(n, _)| *n != i)
            .map(|(_, (k, v))| (k, Value::from(v)))
            .collect(),
    )
}

/// A name list with `name` taken out if it is in, or added at the end.
pub fn name_toggle(list: &[Value], name: &str) -> Value {
    if list.iter().any(|v| v.as_str() == Some(name)) {
        Value::Array(
            list.iter()
                .filter(|v| v.as_str() != Some(name))
                .cloned()
                .collect(),
        )
    } else {
        let mut out = list.to_vec();
        out.push(Value::from(name));
        Value::Array(out)
    }
}

/// A name for a copy of the built-in look that no theme has yet:
/// `my-theme`, then `my-theme-2`, `my-theme-3` and so on.
pub fn copy_name(existing: &[String]) -> String {
    const BASE: &str = "my-theme";
    if !existing.iter().any(|e| e == BASE) {
        return BASE.to_string();
    }
    (2..)
        .map(|n| format!("{BASE}-{n}"))
        .find(|name| !existing.contains(name))
        .expect("an unused name")
}

/// True when `row` answers the search `query`: every word of it appears in
/// the key, the section's title or the key's doc text, ignoring case. An
/// empty query matches every row.
pub fn matches(row: &Row, query: &str) -> bool {
    let place = match row.theme {
        Some(group) => format!("UI Theme {}", group.title()),
        None => row.section.title().to_string(),
    };
    let hay = format!("{} {} {}", row.key, place, doc_text(row.key)).to_lowercase();
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
    fn a_list_row_edits_what_the_file_holds() {
        // Incomplete: the merge drops it, the row keeps it.
        let rows = rows(r#"{ "browser.engines": [{ "keyword": "gh" }] }"#);
        let engines = row(&rows, "browser.engines");
        assert_eq!(engines.value, json!([]));
        assert_eq!(engines.list(), vec![json!({ "keyword": "gh" })]);
        let rows = super::rows("{}");
        assert!(row(&rows, "terminal.env").list().is_empty());
    }

    #[test]
    fn list_items_are_set_added_and_removed() {
        let list = vec![json!("A=1"), json!("B=2")];
        assert_eq!(list_set(&list, 1, json!("B=3")), json!(["A=1", "B=3"]));
        assert_eq!(
            list_set(&list, 2, json!("C=1")),
            json!(["A=1", "B=2", "C=1"])
        );
        assert_eq!(list_remove(&list, 0), json!(["B=2"]));
        assert_eq!(list_remove(&list, 9), json!(["A=1", "B=2"]));
    }

    #[test]
    fn an_engine_is_built_one_part_at_a_time_and_says_what_it_lacks() {
        let new = engine_set(&[], 0, "keyword", "gh");
        assert_eq!(
            new,
            json!([{ "keyword": "gh", "name": "", "searchUrl": "" }])
        );
        let list = new.as_array().unwrap().clone();
        assert!(engine_problem(&list[0]).unwrap().contains("name"));
        let named = engine_set(&list, 0, "name", "GitHub");
        let url = engine_set(named.as_array().unwrap(), 0, "searchUrl", "https://x/?q=");
        assert!(engine_problem(&url[0]).unwrap().contains("%s"));
        let ok = engine_set(url.as_array().unwrap(), 0, "searchUrl", "https://x/?q=%s");
        assert_eq!(engine_problem(&ok[0]), None);
        // The merge agrees: a complete engine is in force.
        let config = merge_config(&json!({ "browser.engines": ok }));
        assert_eq!(config.browser.engines.len(), 1);
    }

    #[test]
    fn a_picker_lists_everything_until_something_is_typed() {
        let items: Vec<String> = ["Dracula", "Solarized Dark", "Violite"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            pick_matches(&items, ""),
            vec!["Dracula", "Solarized Dark", "Violite"]
        );
        assert_eq!(pick_matches(&items, "sol")[0], "Solarized Dark");
        assert!(pick_matches(&items, "zzz").is_empty());
    }

    #[test]
    fn theme_rows_show_the_file_else_the_builtin_look() {
        let rows = theme_rows("{ // mine\n \"ui.windowOpacity\": 0.8, \"tab.radius\": 6 }");
        assert_eq!(rows.len(), crate::settings_schema::THEME_SCHEMA.len());
        let opacity = row(&rows, "ui.windowOpacity");
        assert!(opacity.set && same(&opacity.value, &json!(0.8)));
        assert_eq!(opacity.theme, Some(ThemeGroup::Window));
        let blur = row(&rows, "ui.windowBlur");
        assert!(!blur.set);
        assert_eq!(Some(blur.value.clone()), theme_builtin("ui.windowBlur"));
        // A bar key the file does not set has no value to show.
        assert_eq!(row(&rows, "titlebar.border").value, Value::Null);
        assert!(row(&rows, "tab.radius").set);
        // An empty or missing file is the built-in look.
        assert!(theme_rows("").iter().all(|r| !r.set));
    }

    #[test]
    fn a_theme_edit_is_checked_by_the_theme_parser_and_keeps_comments() {
        let text = "{\n  // my look\n  \"ui.cardRadius\": 8\n}\n";
        let next = theme_apply(text, "titlebar.background", &Write::Set(json!("canvas")))
            .unwrap()
            .unwrap();
        assert!(
            next.contains("// my look") && next.contains("\"titlebar.background\": \"canvas\"")
        );
        let err = theme_apply(text, "titlebar.background", &Write::Set(json!("blue"))).unwrap_err();
        assert!(err.contains("titlebar.background"), "{err}");
        let gone = theme_apply(text, "ui.cardRadius", &Write::Remove)
            .unwrap()
            .unwrap();
        assert!(!gone.contains("cardRadius"));
        // An empty file gets an object to add to.
        let fresh = theme_apply("", "tab.radius", &Write::Set(json!(4)))
            .unwrap()
            .unwrap();
        assert!(crate::ui_theme::parse(&fresh).is_ok());
        assert!(fresh.contains("\"tab.radius\": 4"));
    }

    #[test]
    fn a_look_key_back_at_its_builtin_value_is_removed() {
        let builtin = theme_builtin("ui.windowOpacity").unwrap();
        assert_eq!(theme_write_for("ui.windowOpacity", builtin), Write::Remove);
        assert_eq!(
            theme_write_for("ui.windowOpacity", json!(0.5)),
            Write::Set(json!(0.5))
        );
        // A bar key has no built-in value: what is set is written.
        assert_eq!(
            theme_write_for("tab.radius", json!(0)),
            Write::Set(json!(0))
        );
    }

    #[test]
    fn icon_map_entries_are_one_cell_each() {
        let map = json!({ "claude": "a" });
        assert_eq!(map_entries(&map), vec![("claude".into(), "a".into())]);
        assert_eq!(
            map_set(&map, 1, "vim = b").unwrap(),
            json!({ "claude": "a", "vim": "b" })
        );
        assert_eq!(
            map_set(&map, 0, "codex=c").unwrap(),
            json!({ "codex": "c" })
        );
        assert!(
            map_set(&map, 1, "vim=").is_err(),
            "an empty icon is refused"
        );
        assert!(map_set(&map, 1, "nothing").is_err());
        assert_eq!(map_remove(&map, 0), json!({}));
        // What it builds, the theme takes.
        let built = map_set(&json!({}), 0, "claude=x").unwrap();
        assert!(crate::ui_theme::parse(&json!({ "tab.iconMap": built }).to_string()).is_ok());
    }

    #[test]
    fn a_name_is_toggled_in_and_out_of_its_list() {
        let list = vec![json!("close")];
        assert_eq!(name_toggle(&list, "maximise"), json!(["close", "maximise"]));
        assert_eq!(name_toggle(&list, "close"), json!([]));
    }

    #[test]
    fn a_copy_gets_a_name_no_theme_has() {
        assert_eq!(copy_name(&["default".into()]), "my-theme");
        assert_eq!(
            copy_name(&["my-theme".into(), "my-theme-2".into()]),
            "my-theme-3"
        );
        assert!(crate::ui_theme::valid_name(&copy_name(&[])));
    }

    #[test]
    fn theme_keys_are_searched_and_summarised_from_their_own_docs() {
        let rows = theme_rows("");
        let radius = row(&rows, "tab.radius");
        assert!(matches(radius, "ui theme workspace"));
        assert!(!summary("titlebar.border").is_empty());
        assert!(!summary("ui.windowOpacity").is_empty());
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
        // A sentence that runs over two stored lines comes out whole.
        let theme = summary("theme");
        assert!(theme.starts_with("Terminal colour scheme"), "{theme}");
        assert!(theme.ends_with('.'), "{theme}");
        assert!(!theme.contains("Five"), "only the first sentence: {theme}");
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
