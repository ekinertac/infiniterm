//! The palette's open/closed state and its sources. Port of
//! `paletteState.svelte.ts`, with the four sources of the reference
//! (`commands`, `themes`, `placement`, `slotKind`) answered by the model
//! rather than registered as closures.
//!
//! A SOURCE is the extension point: the palette exists because 500 themes
//! cannot be reached one keystroke at a time, and themes need what commands
//! do not, a `preview` as the selection MOVES and a way to put the previous
//! one back on cancel. Use is recorded on RUN, never on preview.
use super::Model;
use crate::palette::PaletteItem;
use crate::palette_usage::{prune_usage, record_use, use_key, USAGE_LIMIT};
use crate::shortcuts::format_chord;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Commands,
    Themes,
    Placement,
    SlotKind,
}

impl Source {
    pub fn id(self) -> &'static str {
        match self {
            Source::Commands => "commands",
            Source::Themes => "themes",
            Source::Placement => "placement",
            Source::SlotKind => "slotKind",
        }
    }

    /// Whether this source's rows stay where they were designed to be.
    ///
    /// Recency earns its place in a list of hundreds: the five commands you
    /// actually run, or the theme you keep coming back to, at the top. A
    /// list of nine, or three, is a MENU, and a menu is muscle memory: Enter
    /// on a phantom, then Enter again, has to mean "terminal" every time,
    /// and Cmd+Shift+T then the second row has to be the same second row
    /// tomorrow. Ranking those by use moved the rows under the hand that had
    /// learned them.
    pub fn keeps_its_order(self) -> bool {
        matches!(self, Source::Placement | Source::SlotKind)
    }

    /// Prompt text in the empty input.
    pub fn placeholder(self) -> &'static str {
        match self {
            Source::Commands => "Run a command",
            Source::Themes => "Switch theme",
            Source::Placement => "New terminal…",
            Source::SlotKind => "New card in this slot…",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaletteState {
    pub source: Option<Source>,
    pub query: String,
    /// Index into the RANKED list, not the source's items.
    pub index: usize,
}

impl Model {
    /// Always resets the query: a palette that reopens on the last query
    /// looks broken the one time you wanted a fresh search.
    pub fn open_palette(&mut self, source: Source) {
        self.palette = PaletteState {
            source: Some(source),
            query: String::new(),
            index: 0,
        };
    }

    /// `chosen` false means dismissed, and the source's preview is put back.
    pub fn close_palette(&mut self, chosen: bool) {
        if !chosen {
            if let Some(source) = self.palette.source {
                self.palette_preview(source, None);
            }
        }
        self.palette = PaletteState::default();
    }

    pub fn palette_open(&self) -> bool {
        self.palette.source.is_some()
    }

    /// Anything drawn over the canvas that owns the pointer and the keys.
    /// The wheel asks because a scroll inside an overlay's own list still
    /// reaches the canvas element under it, and scrolled the card behind the
    /// pointer as well; the key path asks the three separately, because each
    /// of them handles keys differently.
    pub fn overlay_open(&self) -> bool {
        self.palette_open() || self.prompt.is_open() || self.shortcuts_open || self.omni.open
    }

    /// Recomputed every time it is asked for, so it is never stale.
    pub fn palette_items(
        &self,
        source: Source,
        command_labels: &[(&str, &str)],
    ) -> Vec<PaletteItem> {
        match source {
            // Every registered command, reachable without knowing a chord;
            // `app.palette` excluded, since opening the palette from inside it
            // can never be what you meant.
            Source::Commands => {
                let mut chords: Vec<(&str, String)> = vec![];
                for (chord, id) in &self.keymap {
                    if !chords.iter().any(|(i, _)| i == id) {
                        chords.push((id, format_chord(chord)));
                    }
                }
                command_labels
                    .iter()
                    .filter(|(id, _)| *id != "app.palette")
                    .map(|(id, label)| PaletteItem {
                        id: id.to_string(),
                        label: label.to_string(),
                        hint: chords.iter().find(|(i, _)| i == id).map(|(_, c)| c.clone()),
                    })
                    .collect()
            }
            // The theme in force is listed FIRST, so opening the picker
            // previews what you already have instead of whatever sorts first.
            Source::Themes => {
                let current = self
                    .theme_current
                    .clone()
                    .filter(|c| self.theme_names.contains(c));
                current
                    .iter()
                    .chain(
                        self.theme_names
                            .iter()
                            .filter(|n| Some(*n) != current.as_ref()),
                    )
                    .map(|name| PaletteItem {
                        id: name.clone(),
                        label: name.clone(),
                        hint: None,
                    })
                    .collect()
            }
            Source::Placement => {
                let chord = |id: &str| {
                    self.keymap
                        .iter()
                        .find(|(_, i)| i == id)
                        .map(|(c, _)| format_chord(c))
                };
                vec![
                    (
                        "slot",
                        "Terminal: in a slot you pick — every empty slot gets a letter",
                        Some("a … z".to_string()),
                    ),
                    (
                        "here",
                        "Terminal: beside the active card",
                        chord("card.new.terminal"),
                    ),
                    (
                        "loose",
                        "Terminal: beside the active card, outside its group",
                        chord("card.new.ungrouped"),
                    ),
                    ("file", "Editor: on a file you name", None),
                    ("untitled", "Editor: empty", chord("card.new.editor")),
                    ("diff", "Diff: this card against git HEAD", None),
                    ("browser", "Browser: open a URL", None),
                    ("claude", "Terminal: Claude Code in this directory", None),
                    ("pi", "Terminal: Pi in this directory", None),
                ]
                .into_iter()
                .map(|(id, label, hint)| PaletteItem {
                    id: id.into(),
                    label: label.into(),
                    hint,
                })
                .collect()
            }
            // Enter on a phantom: what goes in the slot. Terminal is the first
            // row, so Enter twice is what Enter once used to be.
            Source::SlotKind => [
                ("terminal", "Terminal"),
                ("editor", "Editor: empty"),
                ("browser", "Browser: open a URL"),
            ]
            .into_iter()
            .map(|(id, label)| PaletteItem {
                id: id.into(),
                label: label.into(),
                hint: None,
            })
            .collect(),
        }
    }

    /// Called as the highlighted item changes, and with `None` on dismissal.
    pub fn palette_preview(&mut self, source: Source, id: Option<&str>) {
        if source == Source::Themes {
            let target = id
                .map(String::from)
                .or_else(|| self.theme_before_preview.clone());
            // Nothing to put back on cancel if no theme was loaded to begin with.
            if let Some(name) = target {
                self.effects.push(super::Effect::LoadTheme(name));
            }
        }
    }

    /// Records a choice, capped so 521 themes cannot grow the map for the
    /// life of the install.
    pub fn note_use(&mut self, source: Source, id: &str) {
        // A menu that keeps its order has no use for a record of use, and
        // recording one would only be a map entry nothing reads.
        if source.keeps_its_order() {
            return;
        }
        self.usage = prune_usage(
            &record_use(&self.usage, &use_key(source.id(), id), self.now_ms),
            USAGE_LIMIT,
        );
        self.dirty_layout = true;
    }
}
