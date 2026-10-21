//! A ui theme: `ui-themes/<name>.json`, how infiniterm LOOKS (#511).
//!
//! Where it fits: `Model::apply_ui_theme_text` runs `parse` on the file that
//! `ui.theme` in settings.json names (and on every edit of it), keeps the
//! result in `Model::ui_theme`, and overlays its look keys on `config.ui`
//! (`Ui::take_look_from`), so every reader of `config.ui.cardRadius` and the
//! like is unchanged. The ui resolves the bar tokens against the live
//! `Chrome` colours each frame, so "canvas" follows the canvas colour.
//! Related: `config_files.rs` (the folder and its watcher), `look_migration.rs`
//! (moves the look keys out of an old settings.json), `settings_doc.rs`
//! (the reference text, from the docs the keys already had), `jsonc.rs`.
//!
//! One file per look, so a look is something you copy, switch and share.
//! Chrome only, never the terminal palette: that is `theme` in settings.json
//! and the 522 iTerm2 schemes. Keys are FLAT and dotted like settings.json.
//! The moved keys keep their `ui.` names (`ui.cardRadius`), so moving one is
//! moving a line and the docs they already had stay true. An unknown key or a
//! bad value rejects the whole file and the look in force stays, the rule
//! settings.json follows since #246.
use crate::jsonc::parse_jsonc;
use serde_json::{Map, Value};

/// The look every install has: the file `ui-themes/default.json` when it
/// exists, else the built-in values with nothing overridden.
pub const DEFAULT_NAME: &str = "default";

/// The folder inside the config folder.
pub const DIR_NAME: &str = "ui-themes";

/// A colour the file names: a literal, or a token the ui resolves each frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColorSpec {
    /// `#rrggbb` or `#rgb`, kept as written; the ui turns it into a colour.
    Hex(String),
    /// The canvas colour behind the cards: a flat bar that blends into it.
    Canvas,
    /// The card ground (the terminal theme's background).
    Card,
    /// What the bars wear without a theme.
    Bar,
    /// Nothing drawn: the window's own background shows through.
    Transparent,
}

/// A bar's edge: a colour, or none at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BorderSpec {
    None,
    Color(ColorSpec),
}

/// One bar's overrides; `None` means "as before ui themes existed".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BarTheme {
    pub background: Option<ColorSpec>,
    pub border: Option<BorderSpec>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UiTheme {
    /// The look keys the file set, as written (`"ui.cardRadius": 8`). Range
    /// clamping and defaults are `config::merge_config`'s, the same code
    /// settings.json used for these keys, so a value means what it always did.
    pub look: Map<String, Value>,
    pub titlebar: BarTheme,
    pub statusbar: BarTheme,
}

/// What kind of JSON value a look key holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Number,
    Bool,
    Text,
    TextOrList,
}

/// Every look key the file accepts, with its value kind. These are the keys
/// that left settings.json in #511. The `settings_doc` rule applies here too:
/// a key someone can write is a key in this list, in `Ui::take_look_from`
/// and in the docs; a test holds each of the three.
pub const LOOK_KEYS: [(&str, Kind); 17] = [
    ("ui.windowOpacity", Kind::Number),
    ("ui.windowBlur", Kind::Bool),
    ("ui.windowColor", Kind::Text),
    ("ui.backgroundImage", Kind::TextOrList),
    ("ui.backgroundImageInterval", Kind::Number),
    ("ui.backgroundImageFade", Kind::Number),
    ("ui.backgroundImageFit", Kind::Text),
    ("ui.cardRadius", Kind::Number),
    ("ui.cardOpacity", Kind::Number),
    ("ui.showGrid", Kind::Bool),
    ("ui.inactiveDim", Kind::Number),
    ("ui.unfocusedDim", Kind::Number),
    ("ui.fontFamily", Kind::Text),
    ("ui.fontSize", Kind::Number),
    ("ui.fontWeight", Kind::Text),
    ("ui.fontWeightBold", Kind::Text),
    ("ui.hideLabelWhenFitted", Kind::Bool),
];

