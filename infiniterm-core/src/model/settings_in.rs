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

/// What a new `keybindings.json` says: the file's job, then commented examples
/// to switch on by deleting the `//` (every example line starts with
/// `//   "`; a test parses them and checks each command exists, so they cannot
/// rot). The `when` examples are the part nobody finds on their own (#269).
pub const EMPTY_KEYBINDINGS: &str = concat!(
    "// Your keybindings. Anything here overrides keybindings.default.json beside it.\n",
    "// Set a chord to null to unbind it and give the key back to the terminal.\n",
    "// A binding can carry a \"when\" (see keybindings.default.json for the shape and the keys).\n",
    "//\n",
    "// Comments and trailing commas are allowed.\n",
    "//\n",
    "// Examples. Delete the // in front of a line to use it:\n",
    "//\n",
    "// Another chord to leave a locked editor (Cmd+Escape is the default):\n",
    "//   \"cmd+shift+e\": \"browser.leave\",\n",
    "// A single Escape leaves a locked editor when it has nothing else to close;\n",
    "// to turn that off:\n",
    "//   \"escape\": {\"command\": null, \"when\": \"editorTextFocus\"},\n",
    "// Go to a line with F2 in a locked editor, but not while its popup is open:\n",
    "//   \"f2\": {\"command\": \"editor.goToLine\", \"when\": \"editorTextFocus && !suggestWidgetVisible\"},\n",
    "// Move workspace tabs from the keyboard:\n",
    "//   \"cmd+ctrl+shift+[\": \"workspace.reorder.left\",\n",
    "//   \"cmd+ctrl+shift+]\": \"workspace.reorder.right\",\n",
    "// Step through the themes:\n",
    "//   \"cmd+ctrl+[\": \"theme.prev\",\n",
    "//   \"cmd+ctrl+]\": \"theme.next\",\n",
    "// Open a file, and start Claude Code in the card's directory:\n",
    "//   \"cmd+o\": \"card.open.file\",\n",
    "//   \"cmd+alt+c\": \"card.new.claude\",\n",
    "{\n}\n"
);

impl Model {
    /// A ui theme's text arrived (#511): `text` is the file's contents, `None`
    /// when `ui-themes/<name>.json` does not exist. The look it sets is laid
    /// over `config.ui`. A file with a mistake, or a name with no file (other
    /// than `default`, which is the built-in look without one), is not applied
    /// and the look in force stays: the rule settings.json follows (#246).
    pub fn apply_ui_theme_text(&mut self, name: &str, text: Option<&str>) {
        let parsed = match text {
            Some(t) => crate::ui_theme::parse(t).map_err(|e| {
                format!("ui-themes/{name}.json not applied, the look you had stays: {e}")
            }),
            None if name == crate::ui_theme::DEFAULT_NAME => Ok(Default::default()),
            None => Err(format!(
                "ui theme \"{name}\" has no file in ui-themes/, the look you had stays"
            )),
        };
        match parsed {
            Ok(theme) => {
                self.look_ui = merge_config(&serde_json::Value::Object(theme.look.clone())).ui;
                self.ui_theme = theme;
                self.ui_theme_name = name.to_string();
                self.config.ui.take_look_from(&self.look_ui);
                self.sync_local_color();
            }
            Err(e) => self.effects.push(Effect::Warn(e)),
        }
    }

    /// The watcher saw `ui-themes/<name>.json` change. Only the theme in force
    /// matters (every file is watched, see `config_files::poll_ui_themes`); one
    /// that never loaded (a mistake at startup) is retried by its name.
    pub fn ui_theme_file_changed(&mut self, name: &str, text: &str) {
        let in_force = name == self.ui_theme_name
            || (self.ui_theme_name.is_empty() && name == self.ui_theme_setting);
        if in_force {
            self.apply_ui_theme_text(name, Some(text));
        }
    }

    /// `ui.theme` as the settings say it. Only a CHANGE loads a file, so a
    /// preview in the picker is not undone by an unrelated settings edit.
    pub fn note_ui_theme_setting(&mut self, name: &str) {
        if name != self.ui_theme_setting {
            self.ui_theme_setting = name.to_string();
            self.effects.push(Effect::LoadUiTheme(name.to_string()));
        }
    }

    /// The local instance's colour is a look key, so a ui theme change moves
    /// the title bar and the Dock icon too; a remote window keeps its host's.
    fn sync_local_color(&mut self) {
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
    }

