//! Startup and the per-frame plumbing: the config directory read and its
//! defaults rewritten, themes seeded and listed, the layout loaded, the
//! backend started; then, every frame, the backend's channels drained into
//! the model, the effects performed, the layout saved on a debounce, the
//! staleness sweep. Port of the async parts of `App.svelte`,
//! `settings.svelte.ts`, `theme.svelte.ts` and `persistence.svelte.ts`.
use crate::{now_ms, AppView, Glide, SAVE_DEBOUNCE_MS};
use gpui::FocusHandle;
use infiniterm_core::app::Backend;
use infiniterm_core::backend::SessionBackend;
use infiniterm_core::commands::CommandRegistry;
use infiniterm_core::config_files::{config_migrate, config_read, config_write, ConfigFile};
use infiniterm_core::grid::Size;
use infiniterm_core::itermcolors::{is_complete_theme, parse_iterm_colors};
use infiniterm_core::keymap::{default_keymap, render_keybindings_default};
use infiniterm_core::model::register::{describe_context, register_commands, run_with_effects};
use infiniterm_core::model::settings_in::{EMPTY_KEYBINDINGS, EMPTY_SETTINGS};
use infiniterm_core::model::Effect;
use infiniterm_core::paths::{home_dir, socket_path};
use infiniterm_core::settings_doc::default_settings_text;
use infiniterm_core::themes_files::{ensure_themes_dir, list_themes, read_theme};
use std::path::PathBuf;

/// Every five seconds: frequent enough for a sixty-second window.
const SWEEP_MS: f64 = 5000.;

impl AppView {
    pub fn new(focus: FocusHandle, scale_factor: f32) -> AppView {
        let mut registry = CommandRegistry::new(|line| eprintln!("[infiniterm] {line}"));
        register_commands(&mut registry);
        registry.set_context(describe_context);
        AppView {
            model: infiniterm_core::model::Model::new(),
            registry,
            backend: Backend::start(&socket_path()),
            animator: crate::Animator::new(),
            chrome: crate::Chrome::default_chrome(),
            bodies: Default::default(),
            focus,
            pan: None,
            gesture: None,
            prompt_field: Default::default(),
            query_field: Default::default(),
            shortcuts_field: Default::default(),
            prompt_was_open: false,
            samples: vec![],
            mouse: infiniterm_core::grid::Point { x: 0., y: 0. },
            seeded: false,
            save_due: None,
            last_sweep: now_ms(),
            glides: Default::default(),
            marked: Default::default(),
            frames: 0,
            fps_window: std::time::Instant::now(),
            fps: 0.,
            themes_dir: PathBuf::new(),
            scale_factor,
        }
    }

    pub fn run_command(&mut self, id: &str) {
        run_with_effects(&mut self.model, &self.registry, id);
        self.perform_effects();
    }

