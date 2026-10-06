//! Applying the config directory's files to the model, and keeping them
//! applied as they change. Port of `settings.svelte.ts` minus the CSS.
//!
//! A broken file falls back rather than failing: a config file is edited by
//! hand, so it WILL be broken sometimes, and a terminal that refuses to
//! start over a stray comma is worse than one that ignores it. Writing a
//! setting back goes through `patch_json_text`, never a reserialise, since
//! the user's comments live in that file.
use super::{Effect, Model};
use crate::config::{flatten, merge_config};
use crate::jsonc::{parse_jsonc, patch_json_text};
use crate::keymap::{default_keymap, merge_keymap, parse_keymap};
use crate::settings_doc::default_settings_text;

/// The first thing a new `settings.json` says, since an empty file explains nothing.
pub const EMPTY_SETTINGS: &str = "// Your settings. Anything here overrides settings.default.json beside it,\n// which lists everything that can be set, with comments.\n//\n// Comments and trailing commas are allowed.\n{\n}\n";

pub const EMPTY_KEYBINDINGS: &str = "// Your keybindings. Anything here overrides keybindings.default.json beside it.\n// Set a chord to null to unbind it and give the key back to the terminal.\n//\n// Comments and trailing commas are allowed.\n{\n}\n";

impl Model {
    pub fn apply_settings_text(&mut self, text: &str) {
        // Checked BEFORE it is applied: a file with an error is not accepted,
        // and the settings in force stay (#246). It used to fall back to the
        // defaults, which read as the app losing every setting (font,
        // background, window colour) after one stray comma. At launch there is
        // nothing yet, and the defaults the model starts with apply.
        match parse_jsonc(text)
            .map_err(|e| e.to_string())
            .and_then(|raw| validate_settings(&raw).map(|()| raw))
        {
            Ok(raw) => {
                self.config = merge_config(&raw);
                self.settings_error = None;
            }
            Err(e) => {
                self.settings_error = Some(e.clone());
                self.effects.push(Effect::Warn(format!(
                    "settings.json not applied, the settings you had stay: {e}"
                )));
            }
        }
        if let Some(dir) = Some(self.config.starting_dir.clone()).filter(|d| !d.is_empty()) {
            self.start_dir = dir;
        }
        // The local instance's colour is a setting, so editing the file changes
        // the title bar and the Dock icon too; a remote window keeps its host's.
        if self.remote.is_none() {
            let color = crate::remote_identity::parse_color(&self.config.ui.window_color);
            if color != self.local_color {
                self.local_color = color;
                self.effects.push(Effect::WindowColor {
                    color,
                    save: super::WindowColorSave::No,
                });
            }
        }
        // A remote instance's cards start in the server's home, whatever
        // folder the local settings it was seeded from name: that one is a
        // Mac path the server has not got (`ift proxy` expands the `~`).
        if self.remote.is_some() {
            self.start_dir = "~".into();
        }
        if let Some(theme) = self.config.theme.clone() {
            self.effects.push(Effect::LoadTheme(theme));
        }
    }

    pub fn apply_keymap_text(&mut self, text: &str) {
        match parse_jsonc(text) {
            Ok(raw) => {
                let parsed = parse_keymap(&raw);
                for error in parsed.errors {
                    self.effects
                        .push(Effect::Warn(format!("keybindings.json: {error}")));
                }
                self.keymap = merge_keymap(&default_keymap(), &parsed.bindings);
            }
            // Not accepted: the bindings you had stay (#246), like settings.json.
            Err(e) => {
                self.effects.push(Effect::Warn(format!(
                    "keybindings.json not applied, the bindings you had stay: {e}"
                )));
            }
        }
    }

    /// The user's file with one setting changed, or nothing when the value
    /// is already there.
    pub fn patched_settings(
        existing: Option<&str>,
        path: &str,
        value: &serde_json::Value,
    ) -> Option<String> {
        patch_json_text(existing.unwrap_or(EMPTY_SETTINGS), path, value)
    }
}

/// Why a parsed settings file must not be applied, or `Ok`. Two things are
/// errors: the file is not one object, and a setting that has a number, a
/// boolean or text for its default holds a different one of those (`"terminal.fontSize":
/// "big"`). Nothing else is: an unknown name, an out-of-range number (it is
/// clamped), and an array, an object or null where a scalar belongs (a list
/// is `ui.backgroundImage`'s other shape) all pass, because a valid file must
/// never be refused over a rule too strict to be right.
pub fn validate_settings(raw: &serde_json::Value) -> Result<(), String> {
    let Some(user) = raw.as_object() else {
        return Err("the file must hold one object, { ... }".into());
    };
    let defaults = parse_jsonc(&default_settings_text())
        .ok()
        .and_then(|v| v.as_object().map(flatten))
        .unwrap_or_default();
    let kind = |v: &serde_json::Value| match v {
        serde_json::Value::Number(_) => Some("a number"),
        serde_json::Value::Bool(_) => Some("true or false"),
        serde_json::Value::String(_) => Some("text"),
        _ => None,
    };
    for (key, value) in flatten(user) {
        let Some(default) = defaults.get(&key) else {
            continue;
        };
        if let (Some(want), Some(got)) = (kind(default), kind(&value)) {
            if want != got {
                return Err(format!("\"{key}\" should be {want}, not {got}"));
            }
        }
    }
    Ok(())
}
