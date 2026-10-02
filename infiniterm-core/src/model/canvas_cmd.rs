//! The canvas and the app around it: zoom and fit, interface scale, the
//! palette, the shortcuts panel, the settings pair, the theme picker,
//! reload. Port of `commands/canvas.ts`.
use super::palette_state::Source;
use super::{ConfigPair, Effect, Model};
use crate::chrome::{clamp_ui_scale, UI_SCALE_STEP};
use crate::config::TerminalBackend;
use crate::config_files::{config_path, ConfigFile};
use crate::grid::Point;
use crate::ift::{open_plan, PathKind};
use crate::saved_layout::CardKind;
use crate::viewport::{bounding_rect, centre_on, viewport_centre, MAX_SCALE, MIN_SCALE};

impl Model {
    /// Keyboard zoom, pinned to the active card at the MIDDLE of the view.
    /// The anchor is deliberately constant: pinning a visible card where it
    /// sat and centring an off-screen one made the same key behave two ways.
    /// Chains from the pending target so a held key ramps evenly.
    pub fn zoom_by(&mut self, factor: f64) {
        let anchor_world = match self.focused() {
            Some(c) => Point {
                x: c.rect.x + c.rect.w / 2.,
                y: c.rect.y + c.rect.h / 2.,
            },
            None => viewport_centre(self.viewport, self.view_size),
        };
        let anchor_screen = Point {
            x: self.view_size.w / 2.,
            y: self.view_size.h / 2.,
        };
        let from = self.pending_scale.unwrap_or(self.viewport.scale);
        let scale = (from * factor).clamp(MIN_SCALE, MAX_SCALE);
        self.effects.push(Effect::AnimateZoom {
            scale,
            anchor_world,
            anchor_screen,
        });
    }

    /// The size of the app's own furniture, independent of canvas zoom.
    /// Announced, because most of what it changes is a border width you are
    /// not looking at directly.
    pub fn set_ui_scale(&mut self, value: f64) {
        self.ui_scale = clamp_ui_scale(value);
        self.dirty_layout = true;
        self.notify(format!("interface {}%", (self.ui_scale * 100.).round()));
    }

    /// The config files open in the app's own editors, as a pair: the
    /// generated defaults on the left, read-only, and your overrides on the
    /// right. A file already open on this workspace is reused. Remembered as
    /// a pair so one close closes both and lands back where you were.
    pub fn open_config_pair(&mut self, keybindings: bool) {
        let (defaults_file, user_file) = if keybindings {
            (ConfigFile::KeybindingsDefault, ConfigFile::Keybindings)
        } else {
            (ConfigFile::SettingsDefault, ConfigFile::Settings)
        };
        let defaults_path = config_path(defaults_file).to_string_lossy().into_owned();
        let user_path = config_path(user_file).to_string_lossy().into_owned();
        let from = self.focused().cloned();
        let return_to = from
            .as_ref()
            .filter(|c| c.kind == CardKind::Terminal)
            .map(|c| c.id.clone());
        let open = |m: &mut Model, path: &str, after: Option<&str>| -> Option<String> {
            let existing = m
                .here()
                .into_iter()
                .find(|c| c.kind == CardKind::Editor && c.path.as_deref() == Some(path))
                .map(|c| c.id.clone());
            existing.or_else(|| m.open_in_card(open_plan(path, PathKind::File, None), after))
        };
        let defaults = open(self, &defaults_path, from.as_ref().map(|c| c.id.as_str()));
        let Some(user) = open(
            self,
            &user_path,
            defaults.as_deref().or(from.as_ref().map(|c| c.id.as_str())),
        ) else {
            return;
        };
        if !self.config_pairs.iter().any(|p| p.ids.contains(&user)) {
            let ids = defaults
                .iter()
                .chain(std::iter::once(&user))
                .cloned()
                .collect();
            self.config_pairs.push(ConfigPair { ids, return_to });
        }
        // Focus set through the extend path, so the extras (the defaults
        // card) survive the focus landing on the user's file.
        let extra = defaults.iter().filter(|d| **d != user).cloned().collect();
        self.focus_extended(&user, extra);
        let rects: Vec<_> = defaults
            .iter()
            .chain(std::iter::once(&user))
            .filter_map(|id| self.card(id))
            .map(|c| c.rect)
            .collect();
        if let Some(bounds) = bounding_rect(&rects) {
            self.apply_viewport(self.fit_viewport(bounds));
        }
    }

