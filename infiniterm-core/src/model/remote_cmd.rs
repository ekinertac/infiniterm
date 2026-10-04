//! `remote.color`: change the colour of a remote instance (#157), in two steps.
//!
//! The first is a palette of named colours previewed live (the title bar and
//! the Dock icon follow the highlight, Escape puts the colour back), with a row
//! to keep the current one, one for the default hashed from the host name, and
//! one for a custom hex code, which opens a prompt as the second step.
//!
//! The model changes `Model::remote`'s colour, which the title bar reads every
//! frame; the Dock icon and the saved `remote.json` are the ui's, reached
//! through `Effect::RemoteColor`. Only a remote instance has the command.
//!
//! Called by `register.rs` (the command), `palette_state.rs` (the source's rows,
//! preview and run) and `cards_cmd.rs` (the hex prompt's answer). Related:
//! `remote_identity.rs` (the names, the hex, the file).

use super::palette_state::Source;
use super::{Effect, Model, Pending, RemotePersist};
use crate::palette::PaletteItem;
use crate::remote_identity::{color_for, name_of, named, parse_hex, to_hex, Rgb, NAMED};

/// Row ids that are not a colour's name.
const KEEP: &str = "keep";
const AUTO: &str = "auto";
const CUSTOM: &str = "custom";

impl Model {
    /// The colour in force, or `None` outside a remote instance.
    fn remote_color(&self) -> Option<Rgb> {
        self.remote.as_ref().map(|r| r.color)
    }

