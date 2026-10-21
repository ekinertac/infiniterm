//! `window.color`: change the colour of a window (#157, #160), in two steps.
//!
//! The first is a palette of named colours previewed live (the title bar and
//! the Dock icon follow the highlight, Escape puts the colour back), with a row
//! to keep the current one, one for the default, and one for a custom hex code,
//! which opens a prompt as the second step.
//!
//! Every instance has it. In a remote instance (`ift connect`) the colour is
//! the host's: the default is hashed from the host name and a choice is saved
//! as `remote.json` in the instance's folder, which the ui writes. In the local
//! instance the colour is optional: the default is none (the normal title bar
//! and the normal icon) and a choice is the setting `ui.windowColor`, saved to
//! settings.json the way the theme is.
//!
//! The model holds the colour (`Model::window_color`, which the title bar reads
//! every frame); the Dock icon and `remote.json` are the ui's, reached through
//! `Effect::WindowColor`.
//!
//! Called by `register.rs` (the command), `palette_state.rs` (the rows, the
//! preview and the run), `cards_cmd.rs` (the hex prompt's answer) and
//! `settings_in.rs` (the setting). Related: `remote_identity.rs` (the names,
//! the hex, the file).

use super::palette_state::Source;
use super::{Effect, Model, Pending, WindowColorSave};
use crate::palette::PaletteItem;
use crate::remote_identity::{color_for, name_of, named, parse_hex, to_hex, Rgb, NAMED};

/// Row ids that are not a colour's name.
const KEEP: &str = "keep";
const AUTO: &str = "auto";
const CUSTOM: &str = "custom";

/// The setting the local instance's colour lives in.
pub const SETTING: &str = "ui.windowColor";

impl Model {
    /// The colour the window wears: the remote host's, else the local choice,
    /// else none.
    pub fn window_color(&self) -> Option<Rgb> {
        self.remote.as_ref().map(|r| r.color).or(self.local_color)
    }

    /// What "default" means here: the host's hashed colour, or no colour.
    fn auto_color(&self) -> Option<Rgb> {
        self.remote.as_ref().map(|r| color_for(&r.target))
    }

    /// Rows for the picker. The first keeps what you have (so opening it
    /// previews nothing new), the last asks for a hex code.
    pub(super) fn window_color_items(&self) -> Vec<PaletteItem> {
        let current = self
            .window_color_before_preview
            .unwrap_or_else(|| self.window_color());
        let said = match current {
            Some(c) => name_of(c).map(str::to_string).unwrap_or_else(|| to_hex(c)),
            None => "none".to_string(),
        };
        let mut items = vec![
            PaletteItem {
                id: KEEP.into(),
                label: format!("Current: {said}"),
                hint: Some("active".into()),
            },
            PaletteItem {
                id: AUTO.into(),
                label: if self.remote.is_some() {
                    "Default: from the host name".into()
                } else {
                    "None: the normal title bar and icon".into()
                },
                hint: None,
            },
        ];
        items.extend(NAMED.iter().map(|(name, hex)| PaletteItem {
            id: (*name).into(),
            label: capitalised(name),
            hint: Some(format!("#{hex}")),
        }));
        items.push(PaletteItem {
            id: CUSTOM.into(),
            label: "Custom hex colour…".into(),
            hint: Some("like 3b82f6".into()),
        });
        items
    }

    /// The colour a row stands for: `Some(None)` is "no colour" (the local
    /// default); `None` is a row that is not a colour.
    fn row_color(&self, id: &str) -> Option<Option<Rgb>> {
        match id {
            KEEP | CUSTOM => self.window_color_before_preview,
            AUTO => Some(self.auto_color()),
            name => named(name).map(Some),
        }
    }

    /// The highlight moved (`Some`) or the picker was dismissed (`None`).
    pub(super) fn window_color_preview(&mut self, id: Option<&str>) {
        let Some(before) = self.window_color_before_preview else {
            return;
        };
        let color = id.and_then(|i| self.row_color(i)).unwrap_or(before);
        self.apply_window_color(color, WindowColorSave::No);
        if id.is_none() {
            self.window_color_before_preview = None;
        }
    }

    /// Enter on a row.
    pub(super) fn window_color_run(&mut self, id: &str) {
        let before = self.window_color_before_preview.take();
        match id {
            KEEP => {
                if let Some(b) = before {
                    self.apply_window_color(b, WindowColorSave::No);
                }
            }
            CUSTOM => {
                if let Some(b) = before {
                    self.apply_window_color(b, WindowColorSave::No);
                }
                let start = before.flatten().map(to_hex).unwrap_or_default();
                self.prompt.ask(
                    "colour: a hex code like 3b82f6",
                    &start,
                    Pending::WindowColor,
                );
            }
            AUTO => {
                let c = self.auto_color();
                self.apply_window_color(c, WindowColorSave::Forget);
            }
            name => {
                if let Some(c) = named(name) {
                    self.apply_window_color(Some(c), WindowColorSave::Save);
                }
            }
        }
    }

