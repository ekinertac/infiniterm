//! `theme.json`: the user's chrome theme, parsed and validated (#425).
//!
//! Where it fits: `Model::apply_theme_file_text` runs `parse` on every change
//! of `~/.config/infiniterm/theme.json` and keeps the result in
//! `Model::chrome_theme`; the ui resolves the tokens against the live
//! `Chrome` colours when it draws the title bar and the status bar, so a
//! token such as "canvas" follows the canvas colour instead of freezing it.
//! Related: `config_files.rs` (the file and its watcher), `jsonc.rs`.
//!
//! Chrome only, never the terminal palette: those are the 522 iTerm2 schemes
//! and `theme` in settings.json. Keys are FLAT and dotted like settings.json
//! (`"titlebar.background": "canvas"`). An unknown key or a bad value rejects
//! the whole file and the theme in force stays, the rule settings.json
//! follows since #246.
use crate::jsonc::parse_jsonc;
use serde_json::Value;

/// A colour the file names: a literal, or a token the ui resolves each frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColorSpec {
    /// `#rrggbb` or `#rgb`, kept as written; the ui turns it into a colour.
    Hex(String),
    /// The canvas colour behind the cards: a flat bar that blends into it.
    Canvas,
    /// The card ground (the terminal theme's background).
    Card,
    /// What the bars wear without a theme file.
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

/// One bar's overrides; `None` means "as before the theme file existed".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BarTheme {
    pub background: Option<ColorSpec>,
    pub border: Option<BorderSpec>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChromeTheme {
    pub titlebar: BarTheme,
    pub statusbar: BarTheme,
}

/// Every key the file accepts. The `settings_doc` rule applies here too: a
/// key someone can write is a key in this list and in the docs.
pub const KEYS: [&str; 4] = [
    "titlebar.background",
    "titlebar.border",
    "statusbar.background",
    "statusbar.border",
];

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

/// Parses the file's text. An empty file is the empty theme; anything that
/// is not a JSON object, an unknown key or a bad value is an error.
pub fn parse(text: &str) -> Result<ChromeTheme, String> {
    if text.trim().is_empty() {
        return Ok(ChromeTheme::default());
    }
    let raw = parse_jsonc(text).map_err(|e| e.to_string())?;
    let obj = raw
        .as_object()
        .ok_or("theme.json must be a JSON object of \"key\": value")?;
    let mut theme = ChromeTheme::default();
    for (key, v) in obj {
        match key.as_str() {
            "titlebar.background" => theme.titlebar.background = Some(color(key, v)?),
            "titlebar.border" => theme.titlebar.border = Some(border(key, v)?),
            "statusbar.background" => theme.statusbar.background = Some(color(key, v)?),
            "statusbar.border" => theme.statusbar.border = Some(border(key, v)?),
            _ => return Err(format!("unknown key \"{key}\"")),
        }
    }
    Ok(theme)
}

/// What a new `theme.json` says: the file's job, then commented examples.
/// A test parses every example line, so they cannot rot.
pub const EMPTY_THEME: &str = concat!(
    "// Your chrome theme: the title bar and the status bar for now.\n",
    "// A colour is #rgb, #rrggbb, \"canvas\" (the canvas behind the cards),\n",
    "// \"card\" (the terminal theme's background), \"bar\" (the default) or\n",
    "// \"transparent\". A border is a colour or \"none\".\n",
    "//\n",
    "// Comments and trailing commas are allowed. A file with a mistake is not\n",
    "// applied and the theme you had stays.\n",
    "//\n",
    "// A flat look, both bars melting into the canvas. Delete the // to use it:\n",
    "//\n",
    "//   \"titlebar.background\": \"canvas\",\n",
    "//   \"titlebar.border\": \"none\",\n",
    "//   \"statusbar.background\": \"canvas\",\n",
    "//   \"statusbar.border\": \"none\",\n",
    "{\n}\n"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_or_object_is_the_empty_theme() {
        assert_eq!(parse("").unwrap(), ChromeTheme::default());
        assert_eq!(parse("  \n").unwrap(), ChromeTheme::default());
        assert_eq!(parse("{}").unwrap(), ChromeTheme::default());
    }

    #[test]
    fn the_flat_look_parses() {
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
    fn bad_values_name_the_key() {
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
    fn an_unknown_key_rejects_the_file() {
        let e = parse(r#"{"titlebar.colour": "canvas"}"#).unwrap_err();
        assert!(e.contains("titlebar.colour"), "{e}");
    }

    #[test]
    fn not_an_object_or_not_json_is_an_error() {
        assert!(parse("[1]").is_err());
        assert!(parse("\"canvas\"").is_err());
        assert!(parse("{").is_err());
    }

    #[test]
    fn every_listed_key_is_parsed() {
        for key in KEYS {
            let v = if key.ends_with("border") {
                "none"
            } else {
                "canvas"
            };
            assert!(parse(&format!(r#"{{"{key}": "{v}"}}"#)).is_ok(), "{key}");
        }
    }

    // Each commented example line is meant to be uncommented, so each must parse.
    #[test]
    fn the_new_file_examples_parse() {
        let body: String = EMPTY_THEME
            .lines()
            .filter_map(|l| l.strip_prefix("//   \""))
            .map(|l| format!("\"{l}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(body.lines().count(), 4);
        let t = parse(&format!("{{\n{body}\n}}")).unwrap();
        assert_eq!(t.statusbar.border, Some(BorderSpec::None));
        assert!(parse(EMPTY_THEME).unwrap() == ChromeTheme::default());
    }
}