/// The four bar keys (#425), with their docs. They exist only in a ui theme.
pub const BAR_KEYS: [(&str, &[&str], &str); 4] = [
    (
        "titlebar.background",
        &[
            "The title bar's ground: #rgb, #rrggbb, \"canvas\" (the canvas behind the cards),",
            "\"card\" (the terminal theme's background), \"bar\" (the default) or \"transparent\".",
        ],
        "\"canvas\"",
    ),
    (
        "titlebar.border",
        &["The title bar's edge: a colour as above, or \"none\"."],
        "\"none\"",
    ),
    (
        "statusbar.background",
        &["The status bar's ground. Same values as titlebar.background."],
        "\"canvas\"",
    ),
    (
        "statusbar.border",
        &["The status bar's edge: a colour, or \"none\"."],
        "\"none\"",
    ),
];

/// Whether a dotted key belongs to a ui theme and not to settings.json.
pub fn is_look_key(key: &str) -> bool {
    LOOK_KEYS.iter().any(|(k, _)| *k == key)
}

/// A settings value with the look keys taken out, and their dotted names.
/// Both spellings go: `"ui.cardRadius": 8` and `"ui": {"cardRadius": 8}`.
/// settings.json no longer accepts these keys, but an old file that still
/// holds them (a host older than this build, sent to a viewer; a key typed
/// by hand) must not fail validation, so they are dropped, not refused.
pub fn strip_look(raw: &Value) -> (Value, Vec<String>) {
    let Some(map) = raw.as_object() else {
        return (raw.clone(), vec![]);
    };
    let mut out = Map::new();
    let mut dropped = vec![];
    for (k, v) in map {
        if is_look_key(k) {
            dropped.push(k.clone());
        } else if k == "ui" {
            match v.as_object() {
                Some(inner) => {
                    let mut kept = Map::new();
                    for (ik, iv) in inner {
                        let full = format!("ui.{ik}");
                        if is_look_key(&full) {
                            dropped.push(full);
                        } else {
                            kept.insert(ik.clone(), iv.clone());
                        }
                    }
                    out.insert(k.clone(), Value::Object(kept));
                }
                None => {
                    out.insert(k.clone(), v.clone());
                }
            }
        } else {
            out.insert(k.clone(), v.clone());
        }
    }
    (Value::Object(out), dropped)
}

/// Whether `name` can be a ui theme's file name: one plain path component, so
/// a `ui.theme` of `../../x` cannot reach outside the folder.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
        && name.trim() == name
}

/// The picker's rows from the file stems in the folder: `default` always
/// (it works with no file), first; the rest A to Z without regard to case.
pub fn names_from_stems(stems: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut rest: Vec<String> = stems
        .into_iter()
        .filter(|s| s != DEFAULT_NAME && valid_name(s))
        .collect();
    rest.sort_by_key(|s| s.to_lowercase());
    rest.dedup();
    std::iter::once(DEFAULT_NAME.to_string())
        .chain(rest)
        .collect()
}

fn is_hex(s: &str) -> bool {
    let Some(h) = s.strip_prefix('#') else {
        return false;
    };
    matches!(h.len(), 3 | 6) && h.chars().all(|c| c.is_ascii_hexdigit())
}

fn color(key: &str, v: &Value) -> Result<ColorSpec, String> {
    let s = v
        .as_str()
        .ok_or_else(|| format!("\"{key}\" must be a string"))?;
    match s {
        "canvas" => Ok(ColorSpec::Canvas),
        "card" => Ok(ColorSpec::Card),
        "bar" => Ok(ColorSpec::Bar),
        "transparent" => Ok(ColorSpec::Transparent),
        _ if is_hex(s) => Ok(ColorSpec::Hex(s.to_string())),
        _ => Err(format!(
            "\"{key}\": \"{s}\" is not #rgb, #rrggbb, \"canvas\", \"card\", \"bar\" or \"transparent\""
        )),
    }
}

fn border(key: &str, v: &Value) -> Result<BorderSpec, String> {
    if v.as_str() == Some("none") {
        return Ok(BorderSpec::None);
    }
    color(key, v)
        .map(BorderSpec::Color)
        .map_err(|e| format!("{e} (or \"none\")"))
}

fn check_kind(key: &str, kind: Kind, v: &Value) -> Result<(), String> {
    let (ok, want) = match kind {
        Kind::Number => (v.is_number(), "a number"),
        Kind::Bool => (v.is_boolean(), "true or false"),
        Kind::Text => (v.is_string(), "text"),
        Kind::TextOrList => (
            v.is_string() || v.as_array().is_some_and(|a| a.iter().all(Value::is_string)),
            "text or a list of text",
        ),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("\"{key}\" must be {want}"))
    }
}

