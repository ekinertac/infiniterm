//! The one-time move of the look keys out of settings.json (#511).
//!
//! Where it fits: `config_files::migrate_look_keys` reads settings.json and
//! `ui-themes/default.json`, calls `migrate`, and writes both back before the
//! settings are applied. Without it every current user's settings.json, which
//! holds `ui.windowOpacity` or `ui.fontSize` or the like, would lose its look
//! the day settings.json stopped reading those keys.
//! Related: `ui_theme.rs` (the key list and the rules), `jsonc.rs`
//! (`patch_json_text` writes into the theme, `remove_key` cuts from settings).
//!
//! Text is patched, never reserialised: the comments in both files are the
//! user's. There is no marker file: once settings.json holds no look key,
//! `migrate` finds nothing and returns `None`, so it is safe to run at every
//! launch, and it also catches a key someone types by hand later.
use crate::config::flatten;
use crate::jsonc::{parse_jsonc, patch_json_text, remove_key, strip_comments};
use crate::ui_theme::{DIR_NAME, LOOK_KEYS};

#[derive(Debug, PartialEq, Eq)]
pub struct Moved {
    /// settings.json without the look keys, with a note where they went.
    pub settings: String,
    /// The theme file with them added.
    pub theme: String,
    /// The dotted names that moved, in the order of `LOOK_KEYS`.
    pub keys: Vec<String>,
}

