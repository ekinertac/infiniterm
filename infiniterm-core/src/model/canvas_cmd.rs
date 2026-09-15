//! The canvas and the app around it: zoom and fit, interface scale, the
//! palette, the shortcuts panel, the settings pair, the theme picker,
//! reload. Port of `commands/canvas.ts`.
use super::palette_state::Source;
use super::{ConfigPair, Effect, Model};
use crate::chrome::{clamp_ui_scale, UI_SCALE_STEP};
use crate::config_files::{config_path, ConfigFile};
use crate::grid::Point;
use crate::ift::{open_plan, PathKind};
use crate::saved_layout::CardKind;
use crate::viewport::{bounding_rect, centre_on, fit_rect, viewport_centre, MAX_SCALE, MIN_SCALE};

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
            self.apply_viewport(fit_rect(bounds, self.view_size));
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
    // The selection's bounding box, which for one card is the card.
    r.register("canvas.zoom.fitCard", "Canvas: fit the selection", |m| {
        let rects: Vec<_> = m.selected().iter().map(|c| c.rect).collect();
        if let Some(bounds) = bounding_rect(&rects) {
            m.frame_card(bounds);
        }
    });
    // The cards on THIS canvas: fitting every workspace's cards framed the
    // union of canvases you could not see.
    r.register("canvas.zoom.fitAll", "Canvas: fit all cards", |m| {
        let rects: Vec<_> = m.here().iter().map(|c| c.rect).collect();
        if let Some(bounds) = bounding_rect(&rects) {
            let next = fit_rect(bounds, m.view_size);
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
    // A reload kills every shell and its chord is a reflex from browsers; it
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