/// Parses a ui theme's text. An empty file is the empty theme (the built-in
/// look); anything that is not a JSON object, an unknown key or a bad value
/// is an error.
pub fn parse(text: &str) -> Result<UiTheme, String> {
    if text.trim().is_empty() {
        return Ok(UiTheme::default());
    }
    let raw = parse_jsonc(text).map_err(|e| e.to_string())?;
    let obj = raw
        .as_object()
        .ok_or("a ui theme must be a JSON object of \"key\": value")?;
    let mut theme = UiTheme::default();
    for (key, v) in obj {
        match key.as_str() {
            "titlebar.background" => theme.titlebar.background = Some(color(key, v)?),
            "titlebar.border" => theme.titlebar.border = Some(border(key, v)?),
            "statusbar.background" => theme.statusbar.background = Some(color(key, v)?),
            "statusbar.border" => theme.statusbar.border = Some(border(key, v)?),
            _ => match LOOK_KEYS.iter().find(|(k, _)| *k == key) {
                Some((_, kind)) => {
                    check_kind(key, *kind, v)?;
                    theme.look.insert(key.clone(), v.clone());
                }
                None => return Err(format!("unknown key \"{key}\"")),
            },
        }
    }
    Ok(theme)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{default_config, merge_config};
    use serde_json::json;

    #[test]
    fn empty_text_is_the_built_in_look() {
        assert_eq!(parse("").unwrap(), UiTheme::default());
        assert_eq!(parse("  \n").unwrap(), UiTheme::default());
        assert_eq!(parse("// only a note\n{}").unwrap(), UiTheme::default());
    }

    #[test]
    fn parses_the_bars_flat_with_comments_and_trailing_commas() {
        let t = parse(
            r##"{
              // flat
              "titlebar.background": "canvas",
              "titlebar.border": "none",
              "statusbar.background": "#101216",
              "statusbar.border": "card",
            }"##,
        )
        .unwrap();
        assert_eq!(t.titlebar.background, Some(ColorSpec::Canvas));
        assert_eq!(t.titlebar.border, Some(BorderSpec::None));
        assert_eq!(
            t.statusbar.background,
            Some(ColorSpec::Hex("#101216".into()))
        );
        assert_eq!(t.statusbar.border, Some(BorderSpec::Color(ColorSpec::Card)));
        assert!(t.look.is_empty());
    }

    #[test]
    fn short_hex_and_every_token_are_accepted() {
        for v in ["#abc", "#A1B2C3", "canvas", "card", "bar", "transparent"] {
            assert!(
                parse(&format!(r#"{{"titlebar.background": "{v}"}}"#)).is_ok(),
                "{v}"
            );
        }
    }

    #[test]
    fn a_background_cannot_be_none() {
        // "none" is a border word; a background that vanishes is "transparent".
        assert!(parse(r#"{"titlebar.background": "none"}"#).is_err());
    }

    #[test]
    fn bad_bar_values_name_the_key() {
        for bad in [
            r##"{"titlebar.background": "#12"}"##,
            r##"{"titlebar.background": "#12345g"}"##,
            r#"{"titlebar.background": 5}"#,
            r#"{"statusbar.border": true}"#,
            r#"{"statusbar.border": "thick"}"#,
        ] {
            let e = parse(bad).unwrap_err();
            assert!(e.contains("titlebar") || e.contains("statusbar"), "{e}");
        }
    }

    #[test]
    fn look_keys_keep_their_values_as_written() {
        let t = parse(
            r#"{
              "ui.cardRadius": 12,
              "ui.showGrid": false,
              "ui.windowColor": "teal",
              "ui.backgroundImage": ["dusk", "~/a.png"],
              "ui.hideLabelWhenFitted": true,
            }"#,
        )
        .unwrap();
        assert_eq!(t.look["ui.cardRadius"], json!(12));
        assert_eq!(t.look["ui.showGrid"], json!(false));
        assert_eq!(t.look["ui.backgroundImage"], json!(["dusk", "~/a.png"]));
        assert_eq!(t.look.len(), 5);
    }

    #[test]
    fn a_look_value_of_the_wrong_kind_rejects_the_file_naming_the_key() {
        for (bad, key) in [
            (r#"{"ui.cardRadius": "big"}"#, "ui.cardRadius"),
            (r#"{"ui.showGrid": "no"}"#, "ui.showGrid"),
            (r#"{"ui.windowColor": 5}"#, "ui.windowColor"),
            (r#"{"ui.backgroundImage": [1]}"#, "ui.backgroundImage"),
            (r#"{"ui.backgroundImage": 3}"#, "ui.backgroundImage"),
        ] {
            let e = parse(bad).unwrap_err();
            assert!(e.contains(key), "{e}");
        }
    }

    // A behaviour key is not a look key: it stays in settings.json.
    #[test]
    fn a_behaviour_key_or_a_typo_rejects_the_file() {
        for key in ["ui.panStep", "ui.animations", "titlebar.colour", "theme"] {
            let e = parse(&format!(r#"{{"{key}": 1}}"#)).unwrap_err();
            assert!(e.contains(key), "{e}");
        }
    }

    #[test]
    fn not_an_object_or_not_json_is_an_error() {
        assert!(parse("[1]").is_err());
        assert!(parse("\"canvas\"").is_err());
        assert!(parse("{").is_err());
    }

    // The three lists that name a look key must agree.
    #[test]
    fn every_look_key_is_a_ui_field_and_is_documented() {
        let ui = serde_json::to_value(default_config().ui).unwrap();
        for (key, _) in LOOK_KEYS {
            let short = key.strip_prefix("ui.").unwrap();
            assert!(ui.get(short).is_some(), "{key} is not a Ui field");
            assert!(
                !crate::settings_doc::doc(key).is_empty(),
                "{key} has no doc"
            );
        }
    }

    #[test]
    fn every_look_key_is_copied_by_take_look_from() {
        for (key, kind) in LOOK_KEYS {
            let sample = match (key, kind) {
                ("ui.backgroundImageFit", _) => json!("contain"),
                ("ui.showGrid", _) => json!(false),
                (_, Kind::Bool) => json!(true),
                (_, Kind::Number) => json!(0.5),
                (_, Kind::Text) => json!("x"),
                (_, Kind::TextOrList) => json!(["dusk"]),
            };
            let look = merge_config(&json!({ key: sample })).ui;
            let mut ui = default_config().ui;
            ui.take_look_from(&look);
            assert_ne!(ui, default_config().ui, "{key} is not copied");
        }
    }

    #[test]
    fn take_look_from_leaves_the_behaviour_alone() {
        let look = merge_config(&json!({"ui.panStep": 99, "ui.cardRadius": 9})).ui;
        let mut ui = default_config().ui;
        ui.take_look_from(&look);
        assert_eq!(ui.card_radius, 9.);
        assert_eq!(ui.pan_step, default_config().ui.pan_step);
    }

    #[test]
    fn strip_look_drops_both_spellings_and_keeps_the_rest() {
        let raw = json!({
            "terminal.fontSize": 16,
            "ui.cardRadius": 8,
            "ui.panStep": 100,
            "ui": {"showGrid": false, "animations": false, "windowOpacity": 0.9},
        });
        let (kept, mut dropped) = strip_look(&raw);
        dropped.sort();
        assert_eq!(
            dropped,
            ["ui.cardRadius", "ui.showGrid", "ui.windowOpacity"]
        );
        assert_eq!(
            kept,
            json!({
                "terminal.fontSize": 16,
                "ui.panStep": 100,
                "ui": {"animations": false},
            })
        );
        assert_eq!(strip_look(&json!([1])), (json!([1]), vec![]));
    }

    #[test]
    fn names_must_be_one_plain_path_component() {
        assert!(valid_name("default"));
        assert!(valid_name("Glass 2"));
        for bad in ["", "..", ".hidden", "a/b", "../x", "a\\b", " pad"] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_picker_lists_default_first_then_a_to_z() {
        let names = names_from_stems(
            ["zen", "Glass", "default", "aurora", "glass", ".x", "a/b"]
                .iter()
                .map(|s| s.to_string()),
        );
        assert_eq!(names, ["default", "aurora", "Glass", "glass", "zen"]);
        assert_eq!(names_from_stems(vec![]), ["default"]);
    }
}