    /// Something to draw a theme's colours from. The app bundle's resources
    /// when running from one; the reference checkout beside this repo while
    /// developing.
    fn bundled_themes() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let in_bundle = exe.parent()?.parent()?.join("Resources").join("themes");
        if in_bundle.is_dir() {
            return Some(in_bundle);
        }
        let dev = home_dir().join("Code/infiniterm/src-tauri/resources/themes");
        dev.is_dir().then_some(dev)
    }

    fn load_theme(&mut self, name: &str) {
        match read_theme(&self.themes_dir, name) {
            Ok(xml) => {
                let theme = parse_iterm_colors(&xml);
                if !is_complete_theme(&theme) {
                    eprintln!("[infiniterm] theme \"{name}\" is missing colours; ignoring");
                    return;
                }
                self.chrome.apply_theme(&theme);
                self.model.theme_current = Some(name.to_string());
                // With hundreds of schemes installed, the name is the only way
                // to know which one you just landed on.
                self.model.notify(name);
            }
            Err(e) => eprintln!("[infiniterm] could not load theme \"{name}\": {e}"),
        }
    }

    fn refresh_themes(&mut self) {
        self.model.theme_names = list_themes(&self.themes_dir);
    }

    /// The one-shot work the model queued. `RunCommand` never reaches here:
    /// `run_with_effects` consumes it.
    pub fn perform_effects(&mut self) {
        let now = now_ms();
        for effect in self.model.take_effects() {
            match effect {
                Effect::AnimatePan { x, y } => self.animator.pan(
                    &mut self.model.viewport,
                    infiniterm_core::grid::Point { x, y },
                    now,
                ),
                Effect::AnimateZoom {
                    scale,
                    anchor_world,
                    anchor_screen,
                } => self.animator.zoom(
                    &mut self.model.viewport,
                    scale,
                    anchor_world,
                    anchor_screen,
                    now,
                ),
                Effect::AnimateFit(to) => self.animator.fit(&mut self.model.viewport, to, now),
                Effect::CancelAnimation => self.animator.cancel(),
                Effect::KillPane(pane) => self.backend.pty.kill(pane),
                Effect::KillAllPanes => self.backend.pty.kill_all(),
                Effect::ClearPane(_) => {} // the terminal body, Phase 4
                Effect::WritePane(pane, bytes) => self.backend.pty.write(pane, &bytes),
                Effect::DraftDelete(id) => {
                    let _ = infiniterm_core::files::draft_delete(&id);
                }
                Effect::MarkSwap(rects) => {
                    for (id, from) in rects {
                        self.marked.insert(id, from);
                    }
                }
                Effect::OpenUrl(url) => {
                    if let Err(e) = infiniterm_core::links_fs::open_url(&url) {
                        self.model.notify(format!("could not open {url}"));
                        eprintln!("[infiniterm] {e}");
                    }
                }
                Effect::SaveSetting { path, value } => {
                    let existing = config_read(ConfigFile::Settings);
                    if let Some(patched) = infiniterm_core::model::Model::patched_settings(
                        existing.as_deref(),
                        &path,
                        &value,
                    ) {
                        if let Err(e) = config_write(ConfigFile::Settings, &patched) {
                            eprintln!("[infiniterm] could not write settings: {e}");
                        }
                    }
                }
                Effect::LoadTheme(name) => self.load_theme(&name),
                Effect::RefreshThemes => self.refresh_themes(),
                Effect::Editor { .. } => {} // the editor body, Phase 6
                Effect::Log(line) => eprintln!("[infiniterm] {line}"),
                Effect::Warn(line) => eprintln!("[infiniterm/warn] {line}"),
                Effect::Reload => {
                    self.flush_save();
                    std::process::exit(0);
                }
                Effect::RunCommand(id) => self.run_command(&id),
            }
        }
    }

    /// Drains the backend's channels into the model, once per frame.
    pub fn drain_backend(&mut self) {
        while let Ok((pane, event)) = self.backend.pane_events.try_recv() {
            self.model.apply_pane_event(pane, &event);
        }
        while let Ok(report) = self.backend.hook_reports.try_recv() {
            self.model.apply_hook(&report);
        }
        while let Ok(statuses) = self.backend.pane_status.try_recv() {
            self.model.apply_pane_statuses(&statuses);
        }
        while let Ok(change) = self.backend.config_changes.try_recv() {
            match change.file {
                ConfigFile::Settings => {
                    self.model.apply_settings_text(&change.contents);
                    self.animator.animations_on = self.model.config.ui.animations;
                }
                _ => self.model.apply_keymap_text(&change.contents),
            }
        }
        while let Ok(req) = self.backend.cli_requests.try_recv() {
            let ids: Vec<String> = self.registry.all().iter().map(|c| c.id.clone()).collect();
            let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
            let reply = self.model.run_ift(&req, &ids);
            self.backend.cli.reply(req.id, reply.ok, reply.text);
        }
        self.perform_effects();
    }

    /// A MarkSwap'd card whose rect changed since the mark starts a glide.
    pub fn start_glides(&mut self, now: f64) {
        let marked = std::mem::take(&mut self.marked);
        for (id, from) in marked {
            if let Some(c) = self.model.card(&id) {
                if c.rect != from {
                    if std::env::var_os("INFINITERM_KEYLOG").is_some() {
                        eprintln!("[glide] {} from {:?} to {:?}", &id[..8], from, c.rect);
                    }
                    self.glides.insert(id, Glide { from, started: now });
                }
            }
        }
        self.glides.retain(|_, g| now - g.started < crate::SWAP_MS);
    }

    /// The saved layout, 500 ms after the last change: a drag is one write,
    /// not sixty. Nothing before `loaded`, nothing while `read_only`.
    pub fn schedule_save(&mut self, now: f64) {
        if self.model.dirty_layout && self.save_due.is_none() {
            self.save_due = Some(now + SAVE_DEBOUNCE_MS);
        }
        if self.save_due.is_some_and(|due| now >= due) {
            self.save_due = None;
            self.write_layout();
        }
    }

    fn write_layout(&mut self) {
        if let Some(text) = self.model.save_text() {
            if let Err(e) = infiniterm_core::layout_file::write_layout(&text) {
                eprintln!("[infiniterm] could not save the layout: {e}");
            }
        }
    }

    /// Immediately, for a quit: anything done in the last half second would
    /// otherwise be lost.
    pub fn flush_save(&mut self) {
        self.save_due = None;
        self.write_layout();
    }

    pub fn maybe_sweep(&mut self, now: f64) {
        if now - self.last_sweep >= SWEEP_MS {
            self.last_sweep = now;
            self.model.sweep_stale();
        }
    }
}