    /// Steps through the installed schemes, so trying them is one keystroke.
    pub fn cycle_theme(&mut self, delta: isize) {
        self.effects.push(Effect::RefreshThemes);
        if self.theme_names.is_empty() {
            self.effects.push(Effect::Warn(
                "no .itermcolors files in the themes directory".into(),
            ));
            return;
        }
        let n = self.theme_names.len() as isize;
        let i = self
            .theme_current
            .as_ref()
            .and_then(|c| self.theme_names.iter().position(|n| n == c))
            .map_or(-1, |i| i as isize);
        let next = ((i + delta) % n + n) % n;
        self.effects
            .push(Effect::LoadTheme(self.theme_names[next as usize].clone()));
    }
}

pub fn register(r: &mut crate::commands::CommandRegistry<Model>) {
    // Centre on the active card if there is one, otherwise hold the point
    // you are already looking at; never reset to the origin.
    r.register("canvas.zoom.actual", "Canvas: actual size (100%)", |m| {
        let point = match m.focused() {
            Some(c) => Point {
                x: c.rect.x + c.rect.w / 2.,
                y: c.rect.y + c.rect.h / 2.,
            },
            None => viewport_centre(m.viewport, m.view_size),
        };
        let next = centre_on(point, 1., m.view_size);
        m.apply_viewport(next);
    });
    r.register("canvas.zoom.in", "Canvas: zoom in", |m| m.zoom_by(1.2));
    r.register("canvas.zoom.out", "Canvas: zoom out", |m| {
        m.zoom_by(1. / 1.2)
    });
    // The selection's SLOT: for one card the card, and for a card that has
    // been split the whole space those halves share. Framing half a slot
    // put the other half off screen, and a split is not somewhere else.
    r.register("canvas.zoom.fitCard", "Canvas: fit the selection", |m| {
        let selected = m.selected();
        let rects: Vec<_> = selected.iter().map(|c| c.rect).collect();
        let groups: Vec<_> = selected.iter().map(|c| c.soft_group_id.clone()).collect();
        if let Some(bounds) = m.slot_bounds(&rects, &groups) {
            m.frame_card(bounds);
        }
    });
    // The cards on THIS canvas: fitting every workspace's cards framed the
    // union of canvases you could not see.
    r.register("canvas.zoom.fitAll", "Canvas: fit all cards", |m| {
        let rects: Vec<_> = m.here().iter().map(|c| c.rect).collect();
        if let Some(bounds) = bounding_rect(&rects) {
            let next = m.fit_viewport(bounds);
            m.apply_viewport(next);
        }
    });
    // Cards dragged about by hand, back into the block new cards fill, in
    // the order you read them now, each keeping its size; then fit all so
    // you see the result. Cmd+Z puts them back.
    // Groups and split clusters move as units, their inside arrangement
    // kept (`layout::tidy_units`); loose cards each on their own.
    r.register("canvas.tidy", "Canvas: tidy the cards into a block", |m| {
        let here: Vec<crate::model::Card> = m.here().into_iter().cloned().collect();
        if here.len() < 2 {
            return;
        }
        // A card's unit: its group, else its split cluster, else itself.
        let mut keys: Vec<String> = vec![];
        let mut pad: Vec<f64> = vec![];
        let unit: Vec<usize> = here
            .iter()
            .map(|c| {
                let (key, p) = match (&c.group_id, &c.soft_group_id) {
                    (Some(g), _) => (format!("g:{g}"), crate::groups::GROUP_PAD),
                    (None, Some(s)) => (format!("s:{s}"), 0.),
                    _ => (format!("c:{}", c.id), 0.),
                };
                match keys.iter().position(|k| *k == key) {
                    Some(u) => u,
                    None => {
                        keys.push(key);
                        pad.push(p);
                        keys.len() - 1
                    }
                }
            })
            .collect();
        let rects: Vec<_> = here.iter().map(|c| c.rect).collect();
        let grid = (
            Point {
                x: crate::grid::HALF_CELL,
                y: crate::grid::HALF_CELL,
            },
            m.default_size(),
        );
        let placed = crate::layout::tidy_units(&rects, &unit, &pad, crate::cards::GUTTER, grid);
        m.remember_layout();
        let ids: Vec<String> = here.iter().map(|c| c.id.clone()).collect();
        m.mark_swap(&ids);
        for (c, rect) in here.iter().zip(placed) {
            if let Some(card) = m.card_mut(&c.id) {
                card.rect = rect;
            }
        }
        m.dirty_layout = true;
        let all: Vec<_> = m.here().iter().map(|c| c.rect).collect();
        if let Some(bounds) = bounding_rect(&all) {
            let next = m.fit_viewport(bounds);
            m.apply_viewport(next);
        }
    });
    // Toggle: the key that opened it is the obvious one to press to dismiss it.
    r.register("app.palette", "Run a command", |m| {
        if m.palette_open() {
            m.close_palette(false);
        } else {
            m.open_palette(Source::Commands);
        }
    });
    // Our own chord for the emoji panel, through the same physical-key
    // path as every other binding. macOS handles Cmd+Ctrl+Space system-wide
    // too, and a gpui action bound to it as well, and which of the three
    // won depended on the focus state of the moment: it worked sometimes.
    // One owner now.
    r.register("app.emoji", "App: emoji & symbols", |m| {
        m.effects.push(Effect::ShowCharacterPalette);
    });
    r.register("app.shortcuts", "App: keyboard shortcuts", |m| {
        m.shortcuts_open = !m.shortcuts_open
    });
    r.register("ui.scale.up", "App: interface bigger", |m| {
        let v = m.ui_scale + UI_SCALE_STEP;
        m.set_ui_scale(v);
    });
    r.register("ui.scale.down", "App: interface smaller", |m| {
        let v = m.ui_scale - UI_SCALE_STEP;
        m.set_ui_scale(v);
    });
    r.register("ui.scale.reset", "App: interface at 100%", |m| {
        m.set_ui_scale(1.)
    });
    r.register(
        "app.settings",
        "App: open settings, beside the defaults",
        |m| m.open_config_pair(false),
    );
    r.register("help.docs", "Help: open the docs", |m| {
        let from = m.focused().map(|c| c.id.clone());
        if let Some(id) = m.open_docs(from.as_deref()) {
            m.set_focus(Some(&id));
        }
    });
    r.register("help.welcome", "Help: open the welcome card", |m| {
        let from = m.focused().map(|c| c.id.clone());
        if let Some(id) = m.open_welcome(from.as_deref()) {
            m.set_focus(Some(&id));
        }
    });
    r.register(
        "app.keybindings",
        "App: open keybindings, beside the defaults",
        |m| m.open_config_pair(true),
    );
    // The theme picker: the reason the palette exists. The theme in force is
    // captured HERE, before any preview runs, so cancel can put it back.
    r.register("theme.pick", "Theme: switch", |m| {
        m.theme_before_preview = m.theme_current.clone();
        m.effects.push(Effect::RefreshThemes);
        m.open_palette(Source::Themes);
    });
    r.register("theme.next", "Theme: next", |m| m.cycle_theme(1));
    r.register("theme.prev", "Theme: previous", |m| m.cycle_theme(-1));
    // Updates (update.rs, updater.rs): a check now, reported either way,
    // and the install, which is a restart through the swap. A restart
    // installs a staged update too; this is the name to find it by.
    r.register("app.update.check", "App: check for updates", |m| {
        m.effects.push(Effect::CheckForUpdate)
    });
    r.register(
        "app.update.install",
        "App: install the downloaded update and restart",
        |m| m.effects.push(Effect::InstallUpdate),
    );
    // The label says "keycast" because that is the word typed into the
    // palette to find it; "show pressed shortcuts" alone matched nothing
    // Ekin tried.
    r.register("app.fullscreen", "App: toggle full screen", |m| {
        m.effects.push(Effect::ToggleFullScreen)
    });
    r.register(
        "app.keycast",
        "App: keycast, show pressed shortcuts on screen",
        |m| m.effects.push(Effect::ToggleKeycast),
    );
    // A reload kills every shell and its chord is a reflex from browsers; it
    // Deliberately a four-key chord. It ends every process in every card,
    // and a restart you did not mean to ask for is expensive in a way that
    // no other binding here is.
    r.register("app.restart", "App: restart", |m| {
        // Under the daemon backend quitting DETACHES, so the shells and
        // anything running in them are still there afterwards and the cards
        // adopt them again. Under a local pty the same keystroke means
        // "kill everything I have open", which is not a thing to do on a
        // chord: say so instead of doing it.
        if m.config.terminal.backend != TerminalBackend::Daemon {
            m.notify("restart needs terminal.backend \"daemon\"; on pty it would kill every shell");
            return;
        }
        m.effects.push(Effect::Restart);
    });

    // exists only while the app is being developed.
    r.register(
        "app.reload",
        "App: reload the interface (development builds only)",
        |m| {
            if !m.dev_build {
                m.notify("reload is a development command; quit and reopen instead");
                return;
            }
            m.effects.push(Effect::KillAllPanes);
            m.effects.push(Effect::Reload);
        },
    );
}
