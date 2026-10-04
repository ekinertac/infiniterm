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
use crate::resize::Fraction;
use crate::shortcuts::format_chord;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Commands,
    Themes,
    Placement,
    SlotKind,
    /// `card.size` (Cmd+Alt+S): the card's size as fractions of the default.
    Sizes,
    /// `snippet.paste` (Cmd+Ctrl+S): the snippets file's names, then a row
    /// that opens the file.
    Snippets,
    /// `card.moveToWorkspace`: the other workspaces, then a new one.
    MoveTo,
    /// `window.color` (#157, #160): named colours previewed live, then a hex code.
    WindowColor,
}

/// Prefixes that tell a card row and a workspace row apart from a command
/// id in the Commands source. A command id never contains a colon.
pub const CARD_ROW: &str = "card:";
pub const WORKSPACE_ROW: &str = "workspace:";

/// The hint the theme picker puts beside the theme in force.
pub const ACTIVE_THEME_HINT: &str = "active";

impl Source {
    pub fn id(self) -> &'static str {
        match self {
            Source::Commands => "commands",
            Source::Themes => "themes",
            Source::Placement => "placement",
            Source::SlotKind => "slotKind",
            Source::Sizes => "sizes",
            Source::Snippets => "snippets",
            Source::MoveTo => "moveTo",
            Source::WindowColor => "windowColor",
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
    /// learned them. The themes keep theirs too (#72): the active one is
    /// pinned on top under its "active" hint, and recency only reshuffled
    /// the rest under it.
    pub fn keeps_its_order(self) -> bool {
        matches!(
            self,
            Source::Placement
                | Source::SlotKind
                | Source::Sizes
                | Source::MoveTo
                | Source::Themes
                | Source::WindowColor
        )
    }

    /// Prompt text in the empty input.
    pub fn placeholder(self) -> &'static str {
        match self {
            Source::Commands => "Run a command",
            Source::Themes => "Switch theme",
            Source::Placement => "New terminal…",
            Source::SlotKind => "New card in this slot…",
            Source::Sizes => "Resize the card to…",
            Source::Snippets => "Paste a snippet…",
            Source::MoveTo => "Move to workspace…",
            Source::WindowColor => "Window colour…",
        }
    }
}

/// The size picker's rows: id, label, width fraction, height fraction.
pub const SIZES: &[(&str, &str, Fraction, Fraction)] = &[
    (
        "full",
        "Full: the default size",
        Fraction::Full,
        Fraction::Full,
    ),
    (
        "half-wide",
        "Half: as a split to the right makes",
        Fraction::Half,
        Fraction::Full,
    ),
    (
        "half-tall",
        "Half: as a split down makes",
        Fraction::Full,
        Fraction::Half,
    ),
    ("quarter", "Quarter", Fraction::Half, Fraction::Half),
    (
        "double-wide",
        "Double wide: two cards side by side",
        Fraction::Double,
        Fraction::Full,
    ),
    (
        "double-tall",
        "Double tall: two cards stacked",
        Fraction::Full,
        Fraction::Double,
    ),
];

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
        self.palette_open()
            || self.prompt.is_open()
            || self.shortcuts_open
            || self.omni.open
            || self.switcher.is_some()
    }

    /// An overlay that owns the keyboard: while one is up no chord reaches
    /// the canvas behind it (input.rs `key_down`). The card switcher is left
    /// out: it is driven by Ctrl held down and handles its keys first.
    pub fn modal_open(&self) -> bool {
        self.overlay_open() && self.switcher.is_none()
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
                let mut items: Vec<PaletteItem> = command_labels
                    .iter()
                    .filter(|(id, _)| *id != "app.palette")
                    .map(|(id, label)| PaletteItem {
                        id: id.to_string(),
                        label: label.to_string(),
                        hint: chords.iter().find(|(i, _)| i == id).map(|(_, c)| c.clone()),
                    })
                    .collect();
                // Every card and every workspace, after the commands, so
                // the palette is also the way to a card by name. Live from
                // the model each time, never stale. A card in another
                // workspace says which, because choosing it switches there.
                let here = self.active_workspace.as_deref();
                for card in &self.cards {
                    let mut label = format!("Card: {}", self.numbered_label(card));
                    if Some(card.workspace_id.as_str()) != here {
                        if let Some(ws) = self.workspaces.iter().find(|w| w.id == card.workspace_id)
                        {
                            label.push_str(&format!(" \u{b7} {}", ws.name));
                        }
                    }
                    items.push(PaletteItem {
                        id: format!("{CARD_ROW}{}", card.id),
                        label,
                        hint: None,
                    });
                }
                for ws in &self.workspaces {
                    if Some(ws.id.as_str()) == here {
                        continue;
                    }
                    items.push(PaletteItem {
                        id: format!("{WORKSPACE_ROW}{}", ws.id),
                        label: format!("Workspace: {}", ws.name),
                        hint: None,
                    });
                }
                items
            }
            // The theme you had when the picker opened is listed FIRST, so
            // opening it previews what you already have instead of whatever
            // sorts first. Not the theme in force: the preview changes that
            // on every move, and listing it first re-sorted the rows under
            // the highlight (#67).
            Source::Themes => {
                let current = self
                    .theme_before_preview
                    .clone()
                    .or_else(|| self.theme_current.clone())
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
                        // Said, not only placed first (#72).
                        hint: (Some(name) == current.as_ref())
                            .then(|| ACTIVE_THEME_HINT.to_string()),
                    })
                    .collect()
            }
            Source::WindowColor => self.window_color_items(),
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
            // The workspaces in tab order, less the one you are on, each with
            // how many cards it holds, then a new one.
            Source::MoveTo => {
                let here = self.active_workspace.clone();
                let mut rows: Vec<PaletteItem> = self
                    .workspaces
                    .iter()
                    .filter(|w| Some(&w.id) != here.as_ref())
                    .map(|w| {
                        let n = self.cards_on(Some(&w.id)).len();
                        PaletteItem {
                            id: w.id.clone(),
                            label: w.name.clone(),
                            hint: Some(format!("{n} card{}", if n == 1 { "" } else { "s" })),
                        }
                    })
                    .collect();
                rows.push(PaletteItem {
                    id: super::workspaces_cmd::NEW_WORKSPACE_ROW.into(),
                    label: "New workspace".into(),
                    hint: None,
                });
                rows
            }
            // Sizes as fractions of the default card, from the card's own
            // corner. A menu, in a fixed order: the splits' shapes first.
            Source::Sizes => SIZES
                .iter()
                .map(|(id, label, _, _)| PaletteItem {
                    id: (*id).into(),
                    label: (*label).into(),
                    hint: None,
                })
                .collect(),
            // The file's names, the first line as the hint, and the way to
            // the file last. Recency applies: the snippet you paste daily
            // rises.
            Source::Snippets => self
                .snippets
                .iter()
                .map(|s| PaletteItem {
                    id: s.name.clone(),
                    label: s.name.clone(),
                    hint: Some(crate::snippets::first_line(&s.text).to_string()),
                })
                .chain(std::iter::once(PaletteItem {
                    id: crate::snippets::EDIT_ROW.into(),
                    label: "Edit snippets…".into(),
                    hint: Some("a folder, one file each".into()),
                }))
                .collect(),
        }
    }

    /// Called as the highlighted item changes, and with `None` on dismissal.
    pub fn palette_preview(&mut self, source: Source, id: Option<&str>) {
        if source == Source::WindowColor {
            self.window_color_preview(id);
            return;
        }
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