/// Reads the config directory, rewrites the generated files, seeds and lists
/// the themes, loads the saved canvas. Order matters: the settings decide
/// the start directory and the theme before the first card is seeded.
pub fn startup(app: &mut AppView) {
    let now = now_ms();
    app.model.tick(now);
    app.model.home = home_dir().to_string_lossy().into_owned();
    app.model.start_dir = app.model.home.clone();
    app.model.view_size = Size { w: 0., h: 0. };

    if let Some(moved) = config_migrate() {
        eprintln!(
            "[infiniterm] moved your old ~/.config/infiniterm.json to {}",
            moved.display()
        );
    }
    // Always rewritten, which is what makes them read-only in practice; also
    // how a new setting reaches an existing install.
    let labels: Vec<(String, String)> = app
        .registry
        .all()
        .iter()
        .map(|c| (c.id.clone(), c.label.clone()))
        .collect();
    let label_for = |id: &str| labels.iter().find(|(i, _)| i == id).map(|(_, l)| l.clone());
    let _ = config_write(ConfigFile::SettingsDefault, &default_settings_text());
    let _ = config_write(
        ConfigFile::KeybindingsDefault,
        &render_keybindings_default(&default_keymap(), label_for),
    );
    let settings = config_read(ConfigFile::Settings).unwrap_or_else(|| {
        let _ = config_write(ConfigFile::Settings, EMPTY_SETTINGS);
        EMPTY_SETTINGS.to_string()
    });
    let keys = config_read(ConfigFile::Keybindings).unwrap_or_else(|| {
        let _ = config_write(ConfigFile::Keybindings, EMPTY_KEYBINDINGS);
        EMPTY_KEYBINDINGS.to_string()
    });
    app.themes_dir = ensure_themes_dir(AppView::bundled_themes().as_deref());
    app.refresh_themes();
    app.model.apply_settings_text(&settings);
    app.model.apply_keymap_text(&keys);
    app.animator.animations_on = app.model.config.ui.animations;
    app.model
        .load_layout(infiniterm_core::layout_file::read_layout().as_deref());
    // Drafts belong to cards; one whose card is gone is a leak, not a backup.
    if !app.model.read_only {
        let keep: Vec<String> = app.model.cards.iter().map(|c| c.id.clone()).collect();
        infiniterm_core::files::draft_prune(&keep);
    }
    app.perform_effects();
    if !app.backend.socket_ok {
        app.model
            .notify("could not bind the socket: hooks and ift are off");
    }
}