    pub fn apply_settings_text(&mut self, text: &str) {
        self.apply_settings_from(text, true);
    }

    /// The HOST's settings.json in a joined viewer (#433): everything applies
    /// but `ui.theme`, because the look is per client (#511); the viewer's
    /// own choice comes from `note_ui_theme_setting`.
    pub fn apply_host_settings_text(&mut self, text: &str) {
        self.apply_settings_from(text, false);
    }

    fn apply_settings_from(&mut self, text: &str, own: bool) {
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
                // The look keys are a ui theme's now (#511). Dropped, not
                // refused: a valid file must never be refused, and a joined
                // viewer is sent a host's file that may be older than this
                // build. The startup move (`look_migration`) takes them out
                // of the user's own file, so a notice here means a key typed
                // by hand since.
                let (raw, dropped) = crate::ui_theme::strip_look(&raw);
                if !dropped.is_empty() && own {
                    self.effects.push(Effect::Warn(format!(
                        "settings.json: {} belong in a ui theme (ui-themes/{}.json), ignored here",
                        dropped.join(", "),
                        crate::ui_theme::DEFAULT_NAME
                    )));
                }
                self.config = merge_config(&raw);
                self.config.ui.take_look_from(&self.look_ui);
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
        self.sync_local_color();
        // Which ui theme is the LOCAL client's choice: a joined viewer keeps
        // its own and ignores the host's `ui.theme` (see `ui_theme_name_in`).
        if own {
            let name = self.config.ui.theme.clone();
            self.note_ui_theme_setting(&name);
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

    /// What a binding's `when` can ask about, as of this moment (#269).
    pub fn key_context(&self) -> crate::when::Context {
        use crate::saved_layout::CardKind;
        let card = self.focused();
        let kind = match card.map(|c| c.kind) {
            None => "none",
            Some(CardKind::Terminal) => "terminal",
            Some(CardKind::Editor) => "editor",
            Some(CardKind::Browser) => "browser",
            Some(CardKind::Diff) => "diff",
            Some(CardKind::Transcript) => "transcript",
            Some(CardKind::Page) => "page",
        };
        let overlay = if self.palette_open() {
            "palette"
        } else if self.prompt.is_open() {
            "prompt"
        } else if self.omni.open {
            "omnibox"
        } else if self.shortcuts_open {
            "shortcuts"
        } else if self.whats_new_open {
            "whatsNew"
        } else if self.find.open {
            "find"
        } else if self.switcher.is_some() {
            "switcher"
        } else {
            "none"
        };
        crate::when::Context {
            card_kind: kind,
            card_locked: card.is_some_and(|c| {
                c.locked && (c.kind.has_editor_body() || c.kind == CardKind::Browser)
            }),
            overlay,
            phantom_focus: card.is_none() && self.selection.phantom.is_some(),
            multi_selection: !self.selection.extra.is_empty(),
            suggest_widget_visible: self.ui_context.suggest_widget_visible,
            find_widget_visible: self.find.open || self.ui_context.find_widget_visible,
            editor_has_selection: self.ui_context.editor_has_selection,
            terminal_has_selection: self.ui_context.terminal_has_selection,
        }
    }

    /// The binding a user's `when` gives `chord` right now: `Some(Some(id))` to
    /// run it, `Some(None)` when it unbinds the chord in this context, `None`
    /// when no conditional binding applies and the keymap decides. The last
    /// matching one in the file wins.
    pub fn conditional_for(&self, chord: &str) -> Option<Option<String>> {
        if self.conditional_keys.is_empty() {
            return None;
        }
        let ctx = self.key_context();
        self.conditional_keys
            .iter()
            .rev()
            .find(|b| b.chord == chord && b.when.as_ref().is_some_and(|w| w.eval(&ctx)))
            .map(|b| b.command.clone())
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
                self.conditional_keys = parsed.conditional;
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

/// The `ui.theme` a settings text names, or `None` when it says none or does
/// not parse. A joined viewer reads its OWN settings.json for this alone
/// (#511): everything else in that window is the host's.
pub fn ui_theme_name_in(text: &str) -> Option<String> {
    let raw = parse_jsonc(text).ok()?;
    let flat = flatten(raw.as_object()?);
    let name = flat.get("ui.theme")?.as_str()?.trim();
    (!name.is_empty()).then(|| name.to_string())
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