/// What moving the look keys out of `settings` does, or `None` when there is
/// nothing to move (or nothing safe to do: a settings file or a theme file
/// that does not parse is left alone; its owner has a notice already).
///
/// `theme` is the current text of `ui-themes/<theme_name>.json`, or `None`
/// when the file does not exist, in which case `seed` is the text it starts
/// from. A key the theme already sets is not overwritten: the theme is the
/// newer word. It is still taken out of settings.json, which no longer reads it.
pub fn migrate(settings: &str, theme: Option<&str>, seed: &str, theme_name: &str) -> Option<Moved> {
    let raw = parse_jsonc(settings).ok()?;
    let flat = flatten(raw.as_object()?);
    let present: Vec<(&str, &serde_json::Value)> = LOOK_KEYS
        .iter()
        .filter_map(|(k, _)| flat.get(*k).map(|v| (*k, v)))
        .collect();
    if present.is_empty() {
        return None;
    }
    let mut theme_text = theme.unwrap_or(seed).to_string();
    let theme_flat = flatten(parse_jsonc(&theme_text).ok()?.as_object()?);
    for (key, value) in &present {
        if !theme_flat.contains_key(*key) {
            if let Some(patched) = patch_json_text(&theme_text, key, value) {
                theme_text = patched;
            }
        }
    }
    let mut settings_text = settings.to_string();
    // Both spellings may be there (flat and nested); `flatten` read the flat one.
    for (key, _) in &present {
        while let Some(cut) = remove_key(&settings_text, key) {
            settings_text = cut;
        }
    }
    let keys: Vec<String> = present.iter().map(|(k, _)| k.to_string()).collect();
    let mut note = format!(
        "\n  // Moved to {DIR_NAME}/{theme_name}.json, where the look lives now: {}.",
        keys.join(", ")
    );
    let open = strip_comments(&settings_text).find('{')?;
    // A `//` runs to the end of its line: in a file kept on one line
    // (`{ "a": 1 }`) the rest of the object would become part of the note.
    if !matches!(settings_text.as_bytes().get(open + 1), Some(b'\n' | b'\r')) {
        note.push('\n');
    }
    settings_text.insert_str(open + 1, &note);
    Some(Moved {
        settings: settings_text,
        theme: theme_text,
        keys,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_doc::default_ui_theme_text;
    use crate::ui_theme::parse;
    use serde_json::json;

    fn seed() -> String {
        default_ui_theme_text()
    }

    // The shape of a real hand-kept file: header, notes, a nested group, a
    // list, a trailing comma, an URL with slashes.
    const REAL: &str = r#"// My settings. Anything here overrides settings.default.json.
{
  "theme": "Violite", // terminal colours
  "terminal.fontSize": 16,

  // I like it a bit see-through
  "ui.windowOpacity": 0.92,
  "ui.windowBlur": true,
  "ui.backgroundImage": [
    "dusk",
    "~/Pictures/lake.jpg", // https://example.com/not-a-comment
  ],
  "ui.cardRadius": 10,
  "ui.panStep": 120,
  "ui": {
    "fontFamily": "SF Pro", // chrome font
    "showFps": true,
  },
  "startingDir": "~/Code",
}
"#;

    #[test]
    fn nothing_to_move_means_none() {
        let s = "{\n  \"terminal.fontSize\": 16,\n  \"ui.panStep\": 100\n}\n";
        assert_eq!(migrate(s, None, &seed(), "default"), None);
        assert_eq!(migrate("{\n}\n", None, &seed(), "default"), None);
    }

    #[test]
    fn a_file_that_does_not_parse_is_left_alone() {
        assert_eq!(
            migrate("{ \"ui.cardRadius\": ", None, &seed(), "default"),
            None
        );
        assert_eq!(migrate("[1]", None, &seed(), "default"), None);
        assert_eq!(
            migrate(
                "{\"ui.cardRadius\": 4}",
                Some("{ broken"),
                &seed(),
                "default"
            ),
            None
        );
    }

    #[test]
    fn moves_every_look_key_and_keeps_the_rest_of_a_real_file() {
        let m = migrate(REAL, None, &seed(), "default").unwrap();
        let settings = parse_jsonc(&m.settings).unwrap();
        assert_eq!(
            settings,
            json!({
                "theme": "Violite",
                "terminal.fontSize": 16,
                "ui.panStep": 120,
                "ui": {"showFps": true},
                "startingDir": "~/Code",
            })
        );
        // Everything a person wrote besides the moved lines is still there.
        assert!(m.settings.starts_with("// My settings."));
        assert!(m.settings.contains("// terminal colours"));
        assert!(m.settings.contains("\"startingDir\": \"~/Code\""));
        // And the new file says what moved.
        assert!(m.settings.contains("ui-themes/default.json"));
        assert!(m.settings.contains("ui.windowOpacity, ui.windowBlur"));
        assert_eq!(m.keys.len(), 5);
    }

    #[test]
    fn the_theme_gets_the_values_and_still_parses_as_a_ui_theme() {
        let m = migrate(REAL, None, &seed(), "default").unwrap();
        let t = parse(&m.theme).unwrap();
        assert_eq!(t.look["ui.windowOpacity"], json!(0.92));
        assert_eq!(t.look["ui.windowBlur"], json!(true));
        assert_eq!(
            t.look["ui.backgroundImage"],
            json!(["dusk", "~/Pictures/lake.jpg"])
        );
        assert_eq!(t.look["ui.cardRadius"], json!(10));
        // The nested spelling moved too.
        assert_eq!(t.look["ui.fontFamily"], json!("SF Pro"));
        assert_eq!(t.look.len(), 5);
        // The reference comments of the seed survive.
        assert!(m.theme.contains("A ui theme: how infiniterm looks"));
    }

    #[test]
    fn a_second_run_finds_nothing() {
        let m = migrate(REAL, None, &seed(), "default").unwrap();
        assert_eq!(
            migrate(&m.settings, Some(&m.theme), &seed(), "default"),
            None
        );
    }

    #[test]
    fn the_result_still_passes_the_settings_validation() {
        let m = migrate(REAL, None, &seed(), "default").unwrap();
        let raw = parse_jsonc(&m.settings).unwrap();
        assert!(crate::model::settings_in::validate_settings(&raw).is_ok());
        assert!(crate::ui_theme::strip_look(&raw).1.is_empty());
    }

    #[test]
    fn a_trailing_comma_and_a_last_entry_cut_leave_a_file_that_parses() {
        let s = "{\n  \"a\": 1,\n  \"ui.showGrid\": false\n}\n";
        let m = migrate(s, None, &seed(), "default").unwrap();
        assert_eq!(parse_jsonc(&m.settings).unwrap(), json!({"a": 1}));
        let s = "{ \"ui.showGrid\": false, }";
        let m = migrate(s, None, &seed(), "default").unwrap();
        assert_eq!(parse_jsonc(&m.settings).unwrap(), json!({}));
        // A file kept on one line: the note must not swallow the closing brace.
        let s = "{\"terminal.fontSize\": 16, \"ui.cardRadius\": 4}";
        let m = migrate(s, None, &seed(), "default").unwrap();
        assert_eq!(
            parse_jsonc(&m.settings).unwrap(),
            json!({"terminal.fontSize": 16})
        );
    }

    #[test]
    fn both_spellings_of_one_key_are_cut() {
        let s = "{\n  \"ui\": {\"cardRadius\": 3},\n  \"ui.cardRadius\": 9\n}\n";
        let m = migrate(s, None, &seed(), "default").unwrap();
        // `flatten` lets the flat spelling win, so that is the value that moves.
        assert_eq!(parse(&m.theme).unwrap().look["ui.cardRadius"], json!(9));
        assert_eq!(
            crate::ui_theme::strip_look(&parse_jsonc(&m.settings).unwrap()).1,
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_existing_theme_is_patched_in_place_and_its_own_value_wins() {
        let theme =
            "// mine\n{\n  \"ui.cardRadius\": 20, // round\n  \"titlebar.border\": \"none\"\n}\n";
        let s = "{\n  \"ui.cardRadius\": 3,\n  \"ui.showGrid\": false\n}\n";
        let m = migrate(s, Some(theme), &seed(), "default").unwrap();
        assert!(m.theme.starts_with("// mine\n"));
        assert!(m.theme.contains("// round"));
        let t = parse(&m.theme).unwrap();
        assert_eq!(t.look["ui.cardRadius"], json!(20));
        assert_eq!(t.look["ui.showGrid"], json!(false));
        assert!(t.titlebar.border.is_some());
        // The settings file loses the key either way.
        assert!(!m.settings.contains("\"ui.cardRadius\""));
    }

    #[test]
    fn works_on_a_bare_empty_settings_file_with_only_look_keys() {
        let s = "// mine\n{\n  \"ui.windowColor\": \"teal\",\n}\n";
        let m = migrate(s, None, "{\n}\n", "default").unwrap();
        assert_eq!(
            parse(&m.theme).unwrap().look["ui.windowColor"],
            json!("teal")
        );
        assert_eq!(parse_jsonc(&m.settings).unwrap(), json!({}));
        assert!(m.settings.starts_with("// mine\n{"));
    }
}
