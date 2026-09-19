//! Startup and the per-frame plumbing: the config directory read and its
//! defaults rewritten, themes seeded and listed, the layout loaded, the
//! backend started; then, every frame, the backend's channels drained into
//! the model, the effects performed, the layout saved on a debounce, the
//! staleness sweep. Port of the async parts of `App.svelte`,
//! `settings.svelte.ts`, `theme.svelte.ts` and `persistence.svelte.ts`.
use crate::{now_ms, AppView, Glide, SAVE_DEBOUNCE_MS};
use gpui::FocusHandle;
use infiniterm_core::app::Backend;
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
        // The settings are read here rather than in `startup`, which runs
        // later: the backend has to exist before the first card does, and
        // which backend it is cannot be changed afterwards.
        let (term_backend, session_buffer_mib) = startup_terminal();
        AppView {
            model: infiniterm_core::model::Model::new(),
            registry,
            backend: Backend::start(&socket_path(), term_backend, session_buffer_mib),
            animator: crate::Animator::new(),
            chrome: crate::Chrome::default_chrome(),
            bodies: Default::default(),
            decoys: Default::default(),
            focus,
            composing: None,
            reveal_on_release: None,
            label_hits: Vec::new(),
            show_character_palette: false,
            pan: None,
            gesture: None,
            body_drag: None,
            hover_body: None,
            context_menu: None,
            prompt_field: Default::default(),
            query_field: Default::default(),
            shortcuts_field: Default::default(),
            omni_field: crate::field::Field::default(),
            find_field: crate::field::Field::default(),
            redraw: false,
            last_paint_ms: 0.,
            clipboard_out: None,
            live_sessions: vec![],
            prompt_was_open: false,
            suggestions: std::sync::mpsc::channel(),
            samples: vec![],
            mouse: infiniterm_core::grid::Point { x: 0., y: 0. },
            seeded: false,
            save_due: None,
            history_due: None,
            last_sweep: now_ms(),
            glides: Default::default(),
            marked: Default::default(),
            body_sizes: Default::default(),
            frames: 0,
            fps_window: std::time::Instant::now(),
            timing: (0., 0., 0, 0.),
            window_seen: None,
            window_save_due: None,
            fps: 0.,
            themes_dir: PathBuf::new(),
            scale_factor,
            cef_running: false,
            reduce_motion: false,
            scheduler: Default::default(),
            ledger: Default::default(),
            palette: infiniterm_term::palette::Palette::default_palette(),
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
        let dev = home_dir().join("Code/infiniterm-tauri/src-tauri/resources/themes");
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
                self.refresh_palette();
                self.model.theme_current = Some(name.to_string());
                // With hundreds of schemes installed, the name is the only way
                // to know which one you just landed on.
                self.model.notify(name);
            }
            Err(e) => eprintln!("[infiniterm] could not load theme \"{name}\": {e}"),
        }
    }

    /// The terminal palette: the theme with the editor's selection colours,
    /// so terminals and editors select alike.
    pub fn refresh_palette(&mut self) {
        let hex3 = |s: &str| {
            crate::chrome::hex(s).map(|_| {
                let h = s.trim_start_matches('#');
                let v = u32::from_str_radix(h, 16).unwrap_or(0);
                [(v >> 16) as u8, (v >> 8) as u8, v as u8]
            })
        };
        self.palette = match &self.chrome.theme {
            Some(theme) => infiniterm_term::palette::Palette::from_theme(
                theme,
                hex3(&self.model.config.editor.selection_color),
                hex3(&self.model.config.editor.selection_text_color),
            ),
            None => infiniterm_term::palette::Palette::default_palette(),
        };
    }

    pub fn terminal_for_pane(
        &mut self,
        pane: infiniterm_core::backend::PaneId,
    ) -> Option<&mut crate::terminal_body::TerminalBody> {
        let Some(id) = self
            .model
            .cards
            .iter()
            .find(|c| c.pane_id == Some(pane))
            .map(|c| c.id.clone())
        else {
            return self.decoy_for_pane(pane);
        };
        self.bodies
            .get_mut(&id)?
            .as_any_mut()
            .downcast_mut::<crate::terminal_body::TerminalBody>()
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
                Effect::FetchSuggestions { query_id, query } => {
                    let tx = self.suggestions.0.clone();
                    // Off the UI thread: curl's worst case is the timeout,
                    // and a frame must never wait on a name lookup.
                    std::thread::spawn(move || {
                        let items = fetch_suggestions(&query);
                        let _ = tx.send((query_id, items));
                    });
                }
                Effect::KillPane(pane) => self.backend.pty.kill(pane),
                Effect::KillAllPanes => self.backend.pty.kill_all(),
                Effect::ClearPane(pane) => {
                    if let Some(body) = self.terminal_for_pane(pane) {
                        body.grid.clear();
                    }
                }
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
                Effect::Editor { card_id, action } => self.editor_effect(&card_id, action),
                Effect::Browser { card_id, action } => self.browser_effect(&card_id, action),
                Effect::Find { card_id, request } => self.find_effect(&card_id, request),
                // Held rather than written: perform_effects has no App to
                // write through, and threading one into sixteen call sites
                // for a clipboard is the wrong trade. The frame flushes it,
                // the same way a terminal's OSC 52 text is flushed.
                Effect::Copy(text) => self.clipboard_out = Some(text),
                Effect::Log(line) => eprintln!("[infiniterm] {line}"),
                Effect::Warn(line) => eprintln!("[infiniterm/warn] {line}"),
                Effect::AgentLog(line) => append_agent_log(&line),
                Effect::Reload => {
                    self.flush_save();
                    std::process::exit(0);
                }
                // Needs the Window, which effects do not have; the frame
                // makes the call. Same shape as the owed-frame flag.
                Effect::ShowCharacterPalette => self.show_character_palette = true,
                Effect::Restart => {
                    self.flush_save();
                    match relaunch_after_exit() {
                        // The waiter is running and holds our pid; nothing
                        // left to do but go, and it opens the bundle once
                        // we are gone. Cards come back to their sessions
                        // and the frame comes back from window.json.
                        Ok(()) => std::process::exit(0),
                        // Still here, so say why rather than quitting into
                        // nothing and looking like a crash.
                        Err(e) => self.model.notify(format!("could not restart: {e}")),
                    }
                }
                Effect::RunCommand(id) => self.run_command(&id),
                Effect::LogFps(n) => eprintln!("[infiniterm] stress zoom {n}: {:.0} fps", self.fps),
                Effect::LogDims => {
                    for (id, body) in self.bodies.iter_mut() {
                        if let Some(t) = body
                            .as_any_mut()
                            .downcast_mut::<crate::terminal_body::TerminalBody>()
                        {
                            eprintln!(
                                "[infiniterm] pane {:?} card {} grid {}x{} cell {:.2}x{:.2} font {} {}px",
                                t.pane,
                                &id[..id.len().min(8)],
                                t.cols(),
                                t.rows(),
                                t.cell_w,
                                t.font_px * t.line_height,
                                t.font_family,
                                t.font_px
                            );
                        }
                    }
                }
            }
        }
    }

    /// Whether anything is moving or arriving: an animation, a gesture, output
    /// waiting to be parsed, a body with unpainted output. When nothing is,
    /// no frame is requested and the window idles; the poll task wakes it.
    /// Which of `needs_frame`'s conditions holds, for the `[paint]` log line.
    pub fn frame_reason(&self) -> &'static str {
        let now = crate::now_ms();
        if self.animator.is_running() {
            "animator"
        } else if self.pan.is_some() {
            "pan"
        } else if self.gesture.is_some() {
            "gesture"
        } else if self.scheduler.pending() {
            "scheduler"
        } else if !self.glides.is_empty() {
            "glides"
        } else if self.bodies.values().any(|b| b.wants_frame(now))
            || self
                .decoys
                .values()
                .any(|d| crate::body::CardBody::wants_frame(d, now))
        {
            "body"
        } else if self.model.notice_expired(now) {
            "notice"
        } else {
            "idle"
        }
    }

    /// Whether the text is in the band where a frame is expensive (glyphs,
    /// not bars) and too small to be worth one per change: see
    /// `chrome::FAR_FONT_PX`.
    pub fn far(&self) -> bool {
        let font = self.model.config.terminal.font_size * self.model.viewport.scale;
        (crate::chrome::LEGIBLE_FONT_PX..crate::chrome::FAR_FONT_PX).contains(&font)
    }

    pub fn needs_frame(&self) -> bool {
        self.redraw
            || self.animator.is_running()
            || self.pan.is_some()
            || self.gesture.is_some()
            || !self.glides.is_empty()
            || {
                let now = crate::now_ms();
                // Content frames (output, blink) are gated when far: at
                // most one every FAR_REFRESH_MS. The output waits in the
                // scheduler meanwhile, under its usual backpressure.
                let content = self.scheduler.pending()
                    || self.bodies.values().any(|b| b.wants_frame(now))
                    || self
                        .decoys
                        .values()
                        .any(|d| crate::body::CardBody::wants_frame(d, now));
                let due = !self.far() || now - self.last_paint_ms >= crate::chrome::FAR_REFRESH_MS;
                (content && due) || self.model.notice_expired(now)
            }
    }

    /// Drains the backend's channels into the model, once per frame.
    pub fn drain_backend(&mut self) {
        while let Ok((pane, event)) = self.backend.pane_events.try_recv() {
            use infiniterm_core::backend::PaneEvent;
            match &event {
                PaneEvent::Output(bytes) => self.scheduler.enqueue(pane, bytes.clone()),
                // A replay is a known, finite bulk that arrives once, at
                // adopt, before any live output. Through the scheduler it
                // took a full canvas about 170 frames (11 cards, 4 MiB each,
                // 64 KiB a frame for four panes) and the history could be
                // WATCHED scrolling past on every launch. The budget exists
                // to bound a flood; this is not one. Fed whole, so it lands
                // in one frame, and acked whole, or the daemon's credit
                // stalls it. The body is always mapped by now: adopt sets
                // the pane and this runs on the same thread, after; the
                // fallback exists so a byte is never dropped, not because
                // it is expected to run.
                PaneEvent::Replay(bytes) => {
                    let fed = match self.terminal_for_pane(pane) {
                        Some(body) => {
                            body.feed_replay(bytes);
                            true
                        }
                        None => false,
                    };
                    if fed {
                        self.backend.pty.ack_now(pane, bytes.len());
                    } else {
                        self.scheduler.enqueue(pane, bytes.clone());
                    }
                }
                _ => {}
            }
            self.model.apply_pane_event(pane, &event);
        }
        while let Ok((query_id, items)) = self.suggestions.1.try_recv() {
            let before = self.model.omni.suggestions.clone();
            self.model.omni_suggestions(query_id, items);
            self.redraw |= self.model.omni.suggestions != before;
        }
        while let Ok(report) = self.backend.hook_reports.try_recv() {
            // Logged around the model, which is the only place that knows
            // what the state WAS: "the card went colourless and I do not
            // know which event did it" is not answerable from a screenshot.
            let before = self.model.card(&report.card_id).map(|c| c.agent);
            self.model.apply_hook(&report);
            let after = self.model.card(&report.card_id).map(|c| c.agent);
            log_agent_event(&report, before, after);
            self.redraw |= before != after;
        }
        while let Ok(statuses) = self.backend.pane_status.try_recv() {
            self.model.apply_pane_statuses(&statuses);
        }
        while let Ok(change) = self.backend.config_changes.try_recv() {
            match change.file {
                ConfigFile::Settings => {
                    self.model.apply_settings_text(&change.contents);
                    self.animator.animations_on =
                        self.model.config.ui.animations && !self.reduce_motion;
                }
                _ => self.model.apply_keymap_text(&change.contents),
            }
        }
        while let Ok(req) = self.backend.cli_requests.try_recv() {
            let ids: Vec<String> = self.registry.all().iter().map(|c| c.id.clone()).collect();
            let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
            let reply = self.model.run_ift(&req, &ids);
            self.backend.cli.reply(req.id, reply.ok, reply.text);
            // An ift verb arrives from the socket, after the element tree
            // for this frame was built, and several of them change chrome
            // rather than anything that draws itself: `ift name` sets a
            // card's title, `ift group` its group. Without owing a frame
            // the model changed and the screen did not, and the card kept
            // its old label until something unrelated repainted the canvas.
            // Claude Code renames its card at the END of a turn, which is
            // exactly when the card stops producing the output that would
            // have forced a frame, so it looked like the rename was refused.
            self.redraw = true;
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

    /// The omnibox's history, on the same debounce as the layout: a page
    /// that redirects three times is one write, not three.
    pub fn schedule_history(&mut self, now: f64) {
        if self.model.history.is_dirty() && self.history_due.is_none() {
            self.history_due = Some(now + SAVE_DEBOUNCE_MS);
        }
        if self.history_due.is_some_and(|due| now >= due) {
            self.history_due = None;
            self.model.history.clear_dirty();
            self.model
                .history
                .save(&infiniterm_core::paths::history_path());
        }
    }

    /// The frame changed: write it half a second after it stops changing.
    pub fn note_window(&mut self, bounds: gpui::WindowBounds, now: f64) {
        let state = crate::window_state::WindowState::of(bounds);
        if self.window_seen.as_ref() != Some(&state) {
            let first = self.window_seen.is_none();
            self.window_seen = Some(state);
            // The first frame is the restore itself, not a change.
            if !first {
                self.window_save_due = Some(now + SAVE_DEBOUNCE_MS);
            }
        }
    }

    pub fn schedule_window_save(&mut self, now: f64) {
        if self.window_save_due.is_some_and(|due| now >= due) {
            self.window_save_due = None;
            if let Some(state) = &self.window_seen {
                state.save();
            }
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
        if self.window_save_due.take().is_some() {
            if let Some(state) = &self.window_seen {
                state.save();
            }
        }
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
/// The backend (and its buffer size) the settings file asks for, read
/// straight from disk because the model does not exist yet. A malformed or
/// missing file means the defaults, which is what `merge_config` already
/// does for everything else.
fn startup_terminal() -> (infiniterm_core::config::TerminalBackend, usize) {
    let text = config_read(ConfigFile::Settings).unwrap_or_default();
    let value = infiniterm_core::jsonc::parse_jsonc(&text).unwrap_or(serde_json::Value::Null);
    let terminal = infiniterm_core::config::merge_config(&value).terminal;
    (terminal.backend, terminal.session_buffer as usize)
}

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
    app.animator.animations_on = app.model.config.ui.animations && !app.reduce_motion;
    // Before any card exists: which sessions the last launch left running.
    app.live_sessions = app.backend.pty.live_sessions();
    app.model
        .load_layout(infiniterm_core::layout_file::read_layout().as_deref());
    // The omnibox ranks against this; a missing file is an empty history.
    app.model.history =
        infiniterm_core::omni::history::History::load(&infiniterm_core::paths::history_path());
    // Sessions this app left running that no card came back for: a crash's
    // litter. Only when the canvas actually loaded, because a save file that
    // failed to parse claims nothing and would make every session an orphan.
    if app.model.loaded {
        let claimed: Vec<String> = app
            .model
            .cards
            .iter()
            .filter_map(|c| c.session.clone())
            .collect();
        app.backend.pty.kill_orphans(&claimed);
    }
    // Drafts belong to cards; one whose card is gone is a leak, not a backup.
    if !app.model.read_only {
        let keep: Vec<String> = app.model.cards.iter().map(|c| c.id.clone()).collect();
        infiniterm_core::files::draft_prune(&keep);
    }
    app.perform_effects();
    if !app.backend.socket_ok {
        app.model
            .notify("could not bind the socket: hooks and ift are off");
    } else if let Some(why) = app.backend.fell_back.clone() {
        app.model.notify(&why);
    }
}

/// One instance, as the reference's single-instance plugin enforced it:
/// the socket path is the lock. A second launch on the same data dir
/// activates the first through its bundle id and exits before it could
/// race the first for the save file.
pub fn another_instance_holds_the_socket() -> bool {
    let path = infiniterm_core::paths::socket_path();
    std::os::unix::net::UnixStream::connect(&path).is_ok()
}

/// curl, not an HTTP crate: one endpoint, off by default, and this app
/// already shells out to git, ps, lsof and open. `-s` keeps the progress
/// meter off stderr; `--max-time` is why a DNS stall cannot hold a frame.
fn suggest_args(query: &str) -> Vec<String> {
    vec![
        "-s".into(),
        "--max-time".into(),
        infiniterm_core::omni::suggest::SUGGEST_TIMEOUT_S.into(),
        infiniterm_core::omni::suggest::suggest_url(query),
    ]
}

/// Anything that goes wrong is no suggestions: the omnibox's local results
/// are already on screen and must not be disturbed by this.
fn fetch_suggestions(query: &str) -> Vec<String> {
    let Ok(out) = std::process::Command::new("curl")
        .args(suggest_args(query))
        .output()
    else {
        return vec![];
    };
    infiniterm_core::omni::suggest::parse_suggest(&String::from_utf8_lossy(&out.stdout))
}

/// Every hook event and what it did to the card's state, appended to
/// `agent.log` beside the save file. EVERY event, not only the ones that
/// changed something: a card that flickers is usually being told the same
/// thing repeatedly, and a log of changes alone hides that.
///
/// Truncated when it passes AGENT_LOG_MAX_BYTES at a write, so it cannot
/// grow without bound in a long-running app.
fn log_agent_event(
    report: &infiniterm_core::hooks::HookReport,
    before: Option<infiniterm_core::agent_state::AgentState>,
    after: Option<infiniterm_core::agent_state::AgentState>,
) {
    let (Some(before), Some(after)) = (before, after) else {
        return;
    };
    let short: String = report.card_id.chars().take(8).collect();
    let arrow = if before == after {
        format!("{:<7} (unchanged)", after.name())
    } else {
        format!("{:<7} -> {}", before.name(), after.name())
    };
    append_agent_log(&format!("{short}  {:<18} {arrow}", report.event));
}

/// One line, stamped, appended. Truncated when it passes the cap so it
/// cannot grow without bound in an app that runs for weeks.
fn append_agent_log(line: &str) {
    use std::io::Write;
    let path = infiniterm_core::paths::agent_log_path();
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > AGENT_LOG_MAX_BYTES) {
        let _ = std::fs::remove_file(&path);
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let _ = writeln!(
        file,
        "{}  {line}",
        infiniterm_core::transcript::clock_ms(crate::now_ms())
    );
}

/// Enough for days of turns, small enough to open in an editor.
const AGENT_LOG_MAX_BYTES: u64 = 512 * 1024;

/// The `.app` an executable lives in: `.../x.app/Contents/MacOS/x` is three
/// ancestors up. `None` for a bare binary, which is what `cargo run` and
/// every test is, and which must not be "restarted" into anything.
fn bundle_of(exe: &std::path::Path) -> Option<&std::path::Path> {
    exe.ancestors()
        .nth(3)
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
}

/// Arranges for this app bundle to be reopened once this process is gone.
///
/// Nothing inside a process can outlive it, so the waiting is done by a
/// detached `/bin/sh` that polls our pid and then runs `open`. `open` and
/// not `exec`: the relauncher must not become the app, or the new instance
/// inherits a shell's environment and every card claims to be running under
/// whatever started it (the trap `local_pty::terminal_identity` exists for).
///
/// The wait is a poll rather than a signal because the child cannot wait on
/// a process that is not its own. 100 ms is below noticing and costs a few
/// wakeups at most: a quit takes well under a second.
fn relaunch_after_exit() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundle = bundle_of(&exe).ok_or("not running from an .app bundle")?;
    let script = format!(
        "while kill -0 {pid} 2>/dev/null; do sleep 0.1; done; exec open {bundle}",
        pid = std::process::id(),
        bundle = infiniterm_core::drop::shell_quote(&bundle.to_string_lossy()),
    );
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Three up and it must be the bundle, or `open` gets handed a path
    // that is not an app and the restart quits into nothing.
    #[test]
    fn the_bundle_is_three_above_the_executable() {
        use std::path::Path;
        assert_eq!(
            bundle_of(Path::new(
                "/Applications/infiniterm.app/Contents/MacOS/infiniterm"
            )),
            Some(Path::new("/Applications/infiniterm.app"))
        );
        // cargo's own binary, which is what every test and `cargo run` is.
        assert_eq!(
            bundle_of(Path::new(
                "/Users/x/Code/infiniterm/target/debug/infiniterm"
            )),
            None
        );
        assert_eq!(bundle_of(Path::new("/infiniterm")), None);
    }

    // The argument list is the part that can be wrong: without --max-time a
    // DNS stall would keep a thread alive for the system's timeout.
    #[test]
    fn the_suggest_command_is_bounded_and_quiet() {
        let args = suggest_args("rust");
        assert!(args.contains(&"--max-time".to_string()));
        assert!(args.contains(&infiniterm_core::omni::suggest::SUGGEST_TIMEOUT_S.to_string()));
        assert!(args.contains(&"-s".to_string()), "no progress meter");
        assert!(args.last().unwrap().contains("q=rust"));
    }
}