    /// The hex prompt's answer: a code, or a notice saying what a code is.
    pub(super) fn window_color_answer(&mut self, text: Option<String>) {
        let Some(text) = text else { return };
        match parse_hex(&text) {
            Some(c) => self.apply_window_color(Some(c), WindowColorSave::Save),
            None => self.notify("not a colour: six hex digits, like 3b82f6"),
        }
    }

    /// Sets the colour and says so to the ui. A local choice is saved here as
    /// a setting (by name when it has one); a remote one is saved by the ui as
    /// `remote.json`.
    fn apply_window_color(&mut self, color: Option<Rgb>, save: WindowColorSave) {
        match &mut self.remote {
            Some(r) => {
                if let Some(c) = color {
                    r.color = c;
                }
            }
            None => {
                self.local_color = color;
                if save != WindowColorSave::No {
                    let value = match (save, color) {
                        (WindowColorSave::Save, Some(c)) => {
                            name_of(c).map(str::to_string).unwrap_or_else(|| to_hex(c))
                        }
                        _ => String::new(),
                    };
                    self.effects.push(Effect::SaveSetting {
                        path: SETTING.into(),
                        value: serde_json::Value::String(value),
                    });
                }
            }
        }
        let color = self.window_color();
        self.effects.push(Effect::WindowColor { color, save });
    }
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

pub fn register(r: &mut crate::commands::CommandRegistry<Model>) {
    r.register(
        "window.color",
        "Window: change the title bar and Dock colour…",
        |m| {
            m.window_color_before_preview = Some(m.window_color());
            m.open_palette(Source::WindowColor);
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_identity::RemoteIdentity;

    fn remote_model() -> Model {
        let mut m = Model::new();
        m.remote = RemoteIdentity::from_vars(Some("ops@box"), None, Some("ff8800"));
        m
    }

    fn open(m: &mut Model) {
        m.window_color_before_preview = Some(m.window_color());
        m.open_palette(Source::WindowColor);
    }

    fn colour(m: &Model) -> Rgb {
        m.window_color().unwrap()
    }

    fn saved_setting(m: &Model) -> Option<String> {
        m.effects.iter().find_map(|e| match e {
            Effect::SaveSetting { path, value } if path == SETTING => {
                value.as_str().map(str::to_string)
            }
            _ => None,
        })
    }

    #[test]
    fn every_instance_has_the_command() {
        let mut m = Model::new();
        let mut r = crate::commands::CommandRegistry::new(|_| {});
        register(&mut r);
        assert!(r.run("window.color", &mut m));
        assert!(m.palette_open(), "the local instance too");
        assert_eq!(
            m.window_color_before_preview,
            Some(None),
            "it had no colour"
        );
    }

    #[test]
    fn the_picker_lists_the_current_colour_first_then_the_default_the_names_and_custom() {
        let mut m = remote_model();
        open(&mut m);
        let items = m.palette_items(Source::WindowColor, &[]);
        assert_eq!(items[0].id, "keep");
        assert_eq!(items[0].hint.as_deref(), Some("active"));
        assert_eq!(
            items[0].label, "Current: ff8800",
            "an unnamed colour shows as hex"
        );
        assert_eq!(items[1].id, "auto");
        assert_eq!(items[1].label, "Default: from the host name");
        assert_eq!(items.last().unwrap().id, "custom");
        let blue = items.iter().find(|i| i.id == "blue").unwrap();
        assert_eq!(
            (blue.label.as_str(), blue.hint.as_deref()),
            ("Blue", Some("#3b82f6"))
        );
        assert_eq!(items.len(), NAMED.len() + 3);
        assert!(Source::WindowColor.keeps_its_order());
    }

    #[test]
    fn moving_the_highlight_previews_and_escape_puts_the_colour_back() {
        let mut m = remote_model();
        open(&mut m);
        m.effects.clear();
        m.palette_preview(Source::WindowColor, Some("blue"));
        assert_eq!(colour(&m), named("blue").unwrap());
        assert!(
            m.effects.iter().any(|e| matches!(
                e,
                Effect::WindowColor {
                    save: WindowColorSave::No,
                    ..
                }
            )),
            "the Dock follows, nothing is saved"
        );
        m.palette_preview(Source::WindowColor, Some("custom"));
        assert_eq!(
            colour(&m),
            (0xff, 0x88, 0x00),
            "the custom row shows what you have"
        );
        m.palette_preview(Source::WindowColor, Some("rose"));
        m.close_palette(false);
        assert_eq!(
            colour(&m),
            (0xff, 0x88, 0x00),
            "dismissed: back to the start"
        );
        assert!(m.window_color_before_preview.is_none());
    }

    #[test]
    fn a_remote_choice_is_kept_and_left_for_the_ui_to_save() {
        let mut m = remote_model();
        open(&mut m);
        m.effects.clear();
        m.palette_run(Source::WindowColor, "teal");
        assert_eq!(colour(&m), named("teal").unwrap());
        assert!(m.effects.iter().any(|e| matches!(e, Effect::WindowColor { save: WindowColorSave::Save, color } if *color == named("teal"))));
        assert_eq!(
            saved_setting(&m),
            None,
            "a remote window does not touch settings.json"
        );
    }

    #[test]
    fn the_default_row_goes_back_to_the_hashed_colour_and_forgets_the_choice() {
        let mut m = remote_model();
        open(&mut m);
        m.effects.clear();
        m.palette_run(Source::WindowColor, "auto");
        assert_eq!(colour(&m), color_for("ops@box"));
        assert!(m.effects.iter().any(|e| matches!(
            e,
            Effect::WindowColor {
                save: WindowColorSave::Forget,
                ..
            }
        )));
    }

    #[test]
    fn keeping_the_current_colour_changes_nothing() {
        let mut m = remote_model();
        open(&mut m);
        m.palette_preview(Source::WindowColor, Some("pink"));
        m.palette_run(Source::WindowColor, "keep");
        assert_eq!(colour(&m), (0xff, 0x88, 0x00));
    }

    #[test]
    fn the_custom_row_asks_for_a_hex_code_and_applies_a_good_one() {
        let mut m = remote_model();
        open(&mut m);
        m.palette_run(Source::WindowColor, "custom");
        assert!(m.prompt.is_open(), "the second step is a prompt");
        assert_eq!(colour(&m), (0xff, 0x88, 0x00), "nothing changed yet");
        m.effects.clear();
        m.window_color_answer(Some("#12ab34".into()));
        assert_eq!(colour(&m), (0x12, 0xab, 0x34));
        assert!(m.effects.iter().any(|e| matches!(
            e,
            Effect::WindowColor {
                save: WindowColorSave::Save,
                ..
            }
        )));
        m.notice = None;
        m.window_color_answer(Some("blue-ish".into()));
        assert_eq!(colour(&m), (0x12, 0xab, 0x34));
        assert!(m.notice.as_deref().unwrap_or("").contains("six hex digits"));
        m.notice = None;
        m.window_color_answer(None);
        assert!(m.notice.is_none());
    }

    // The local instance (#160): no colour by default, a choice is a setting.
    #[test]
    fn the_local_picker_starts_at_none_and_its_default_row_says_so() {
        let mut m = Model::new();
        open(&mut m);
        let items = m.palette_items(Source::WindowColor, &[]);
        assert_eq!(items[0].label, "Current: none");
        assert_eq!(items[1].label, "None: the normal title bar and icon");
        assert_eq!(m.window_color(), None);
    }

    #[test]
    fn a_local_choice_is_saved_as_a_setting_by_name_or_by_hex() {
        let mut m = Model::new();
        open(&mut m);
        m.effects.clear();
        m.palette_preview(Source::WindowColor, Some("teal"));
        assert_eq!(m.window_color(), named("teal"), "previewed");
        assert_eq!(saved_setting(&m), None, "a preview saves nothing");
        m.palette_run(Source::WindowColor, "teal");
        assert_eq!(saved_setting(&m).as_deref(), Some("teal"));
        assert!(m.effects.iter().any(|e| matches!(
            e,
            Effect::WindowColor {
                save: WindowColorSave::Save,
                ..
            }
        )));
        m.effects.clear();
        m.window_color_answer(Some("12ab34".into()));
        assert_eq!(
            saved_setting(&m).as_deref(),
            Some("12ab34"),
            "no name: the hex"
        );
        assert_eq!(m.window_color(), Some((0x12, 0xab, 0x34)));
    }

    #[test]
    fn the_local_none_row_removes_the_colour_and_saves_an_empty_setting() {
        let mut m = Model::new();
        m.local_color = named("blue");
        open(&mut m);
        m.effects.clear();
        m.palette_run(Source::WindowColor, "auto");
        assert_eq!(m.window_color(), None);
        assert_eq!(saved_setting(&m).as_deref(), Some(""));
        assert!(
            m.effects
                .iter()
                .any(|e| matches!(e, Effect::WindowColor { color: None, .. })),
            "the Dock goes back to the plain icon"
        );
    }

    #[test]
    fn dismissing_the_local_picker_puts_back_none() {
        let mut m = Model::new();
        open(&mut m);
        m.palette_preview(Source::WindowColor, Some("red"));
        assert!(m.window_color().is_some());
        m.close_palette(false);
        assert_eq!(m.window_color(), None);
    }

    #[test]
    fn a_remote_window_wears_the_hosts_colour_not_the_local_setting() {
        let mut m = remote_model();
        m.local_color = named("blue");
        assert_eq!(m.window_color(), Some((0xff, 0x88, 0x00)));
    }
}