    /// Rows for the picker. The first keeps what you have (so opening it
    /// previews nothing new), the last asks for a hex code.
    pub(super) fn remote_color_items(&self) -> Vec<PaletteItem> {
        let current = self
            .remote_color_before_preview
            .or_else(|| self.remote_color());
        let label_of = |c: Rgb| name_of(c).map(str::to_string).unwrap_or_else(|| to_hex(c));
        let mut items = vec![PaletteItem {
            id: KEEP.into(),
            label: format!("Current: {}", current.map(label_of).unwrap_or_default()),
            hint: Some("active".into()),
        }];
        items.push(PaletteItem {
            id: AUTO.into(),
            label: "Default: from the host name".into(),
            hint: None,
        });
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

    /// The colour a row stands for.
    fn remote_color_of_row(&self, id: &str) -> Option<Rgb> {
        match id {
            KEEP | CUSTOM => self.remote_color_before_preview,
            AUTO => self.remote.as_ref().map(|r| color_for(&r.target)),
            name => named(name),
        }
    }

    /// The highlight moved (`Some`) or the picker was dismissed (`None`).
    pub(super) fn remote_color_preview(&mut self, id: Option<&str>) {
        let Some(before) = self.remote_color_before_preview else {
            return;
        };
        let color = id
            .and_then(|i| self.remote_color_of_row(i))
            .unwrap_or(before);
        self.set_remote_color(color, RemotePersist::No);
        if id.is_none() {
            self.remote_color_before_preview = None;
        }
    }

    /// Enter on a row.
    pub(super) fn remote_color_run(&mut self, id: &str) {
        let before = self.remote_color_before_preview.take();
        match id {
            KEEP => {
                if let Some(b) = before {
                    self.set_remote_color(b, RemotePersist::No);
                }
            }
            CUSTOM => {
                if let Some(b) = before {
                    self.set_remote_color(b, RemotePersist::No);
                }
                let start = before.map(to_hex).unwrap_or_default();
                self.prompt.ask(
                    "colour: a hex code like 3b82f6",
                    &start,
                    Pending::RemoteColor,
                );
            }
            AUTO => {
                if let Some(c) = self.remote_color_of_row(AUTO) {
                    self.set_remote_color(c, RemotePersist::Forget);
                }
            }
            name => {
                if let Some(c) = named(name) {
                    self.set_remote_color(c, RemotePersist::Save);
                }
            }
        }
    }

    /// The hex prompt's answer: a code, or a notice saying what a code is.
    pub(super) fn remote_color_answer(&mut self, text: Option<String>) {
        let Some(text) = text else { return };
        match parse_hex(&text) {
            Some(c) => self.set_remote_color(c, RemotePersist::Save),
            None => self.notify("not a colour: six hex digits, like 3b82f6"),
        }
    }

    fn set_remote_color(&mut self, color: Rgb, persist: RemotePersist) {
        if let Some(r) = &mut self.remote {
            r.color = color;
            self.effects.push(Effect::RemoteColor { color, persist });
        }
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
        "remote.color",
        "Remote: change the title bar and Dock colour…",
        |m| {
            if m.remote.is_none() {
                m.notify("this is not a remote instance (ift connect opens one)");
                return;
            }
            m.remote_color_before_preview = m.remote_color();
            m.open_palette(Source::RemoteColor);
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
        m.remote_color_before_preview = m.remote_color();
        m.open_palette(Source::RemoteColor);
    }

    fn colour(m: &Model) -> Rgb {
        m.remote.as_ref().unwrap().color
    }

    #[test]
    fn only_a_remote_instance_has_the_command() {
        let mut m = Model::new();
        let mut r = crate::commands::CommandRegistry::new(|_| {});
        register(&mut r);
        assert!(r.run("remote.color", &mut m));
        assert!(!m.palette_open());
        assert!(m
            .notice
            .as_deref()
            .unwrap_or("")
            .contains("not a remote instance"));
    }

    #[test]
    fn the_picker_lists_the_current_colour_first_then_the_default_the_names_and_custom() {
        let mut m = remote_model();
        open(&mut m);
        let items = m.palette_items(Source::RemoteColor, &[]);
        assert_eq!(items[0].id, "keep");
        assert_eq!(items[0].hint.as_deref(), Some("active"));
        assert_eq!(
            items[0].label, "Current: ff8800",
            "an unnamed colour shows as hex"
        );
        assert_eq!(items[1].id, "auto");
        assert_eq!(items.last().unwrap().id, "custom");
        let blue = items.iter().find(|i| i.id == "blue").unwrap();
        assert_eq!(
            (blue.label.as_str(), blue.hint.as_deref()),
            ("Blue", Some("#3b82f6"))
        );
        assert_eq!(items.len(), NAMED.len() + 3);
        assert!(Source::RemoteColor.keeps_its_order());
    }

    #[test]
    fn moving_the_highlight_previews_and_escape_puts_the_colour_back() {
        let mut m = remote_model();
        open(&mut m);
        m.effects.clear();
        m.palette_preview(Source::RemoteColor, Some("blue"));
        assert_eq!(colour(&m), named("blue").unwrap());
        assert!(
            m.effects.iter().any(|e| matches!(
                e,
                Effect::RemoteColor {
                    persist: RemotePersist::No,
                    ..
                }
            )),
            "the Dock follows, nothing is saved"
        );
        m.palette_preview(Source::RemoteColor, Some("custom"));
        assert_eq!(
            colour(&m),
            (0xff, 0x88, 0x00),
            "the custom row shows what you have"
        );
        m.palette_preview(Source::RemoteColor, Some("rose"));
        m.close_palette(false);
        assert_eq!(
            colour(&m),
            (0xff, 0x88, 0x00),
            "dismissed: back to the start"
        );
        assert!(m.remote_color_before_preview.is_none());
    }

    #[test]
    fn a_named_colour_is_kept_and_saved() {
        let mut m = remote_model();
        open(&mut m);
        m.effects.clear();
        m.palette_preview(Source::RemoteColor, Some("teal"));
        m.palette_run(Source::RemoteColor, "teal");
        m.close_palette(true);
        assert_eq!(colour(&m), named("teal").unwrap());
        assert!(m.effects.iter().any(|e| matches!(e, Effect::RemoteColor { persist: RemotePersist::Save, color } if *color == named("teal").unwrap())));
    }

    #[test]
    fn the_default_row_goes_back_to_the_hashed_colour_and_forgets_the_choice() {
        let mut m = remote_model();
        open(&mut m);
        m.effects.clear();
        m.palette_run(Source::RemoteColor, "auto");
        assert_eq!(colour(&m), color_for("ops@box"));
        assert!(m.effects.iter().any(|e| matches!(
            e,
            Effect::RemoteColor {
                persist: RemotePersist::Forget,
                ..
            }
        )));
    }

    #[test]
    fn keeping_the_current_colour_changes_nothing() {
        let mut m = remote_model();
        open(&mut m);
        m.palette_preview(Source::RemoteColor, Some("pink"));
        m.palette_run(Source::RemoteColor, "keep");
        assert_eq!(colour(&m), (0xff, 0x88, 0x00));
    }

    #[test]
    fn the_custom_row_asks_for_a_hex_code_and_applies_a_good_one() {
        let mut m = remote_model();
        open(&mut m);
        m.palette_run(Source::RemoteColor, "custom");
        assert!(m.prompt.is_open(), "the second step is a prompt");
        assert_eq!(colour(&m), (0xff, 0x88, 0x00), "nothing changed yet");
        m.effects.clear();
        m.remote_color_answer(Some("#12ab34".into()));
        assert_eq!(colour(&m), (0x12, 0xab, 0x34));
        assert!(m.effects.iter().any(|e| matches!(
            e,
            Effect::RemoteColor {
                persist: RemotePersist::Save,
                ..
            }
        )));
        // A bad code says what a code is and keeps the colour.
        m.notice = None;
        m.remote_color_answer(Some("blue-ish".into()));
        assert_eq!(colour(&m), (0x12, 0xab, 0x34));
        assert!(m.notice.as_deref().unwrap_or("").contains("six hex digits"));
        // Escape on the prompt (no answer) changes nothing.
        m.notice = None;
        m.remote_color_answer(None);
        assert!(m.notice.is_none());
    }
}
