//! Applying the config directory's files to the model, and keeping them
//! applied as they change. Port of `settings.svelte.ts` minus the CSS.
//!
//! A broken file falls back rather than failing: a config file is edited by
//! hand, so it WILL be broken sometimes, and a terminal that refuses to
//! start over a stray comma is worse than one that ignores it. Writing a
//! setting back goes through `patch_json_text`, never a reserialise, since
//! the user's comments live in that file.
use super::{Effect, Model};
use crate::config::{default_config, merge_config};
use crate::jsonc::{parse_jsonc, patch_json_text};
use crate::keymap::{default_keymap, merge_keymap, parse_keymap};

/// The first thing a new `settings.json` says, since an empty file explains nothing.
pub const EMPTY_SETTINGS: &str = "// Your settings. Anything here overrides settings.default.json beside it,\n// which lists everything that can be set, with comments.\n//\n// Comments and trailing commas are allowed.\n{\n}\n";

pub const EMPTY_KEYBINDINGS: &str = "// Your keybindings. Anything here overrides keybindings.default.json beside it.\n// Set a chord to null to unbind it and give the key back to the terminal.\n//\n// Comments and trailing commas are allowed.\n{\n}\n";

impl Model {
    pub fn apply_settings_text(&mut self, text: &str) {
        match parse_jsonc(text) {
            Ok(raw) => {
                self.config = merge_config(&raw);
                self.settings_error = None;
            }
            Err(e) => {
                self.settings_error = Some(e.to_string());
                self.effects
                    .push(Effect::Warn(format!("settings.json: {e}")));
                self.config = default_config();
            }
        }
        if let Some(dir) = Some(self.config.starting_dir.clone()).filter(|d| !d.is_empty()) {
            self.start_dir = dir;
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
            Err(e) => {
                self.effects
                    .push(Effect::Warn(format!("keybindings.json: {e}")));
                self.keymap = default_keymap();
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
