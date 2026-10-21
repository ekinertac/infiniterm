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
        let mut registry = CommandRegistry::new(|line| {
            eprintln!("[infiniterm] {line}");
            // Every command that runs, however it was asked for, for
            // `ift usage` (usage_log.rs). Local only.
            if let Some(id) = line.strip_prefix("cmd ").and_then(|r| r.split(' ').next()) {
                let _ = infiniterm_core::usage_log::record(
                    &infiniterm_core::paths::usage_log_path(),
                    id,
                    crate::now_ms() as u64,
                );
            }
        });
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
            updater: None,
            build: None,
            reveal_on_release: None,
            label_hits: Vec::new(),
            show_character_palette: false,
            pan: None,
            scroll_nav: Default::default(),
            left_on_canvas: false,
            middle_press: None,
            marquee: None,
            crowded: false,
            gesture: None,
            body_drag: None,
            hover_body: None,
            hover_edge: None,
            context_menu: None,
            prompt_field: Default::default(),
            query_field: Default::default(),
            shortcuts_field: Default::default(),
            shortcuts_index: 0,
            pick_request: None,
            #[cfg(unix)]
            join: None,
            last_agents: String::new(),
            #[cfg(unix)]
            join_updates: None,
            #[cfg(unix)]
            uplink: None,
            viewer: false,
            viewer_poke: false,
            #[cfg(unix)]
            canvas_rx: None,
            read_requests: vec![],
            shortcuts_scroll: gpui::ScrollHandle::new(),
            whats_new_body: None,
            omni_field: crate::field::Field::default(),
            find_field: crate::field::Field::default(),
            redraw: false,
            window_bg: None,
            window_titled: false,
            keycast_on: false,
            keycasts: Vec::new(),
            last_paint_ms: 0.,
            clipboard_out: None,
            live_sessions: vec![],
            prompt_was_open: false,
            prompt_serial: 0,
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
            timing: (0., 0., 0, 0.),
            window_seen: None,
            window_save_due: None,
            frame_rate: Default::default(),
            themes_dir: PathBuf::new(),
            backgrounds_dir: AppView::bundled_backgrounds(),
            paste_request: None,
            extracted: Default::default(),
            extracted_size: Default::default(),
            extract_request: None,
            page_menu_open: false,
            background: Default::default(),
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

    /// Something to draw a theme's colours from, in the order a build is
    /// most likely to have one: where this build ships them, then the
    /// reference checkout somebody developing has beside this repo.
    ///
    /// The 521 schemes live in the archived Tauri app
    /// (`ekinertac/infiniterm-tauri`), not in this repo, which is why the
    /// second rung is a path into a sibling checkout rather than an asset
    /// directory. `tools/bundle.sh` copies them into the Mac bundle from
    /// there; a Windows release will put them beside the exe, which is the
    /// first rung's Windows half and is also why a debug build on Windows
    /// finds nothing until that checkout exists.
    fn bundled_themes() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let dir = exe.parent()?;
        let shipped = if cfg!(target_os = "macos") {
            // infiniterm.app/Contents/MacOS/infiniterm -> Contents/Resources
            dir.parent()?.join("Resources").join("themes")
        } else {
            // An unpacked release folder: the exe with its resources beside it.
            dir.join("themes")
        };
        if shipped.is_dir() {
            return Some(shipped);
        }
        let dev = home_dir().join("Code/infiniterm-tauri/src-tauri/resources/themes");
        dev.is_dir().then_some(dev)
    }

    /// The bundle's `backgrounds`, else this checkout's while developing.
    fn bundled_backgrounds() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let in_bundle = exe
            .parent()?
            .parent()?
            .join("Resources")
            .join("backgrounds");
        if in_bundle.is_dir() {
            return Some(in_bundle);
        }
        let dev = home_dir().join("Code/infiniterm/assets/backgrounds");
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
                hex3(&self.model.config.terminal.cursor_color),
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

    /// Reads `ui-themes/<name>.json` into the model and redraws what the look
    /// drives. A missing file is told to the model, which keeps the look you
    /// had unless the name is `default`.
    pub fn load_ui_theme(&mut self, name: &str) {
        let text = infiniterm_core::config_files::ui_theme_read(name);
        self.model.apply_ui_theme_text(name, text.as_deref());
        self.after_look_change();
    }

    /// What follows a change of the look: the chrome's typeface (the rest of
    /// the look is read from the model each frame).
    fn after_look_change(&mut self) {
        self.chrome.apply_ui_typography(&self.model.config.ui);
        self.redraw = true;
    }

    /// The snippets folder into the model, as the picker opens. A missing
    /// folder is made and seeded: with the entries of the day-old
    /// `snippets.json` when there is one (left in place, never rewritten),
    /// else with the examples, so "Edit snippets…" opens files that show
    /// the shape. A file that cannot be read as text is skipped and logged.
    fn refresh_snippets(&mut self) {
        use infiniterm_core::config_files::{config_read, snippets_dir, ConfigFile};
        use infiniterm_core::snippets;
        let dir = snippets_dir();
        if !dir.is_dir() {
            let seed: Vec<(String, String)> =
                match config_read(ConfigFile::Snippets).map(|json| snippets::from_json(&json)) {
                    Some(Ok(files)) => files,
                    Some(Err(e)) => {
                        self.model
                            .log(format!("[snippets] snippets.json not moved: {e}"));
                        vec![]
                    }
                    None => snippets::EXAMPLES
                        .iter()
                        .map(|(n, t)| (n.to_string(), t.to_string()))
                        .collect(),
                };
            let written = std::fs::create_dir_all(&dir).and_then(|()| {
                seed.iter()
                    .try_for_each(|(name, text)| std::fs::write(dir.join(name), text))
            });
            if let Err(e) = written {
                self.model
                    .log(format!("[snippets] could not seed {}: {e}", dir.display()));
            }
        }
        let mut files = vec![];
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            match std::fs::read_to_string(&path) {
                Ok(text) => files.push((name, text)),
                Err(e) => self.model.log(format!("[snippets] {name} skipped: {e}")),
            }
        }
        self.model.snippets = snippets::collect(files);
    }

    /// A snippet into a card the way Cmd+V goes: the terminal's paste path
    /// (bracketed when the program asked), any other body's `insert_text`.
    /// A masked or displaced card has no live body and gets nothing.
    pub(crate) fn paste_text(&mut self, card_id: &str, text: &str) {
        let Some(body) = self.live_body(card_id) else {
            return;
        };
        match body
            .as_any_mut()
            .downcast_mut::<crate::terminal_body::TerminalBody>()
        {
            Some(t) => t.paste_text(text),
            None => body.insert_text(text),
        }
        self.flush_writes();
        self.redraw = true;
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
                // A parked card reopened: the session keeps running and is
                // listed as live, so its new body adopts it and the ring's
                // replay redraws what it missed (`reconcile_terminals`).
                Effect::ReleasePane(pane) => {
                    let session = self.backend.pty.session_id(pane);
                    if self.backend.pty.release(pane) {
                        if let Some(s) = session {
                            self.live_sessions.push(s);
                        }
                    }
                }
                Effect::KillAllPanes => self.backend.pty.kill_all(),
                Effect::ClearPane(pane) => {
                    if let Some(body) = self.terminal_for_pane(pane) {
                        body.grid.clear();
                    }
                }
                Effect::RedrawPane(pane) => {
                    let (cols, rows) = self
                        .terminal_for_pane(pane)
                        .map(|b| (b.cols(), b.rows()))
                        .unwrap_or((0, 0));
                    if self.backend.pty.redraw(pane, cols, rows)
                        == infiniterm_core::backend::Redraw::Rejoin
                    {
                        // A viewer's card: its connection is gone on purpose.
                        // The card is read as one waiting for a session (no
                        // pane), the grid stays as it is until the new
                        // join's replay resets it (`reconcile_terminals`).
                        let id = self
                            .model
                            .cards
                            .iter()
                            .find(|c| c.pane_id == Some(pane))
                            .map(|c| c.id.clone());
                        if let Some(id) = id {
                            if let Some(c) = self.model.card_mut(&id) {
                                c.pane_id = None;
                            }
                            if let Some(body) = self.bodies.get_mut(&id).and_then(|b| {
                                b.as_any_mut()
                                    .downcast_mut::<crate::terminal_body::TerminalBody>()
                            }) {
                                body.pane = None;
                                body.lost =
                                    Some(crate::terminal_body::LostLink::redraw(crate::now_ms()));
                                body.mark_dirty();
                            }
                        }
                    }
                }
                Effect::WritePane(pane, bytes) => self.backend.pty.write(pane, &bytes),
                Effect::DraftDelete(id) => {
                    let _ = infiniterm_core::files::draft_delete(&id);
                }
                Effect::WindowColor { color, save } => {
                    // The title bar reads the model; the Dock tile is ours.
                    match color {
                        Some(c) => crate::instance_icon::set_badge(c),
                        None => crate::instance_icon::clear_badge(),
                    }
                    // A remote window keeps its colour in its folder; a local
                    // one was saved by the model, as a setting.
                    if self.model.remote.is_some() {
                        use infiniterm_core::model::WindowColorSave as Save;
                        let dir = infiniterm_core::paths::app_support_dir();
                        let saved = match (save, color) {
                            (Save::Save, Some(c)) => {
                                infiniterm_core::remote_identity::save_color(&dir, Some(c))
                            }
                            (Save::Forget, _) => {
                                infiniterm_core::remote_identity::save_color(&dir, None)
                            }
                            _ => Ok(()),
                        };
                        if let Err(e) = saved {
                            self.model.notify(format!("could not save the colour: {e}"));
                        }
                    }
                }
                Effect::ReadCard {
                    request_id,
                    card_id,
                    last,
                    scrollback,
                } => {
                    // Answered by the poll timer after the card's pending
                    // output is parsed (`answer_reads`), not here: a window
                    // that is not drawing has not parsed it.
                    self.read_requests.push(crate::terminals::ReadRequest {
                        request_id,
                        card_id,
                        last,
                        scrollback,
                    });
                }
                Effect::CliReply { id, ok, text } => {
                    self.backend.cli.reply(id, ok, text);
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
                // The settings in a joined window are the host's, shown
                // read only for now: writing one here would put a file in the
                // viewer's own folder that nothing reads.
                // `ui.theme` is the exception: the look is per client (#511),
                // so a viewer's choice is written to its own settings.json.
                Effect::SaveSetting { path, .. } if self.viewer && path != "ui.theme" => {
                    self.model.notify(format!(
                        "{path}: the settings in this window are the host's; change them on the host"
                    ));
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
                // A look key goes into the ui theme in force, text patched like
                // settings.json is. The watcher reads it back a second later.
                Effect::SaveLook { path, value } => {
                    let name = if self.model.ui_theme_name.is_empty() {
                        infiniterm_core::ui_theme::DEFAULT_NAME.to_string()
                    } else {
                        self.model.ui_theme_name.clone()
                    };
                    let existing = infiniterm_core::config_files::ui_theme_read(&name)
                        .unwrap_or_else(infiniterm_core::settings_doc::default_ui_theme_text);
                    if let Some(patched) =
                        infiniterm_core::jsonc::patch_json_text(&existing, &path, &value)
                    {
                        if let Err(e) =
                            infiniterm_core::config_files::ui_theme_write(&name, &patched)
                        {
                            eprintln!("[infiniterm] could not write ui theme {name}: {e}");
                        }
                    }
                }
                Effect::LoadUiTheme(name) => self.load_ui_theme(&name),
                Effect::RefreshUiThemes => {
                    self.model.ui_theme_names = infiniterm_core::config_files::list_ui_themes();
                }
                Effect::LoadTheme(name) => self.load_theme(&name),
                Effect::RefreshThemes => self.refresh_themes(),
                Effect::RefreshSnippets => self.refresh_snippets(),
                Effect::PasteText { card_id, text } => self.paste_text(&card_id, &text),
                // The right-click menu's terminal acts (#278). The clipboard is
                // read in the poll task, where there is an app context.
                Effect::Terminal { card_id, action } => {
                    use infiniterm_core::model::TerminalAction;
                    match action {
                        TerminalAction::Copy => {
                            if let Some(text) = self
                                .terminal_body(&card_id)
                                .and_then(|b| b.grid.selection_text())
                            {
                                self.clipboard_out = Some(text);
                            }
                        }
                        TerminalAction::Paste => self.paste_request = Some(card_id),
                        TerminalAction::SearchSelection => {
                            if let Some(text) = self
                                .terminal_body(&card_id)
                                .and_then(|b| b.grid.selection_text())
                            {
                                self.model.open_web_search(&text);
                            }
                        }
                    }
                }
                Effect::Editor { card_id, action } => self.editor_effect(&card_id, action),
                Effect::Browser { card_id, action } => self.browser_effect(&card_id, action),
                // A terminal's scrollback is ours to search; a page is
                // Chromium's (browsers.rs).
                Effect::Find { card_id, request } => {
                    if self.terminal_body(&card_id).is_some() {
                        self.terminal_find(&card_id, request);
                    } else {
                        self.find_effect(&card_id, request);
                    }
                }
                Effect::FindSelection(card_id) => {
                    // The first line of the selection: a query is one line.
                    let text = self
                        .terminal_body(&card_id)
                        .and_then(|b| b.grid.selection_text())
                        .and_then(|t| t.lines().next().map(str::to_string))
                        .filter(|t| !t.trim().is_empty());
                    match text {
                        Some(t) => {
                            self.find_field = crate::field::Field::open(&t, false);
                            self.model.find_with(&t);
                        }
                        None => self.model.notify("select some text first"),
                    }
                }
                // Held rather than written: perform_effects has no App to
                // write through, and threading one into sixteen call sites
                // for a clipboard is the wrong trade. The frame flushes it,
                // the same way a terminal's OSC 52 text is flushed.
                Effect::Copy(text) => self.clipboard_out = Some(text),
                Effect::PickFile { from } => self.pick_request = Some(from),
                Effect::Visual(card_id) => {
                    if let Some(b) = self.terminal_body(&card_id) {
                        b.grid.visual_enter();
                        b.dirty = true;
                    }
                }
                Effect::Log(line) => eprintln!("[infiniterm] {line}"),
                Effect::Warn(line) => eprintln!("[infiniterm/warn] {line}"),
                Effect::AgentLog(line) => append_agent_log(&line),
                Effect::Reload => {
                    self.flush_save();
                    leave_now();
                }
                // Needs the Window, which effects do not have; the frame
                // makes the call. Same shape as the owed-frame flag.
                Effect::ShowCharacterPalette => self.show_character_palette = true,
                Effect::ToggleFullScreen => crate::fullscreen::toggle(),
                Effect::ToggleKeycast => {
                    self.keycast_on = !self.keycast_on;
                    self.keycasts.clear();
                    self.model.notify(if self.keycast_on {
                        "showing pressed shortcuts"
                    } else {
                        "pressed shortcuts hidden"
                    });
                    self.redraw = true;
                }
                Effect::CheckForUpdate => match &self.updater {
                    Some(u) => {
                        u.check_now();
                        self.model.notify("checking for updates…");
                    }
                    None => self
                        .model
                        .notify("updates are for the app installed in Applications"),
                },
                Effect::InstallUpdate => {
                    if self.updater.as_ref().is_some_and(|u| u.staged.is_some()) {
                        self.install_update_and_restart();
                    } else {
                        self.model.notify(
                            "no update is downloaded yet; App: check for updates looks now",
                        );
                    }
                }
                // A restart with an update waiting installs it: the notice
                // says "restart to install", and this is that restart.
                Effect::Restart if self.updater.as_ref().is_some_and(|u| u.staged.is_some()) => {
                    self.install_update_and_restart();
                }
                Effect::Restart => {
                    self.flush_save();
                    match relaunch_after_exit() {
                        // The waiter is running and holds our pid; nothing
                        // left to do but go, and it opens the bundle once
                        // we are gone. Cards come back to their sessions
                        // and the frame comes back from window.json.
                        Ok(()) => leave_now(),
                        // Still here, so say why rather than quitting into
                        // nothing and looking like a crash.
                        Err(e) => self.model.notify(format!("could not restart: {e}")),
                    }
                }
                Effect::RunCommand(id) => self.run_command(&id),
                Effect::LogFps(n) => eprintln!(
                    "[infiniterm] stress zoom {n}: {:.0} fps",
                    self.frame_rate.shown(now_ms()).unwrap_or(0.)
                ),
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
        } else if self.model.whats_new_wants_frame(now) {
            "whats-new"
        } else if self.model.notice_expired(now) {
            "notice"
        } else if self.model.done_seen_due(now) {
            "seen"
        } else {
            "idle"
        }
    }

    /// Whether content frames are rationed (`chrome::FAR_REFRESH_MS`): only
    /// while the last frame was over its glyph budget and drew bars.
    pub fn far(&self) -> bool {
        self.crowded
    }

    pub fn needs_frame(&self) -> bool {
        self.redraw
            || self.animator.is_running()
            || self.pan.is_some()
            || self.gesture.is_some()
            || !self.glides.is_empty()
            || self.background.needs_frame(crate::now_ms())
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
                (content && due)
                    || self.model.notice_expired(now)
                    || self.model.whats_new_wants_frame(now)
                    || self.model.done_seen_due(now)
                    || self.frame_rate.expired(now)
                    || self
                        .keycasts
                        .iter()
                        .any(|k| crate::keycast::keycast_alpha(k.at, now) > 0.)
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
                    let host_size = self.backend.pty.viewer_pane_size(pane);
                    let fed = match self.terminal_for_pane(pane) {
                        Some(body) => {
                            if let Some((c, r)) = host_size {
                                body.fit_host_cells(c as usize, r as usize);
                            }
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
            let waiting = self.model.waiting_parked().map(|c| c.id.clone());
            self.model.apply_hook(&report);
            let after = self.model.card(&report.card_id).map(|c| c.agent);
            log_agent_event(&report, before, after);
            // A closed card's agent starting to wait changes the status bar.
            self.redraw |=
                before != after || waiting != self.model.waiting_parked().map(|c| c.id.clone());
        }
        while let Ok(statuses) = self.backend.pane_status.try_recv() {
            self.model.apply_pane_statuses(&statuses);
        }
        while let Ok(change) = self.backend.config_changes.try_recv() {
            // A joined window runs on the HOST's files, which arrive on the
            // canvas stream (#433); its own folder is empty and stays so,
            // except for the look, which is per client (#511): `ui.theme` in
            // its own settings.json and the ui theme files.
            if self.viewer {
                match change.file {
                    ConfigFile::Settings => {
                        let name =
                            infiniterm_core::model::settings_in::ui_theme_name_in(&change.contents)
                                .unwrap_or_else(|| infiniterm_core::ui_theme::DEFAULT_NAME.into());
                        self.model.note_ui_theme_setting(&name);
                    }
                    ConfigFile::UiTheme => {
                        if let Some(name) = &change.name {
                            self.model.ui_theme_file_changed(name, &change.contents);
                            self.after_look_change();
                        }
                    }
                    _ => {}
                }
                self.redraw = true;
                continue;
            }
            match change.file {
                ConfigFile::Settings => {
                    self.apply_settings_text(&change.contents);
                    #[cfg(unix)]
                    if let Some(hub) = &self.join {
                        hub.publish_config(
                            infiniterm_core::backend::join::KIND_SETTINGS,
                            &change.contents,
                        );
                    }
                }
                // A ui theme is this window's own look (#425, #511): the look
                // stays per client, so it is never published to a joined viewer.
                ConfigFile::UiTheme => {
                    if let Some(name) = &change.name {
                        self.model.ui_theme_file_changed(name, &change.contents);
                        self.after_look_change();
                    }
                }
                _ => {
                    self.model.apply_keymap_text(&change.contents);
                    #[cfg(unix)]
                    if let Some(hub) = &self.join {
                        hub.publish_config(
                            infiniterm_core::backend::join::KIND_KEYBINDINGS,
                            &change.contents,
                        );
                    }
                }
            }
            // An edit made outside the app arrives while the window idles:
            // without this the new settings were applied and never drawn
            // until something else asked for a frame (#216).
            self.redraw = true;
        }
        self.drain_updates();
        while let Ok(req) = self.backend.cli_requests.try_recv() {
            let commands: Vec<(&str, &str)> = self
                .registry
                .all()
                .iter()
                .map(|c| (c.id.as_str(), c.label.as_str()))
                .collect();
            let reply = self.model.run_ift(&req, &commands);
            // An `edit` is answered when its in-place editor closes
            // (Effect::CliReply), which is what the `ift` is waiting for.
            if !std::mem::take(&mut self.model.reply_deferred) {
                self.backend.cli.reply(req.id, reply.ok, reply.text);
            }
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
        // Folders sent by Finder's service: a terminal card each, the last
        // focused. A refused one (gone since) is a notice, not a card.
        for dir in crate::finder_service::take() {
            if let Err(e) = self.model.open_terminal_at(&dir) {
                self.model.notify(e);
            }
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
        let mut state = crate::window_state::WindowState::of(bounds);
        // Covering, gpui sees a window the size of the screen. What is kept
        // is the frame from before, marked full screen, so a quit reopens
        // covered at launch and un-covering has somewhere to return.
        if crate::fullscreen::covering() {
            match &self.window_seen {
                Some(before) => {
                    state = crate::window_state::WindowState {
                        mode: "fullscreen".into(),
                        ..before.clone()
                    }
                }
                None => return,
            }
        }
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
        // A viewer saves nothing of its own: what it edited goes up to the
        // host, which makes it so (#423).
        if self.viewer {
            self.send_up();
            return;
        }
        if let Some(text) = self.model.save_text() {
            if let Err(e) = infiniterm_core::layout_file::write_layout(&text) {
                eprintln!("[infiniterm] could not save the layout: {e}");
            }
            // The same text, to every viewer that joined, with the revision
            // it is. The viewer keeps its own viewport and focus; nothing is
            // stripped here.
            #[cfg(unix)]
            if let Some(hub) = &self.join {
                // Remembered by revision, so a viewer's edit can be compared
                // with the canvas it started from and merged field by field.
                let rev = self.model.record_published(&text);
                hub.publish_layout(&text, rev);
            }
        }
    }

    /// A viewer's canvas, to the host, on the revision it was edited on.
    /// Joining is unix-only, so there is no host to send to elsewhere.
    #[cfg(windows)]
    fn send_up(&mut self) {}

    #[cfg(unix)]
    fn send_up(&mut self) {
        // Nothing to say before the host's own canvas has arrived.
        if !self.model.snapshot_seen {
            return;
        }
        let Some(up) = &self.uplink else {
            return;
        };
        let layout = self.model.layout_snapshot();
        let _ = up.send(infiniterm_core::backend::join::RemoteUpdate {
            base: self.model.layout_rev,
            layout,
        });
    }

    /// What the viewers sent up since the last tick (#423), applied to this
    /// canvas in place. An edit made on an old canvas is dropped and the host
    /// republishes its own, which is how that viewer finds out. True when the
    /// canvas changed, so the next frame starts any shell it now needs.
    #[cfg(windows)]
    pub fn apply_remote_updates(&mut self) -> bool {
        false
    }

    #[cfg(unix)]
    pub fn apply_remote_updates(&mut self) -> bool {
        use infiniterm_core::model::persist::RemoteApply;
        let Some(rx) = &self.join_updates else {
            return false;
        };
        let updates: Vec<_> = rx.try_iter().collect();
        let mut changed = false;
        for u in updates {
            match self.model.apply_remote_layout(&u.layout, u.base) {
                Ok(RemoteApply::Applied) => changed = true,
                Ok(RemoteApply::Stale) => {
                    self.write_layout();
                    changed = true;
                }
                Err(why) => eprintln!("[infiniterm] a viewer's canvas was refused: {why}"),
            }
        }
        changed
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

    /// A settings file's text applied to the model and the chrome that
    /// follows it. The same for the app's own file and, in a joined window,
    /// the host's.
    fn apply_settings_text(&mut self, text: &str) {
        self.model.apply_settings_text(text);
        self.after_settings();
    }

    /// The host's file in a joined window: everything but the look (#511).
    fn apply_host_settings_text(&mut self, text: &str) {
        self.model.apply_host_settings_text(text);
        self.after_settings();
    }

    fn after_settings(&mut self) {
        self.chrome.apply_ui_typography(&self.model.config.ui);
        self.animator.animations_on = self.model.config.ui.animations && !self.reduce_motion;
    }

    /// What the host's canvas said since the last tick (a viewer, #423): a
    /// layout is applied in place, agent states on top of it, and a status
    /// (joined, lost, refused) is shown as a notice.
    #[cfg(windows)]
    pub fn drain_canvas(&mut self) {}

    #[cfg(unix)]
    pub fn drain_canvas(&mut self) {
        use infiniterm_core::backend::join_client::CanvasEvent;
        let Some(rx) = &self.canvas_rx else {
            return;
        };
        let events: Vec<CanvasEvent> = rx.try_iter().collect();
        for event in events {
            match event {
                CanvasEvent::Layout(text) => {
                    // An edit still waiting for its debounce would be
                    // overwritten by this snapshot (a closed card comes
                    // back, a move is undone) and the next send would carry
                    // the host's own state. Send it first, on the revision
                    // it was made on; the host's next canvas has it applied.
                    if self.save_due.is_some() || self.model.dirty_layout {
                        self.flush_save();
                    }
                    if let Err(why) = self.model.apply_snapshot(&text) {
                        self.model.notify(&why);
                    }
                }
                CanvasEvent::Revision(rev) => self.model.layout_rev = rev,
                CanvasEvent::Agents(text) => self.model.apply_agent_states(&text),
                CanvasEvent::Connected => {
                    self.viewer_poke = true;
                    self.redraw = true;
                }
                CanvasEvent::Settings(text) => {
                    self.apply_host_settings_text(&text);
                    self.redraw = true;
                }
                CanvasEvent::Keybindings(text) => {
                    self.model.apply_keymap_text(&text);
                    self.redraw = true;
                }
                CanvasEvent::Status(text) => self.model.notify(&text),
                CanvasEvent::Fatal(text) => {
                    eprintln!("[infiniterm] {text}");
                    self.model.notify(&text);
                }
            }
        }
    }

    /// The host sends viewers the agent states when they change (#423).
    /// There are no viewers on Windows.
    #[cfg(windows)]
    pub fn publish_agents(&mut self) {}

    #[cfg(unix)]
    pub fn publish_agents(&mut self) {
        let Some(hub) = &self.join else {
            return;
        };
        let text = self.model.agent_states_text();
        if text != self.last_agents {
            hub.publish_agents(&text);
            self.last_agents = text;
        }
    }

    pub fn maybe_sweep(&mut self, now: f64) {
        // A viewer's states are the host's; a local sweep would end a working
        // card the host still has working.
        if self.viewer {
            return;
        }
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

/// Opens the socket `ift join` viewers reach through ssh, and gives them the
/// canvas as it is now (#423). A failure is said and costs nothing else: the
/// app runs without joining. Unix only: the hub rides on a unix socket.
#[cfg(windows)]
fn start_join(_app: &mut AppView) {}

#[cfg(unix)]
fn start_join(app: &mut AppView) {
    // A viewer is a window onto the host's canvas, never a host: it would
    // relay sessions that are not its own (found in the #423 drive, which
    // showed a viewer binding its own j.sock).
    let Some((relay, write, nudge)) = app.backend.pty.join_parts() else {
        return;
    };
    if !app.model.config.join.enabled || app.viewer {
        // No server, so nothing is kept for one.
        relay.lock().unwrap().set_enabled(false);
        return;
    }
    let hub = infiniterm_core::backend::join::JoinHub::new(relay, write).with_nudge(nudge);
    // The host's files as they are now, for a viewer that joins before the
    // first change (#433).
    for (file, kind) in [
        (
            ConfigFile::Settings,
            infiniterm_core::backend::join::KIND_SETTINGS,
        ),
        (
            ConfigFile::Keybindings,
            infiniterm_core::backend::join::KIND_KEYBINDINGS,
        ),
    ] {
        if let Some(text) = config_read(file) {
            hub.publish_config(kind, &text);
        }
    }
    match hub.serve(&infiniterm_core::paths::join_socket()) {
        Ok(()) => {
            app.join_updates = hub.take_updates();
            app.join = Some(hub);
        }
        Err(e) => eprintln!("[infiniterm/warn] ift join is off: {e}"),
    }
    // Viewers that join before the first save still get a canvas.
    app.write_layout();
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
    // The zsh integration every new card's shell loads (command marks for
    // program_state.rs), rewritten so an update's version wins.
    if let Err(e) = infiniterm_core::shell_integration::install(
        &infiniterm_core::paths::shell_integration_dir(),
    ) {
        eprintln!("[infiniterm/warn] could not write the zsh integration: {e}");
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
    // The look keys an older settings.json holds move into
    // ui-themes/default.json BEFORE the settings are read (#511), and that
    // file is created when missing. A joined viewer has no files of its own
    // to move (its look is whatever its folder holds).
    // (There is no joined viewer on Windows, so the move always runs there.)
    #[cfg(unix)]
    let is_viewer = infiniterm_core::backend::remote::join_from_env().is_some();
    #[cfg(windows)]
    let is_viewer = false;
    if !is_viewer {
        match infiniterm_core::config_files::migrate_look_keys() {
            Ok(keys) if !keys.is_empty() => eprintln!(
                "[infiniterm] moved {} look settings from settings.json to ui-themes/default.json: {}",
                keys.len(),
                keys.join(", ")
            ),
            Ok(_) => {}
            Err(e) => eprintln!("[infiniterm/warn] could not move the look settings: {e}"),
        }
    }
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
    // The look, now, so the first frame wears it; the effect that the
    // settings queued for the same file would only load it twice.
    let ui_theme = app.model.config.ui.theme.clone();
    app.load_ui_theme(&ui_theme);
    app.model
        .effects
        .retain(|e| !matches!(e, Effect::LoadUiTheme(_)));
    app.chrome.apply_ui_typography(&app.model.config.ui);
    app.model.apply_keymap_text(&keys);
    // The "Start here" card's file, rewritten every launch like the defaults
    // so its keys follow the keymap in force and its agent list follows what
    // is installed now. A first launch opens it (`seed_first_card`).
    {
        use infiniterm_core::welcome;
        let env_path = |k: &str| std::env::var_os(k).map(std::path::PathBuf::from);
        let home = infiniterm_core::paths::home_dir();
        let agents = welcome::agents_on_this_mac(
            &home,
            env_path("XDG_CONFIG_HOME").as_deref(),
            env_path("CODEX_HOME").as_deref(),
        );
        let path = welcome::welcome_path();
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| {
                std::fs::write(
                    &path,
                    welcome::render(&app.model.keymap, &agents, app.model.modifier_labels()),
                )
            });
        if let Err(e) = written {
            eprintln!("[infiniterm/warn] could not write {}: {e}", path.display());
        }
        // The changelog card's file (`help.changelog`), this build's own text.
        let path = infiniterm_core::changelog::changelog_path();
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, infiniterm_core::changelog::text()));
        if let Err(e) = written {
            eprintln!("[infiniterm/warn] could not write {}: {e}", path.display());
        }
        // The first launch after an update shows what changed (#402): the
        // overlay's file is this build's released changelog, and it opens
        // when the version last run is older than this one. A first install,
        // a scratch data dir and a remote window have no stored version, or
        // none worth showing, and only record it.
        let whats_new = infiniterm_core::changelog::whats_new_path();
        let written = whats_new
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&whats_new, infiniterm_core::changelog::released_text()));
        if let Err(e) = written {
            eprintln!(
                "[infiniterm/warn] could not write {}: {e}",
                whats_new.display()
            );
        }
        let running = env!("CARGO_PKG_VERSION");
        let updated = infiniterm_core::changelog::read_last_version()
            .is_some_and(|stored| infiniterm_core::changelog::version_newer(&stored, running));
        if updated && app.model.config.ui.show_whats_new && app.model.remote.is_none() {
            app.model.open_whats_new();
        }
        if let Err(e) = infiniterm_core::changelog::write_last_version(running) {
            eprintln!("[infiniterm/warn] could not record the version: {e}");
        }
        // The in-app docs (`help.docs`), the same pages as infiniterm.app,
        // rewritten every launch so they match this build.
        let dir = infiniterm_core::help_docs::docs_dir();
        let written = std::fs::create_dir_all(&dir).and_then(|()| {
            infiniterm_core::help_docs::files()
                .into_iter()
                .try_for_each(|(name, text)| std::fs::write(dir.join(name), text))
        });
        if let Err(e) = written {
            eprintln!(
                "[infiniterm/warn] could not write the docs to {}: {e}",
                dir.display()
            );
        }
    }
    app.animator.animations_on = app.model.config.ui.animations && !app.reduce_motion;
    // Before any card exists: which sessions the last launch left running.
    // A remote instance (`ift connect`): what it is called, its colour, and
    // a Dock icon badged with it. Before any card, since the model refuses
    // browser cards once this is set.
    app.model.remote = infiniterm_core::remote_identity::RemoteIdentity::from_env();
    crate::instance_icon::init();
    if let Some(r) = &app.model.remote {
        crate::instance_icon::set_badge(r.color);
        // The settings were applied before this, with the Mac's folder.
        app.model.start_dir = "~".into();
    }
    #[cfg(unix)]
    let join_host = infiniterm_core::backend::remote::join_from_env();
    // No joining on Windows (unix sockets and ssh), so never a viewer.
    #[cfg(windows)]
    let join_host: Option<()> = None;
    app.viewer = join_host.is_some();
    // A viewer asks the host nothing at launch: the canvas arrives from the
    // client thread, and every session on it is adoptable through the host.
    app.live_sessions = if app.viewer {
        vec![]
    } else {
        app.backend.pty.live_sessions()
    };
    app.model.can_park = app.backend.pty.can_park();
    if let Some(host) = join_host {
        // An empty, read-only canvas; the first snapshot fills it in place.
        app.model.load_layout(None);
        app.model.first_run = false;
        app.model.read_only = true;
        #[cfg(unix)]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let (edit_tx, edit_rx) = std::sync::mpsc::channel();
            app.canvas_rx = Some(rx);
            app.uplink = Some(edit_tx);
            std::thread::spawn(move || {
                infiniterm_core::backend::join_client::run_canvas_client(host, tx, edit_rx)
            });
        }
        #[cfg(windows)]
        let _ = host;
    } else {
        app.model
            .load_layout(infiniterm_core::layout_file::read_layout().as_deref());
    }
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
    start_updater(app);
    start_join(app);
    app.perform_effects();
    if !app.backend.socket_ok {
        app.model
            .notify("could not bind the socket: hooks and ift are off");
    } else if let Some(why) = app.backend.fell_back.clone() {
        app.model.notify(&why);
    }
}

/// One instance, as the reference's single-instance plugin enforced it:
/// the endpoint is the lock (`infiniterm-core`'s `transport`, a unix socket
/// or a named pipe). A second launch on the same data dir activates the
/// first and exits before it could race it for the save file.
#[cfg(not(windows))]
pub fn another_instance_holds_the_socket() -> bool {
    infiniterm_core::transport::is_live(&infiniterm_core::paths::socket_path())
}

/// Two questions on Windows, not one: something is listening AND the process
/// that claimed the endpoint still exists.
///
/// A terminated process is not reaped while a thread of it is stuck in the
/// kernel, and CEF leaves one that is, so the pipe goes on accepting
/// connections for about forty seconds after the app has gone. A relaunch
/// inside that window was told the endpoint was held and exited, which reads
/// as "infiniterm will not start" (measured 2026-09-25). See
/// `infiniterm_core::instance` for why this cannot let two instances onto
/// one save file, which an earlier attempt at the same problem did.
#[cfg(windows)]
pub fn another_instance_holds_the_socket() -> bool {
    let data = infiniterm_core::paths::app_support_dir();
    infiniterm_core::instance::holds_the_endpoint(
        infiniterm_core::transport::is_live(&infiniterm_core::paths::socket_path()),
        infiniterm_core::instance::owner(&data),
        infiniterm_core::instance::process_alive,
    )
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
#[cfg(not(windows))]
fn bundle_of(exe: &std::path::Path) -> Option<&std::path::Path> {
    exe.ancestors()
        .nth(3)
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
}

/// What an update replaces on Windows: the folder the exe sits in, which is
/// what `tools/dist.ps1` produces and what somebody unpacks. There is no
/// bundle to walk up to and no install location to insist on; whether that
/// folder may be updated at all is `update::applies_to`.
#[cfg(windows)]
fn bundle_of(exe: &std::path::Path) -> Option<&std::path::Path> {
    exe.parent()
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
/// The updater, for the app installed in an Applications folder and never
/// for a scratch instance or a development bundle (`update::applies_to`).
/// The staging dir from the last run is emptied first: it holds the bundle
/// the last update moved aside, or a download nobody installed.
fn start_updater(app: &mut AppView) {
    // The status bar's build number, which is the whole of the updater that
    // Windows has so far. It is stamped into the binary there rather than
    // read back out of a bundle, so there is nothing to find first; see
    // `updater::build_of`.
    #[cfg(windows)]
    {
        app.build = crate::updater::build_of(std::path::Path::new(""));
    }
    #[cfg(not(windows))]
    {
        app.build = std::env::current_exe()
            .ok()
            .and_then(|exe| bundle_of(&exe).and_then(crate::updater::build_of));
    }
    if std::env::var_os("INFINITERM_DATA_DIR").is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(bundle) = bundle_of(&exe) else {
        return;
    };
    if !infiniterm_core::update::applies_to(bundle, &home_dir()) {
        return;
    }
    let dir = infiniterm_core::paths::app_support_dir().join("update");
    crate::updater::clean(&dir);
    let Some(running) = crate::updater::build_of(bundle) else {
        // Windows stamps this at compile time, so a build without one is a
        // `cargo build` rather than a release; macOS reads it from the
        // bundle's plist.
        eprintln!("[infiniterm] no build number in this build; updates are off");
        return;
    };
    app.updater = Some(crate::updater::start(bundle.to_path_buf(), running, dir));
}

impl AppView {
    /// What the updater thread found, into the chrome: a staged update
    /// gets a notice and the status bar's hint; a check somebody asked for
    /// says how it went.
    fn drain_updates(&mut self) {
        let Some(u) = &mut self.updater else { return };
        let mut said: Option<String> = None;
        while let Ok(e) = u.events.try_recv() {
            said = Some(match e {
                crate::updater::UpdateEvent::Ready(s) => {
                    let text = format!(
                        "infiniterm {} ({}) is ready: restart to install (Cmd+Ctrl+Shift+R)",
                        s.version, s.build
                    );
                    u.staged = Some(s);
                    text
                }
                crate::updater::UpdateEvent::UpToDate => "infiniterm is up to date".into(),
                crate::updater::UpdateEvent::Failed(e) => format!("update check failed: {e}"),
            });
        }
        if let Some(text) = said {
            // After the "checking for updates…" line, which a fast answer
            // would otherwise wipe before it could be read.
            self.model.notify_after_read(text);
            self.redraw = true;
        }
    }

    /// Hands the swap to a waiter that runs once this process is gone
    /// (`update::swap_script`), then quits. The Applications folder must be
    /// writable by this user, or the move would fail after we had already
    /// gone; checked first, and said.
    fn install_update_and_restart(&mut self) {
        let Some(u) = &self.updater else { return };
        let Some(staged) = u.staged.clone() else {
            return;
        };
        // The app on disk may be newer than what was staged (installed
        // since): then this is a plain restart, and the stale download goes.
        let on_disk = std::fs::read_to_string(u.bundle.join("Contents/Info.plist"))
            .ok()
            .and_then(|t| infiniterm_core::update::plist_build(&t));
        if !infiniterm_core::update::staged_is_newer(staged.build, on_disk) {
            let _ = std::fs::remove_dir_all(&staged.app);
            if let Some(u) = self.updater.as_mut() {
                u.staged = None;
            }
            self.flush_save();
            if relaunch_after_exit().is_ok() {
                std::process::exit(0);
            }
            return;
        }
        let parent = u.bundle.parent().unwrap_or(std::path::Path::new("/"));
        let probe = parent.join(".infiniterm-update-probe");
        if std::fs::write(&probe, b"").is_err() {
            self.model.notify(format!(
                "cannot replace the app: {} is not writable; install the DMG from the releases page",
                parent.display()
            ));
            return;
        }
        let _ = std::fs::remove_file(&probe);
        // The old app moved out of the way, deleted at the next launch by
        // `updater::clean`. A bundle on macOS; on Windows the release is a
        // plain folder, and a `.app` suffix there would only be confusing.
        let aside = if cfg!(windows) {
            u.dir.join(format!("old-{}", std::process::id()))
        } else {
            u.dir.join(format!("old-{}.app", std::process::id()))
        };
        let script = infiniterm_core::update::swap_script(
            std::process::id(),
            &u.bundle.to_string_lossy(),
            &staged.app.to_string_lossy(),
            &aside.to_string_lossy(),
        );
        self.flush_save();
        // `spawn_waiter`, not a bare `/bin/sh`, which is what this used to
        // be on both platforms: Windows has no /bin/sh, so every Windows
        // update got as far as downloading, verifying and staging and then
        // failed at the last step with "could not start the update". It
        // also has to break away from the job, or it would be killed by the
        // exit below before it could do anything.
        match spawn_waiter(&script) {
            Ok(()) => leave_now(),
            Err(e) => self
                .model
                .notify(format!("could not start the update: {e}")),
        }
    }
}

/// End the app, from anywhere that means to.
///
/// Four callers: the window closing, Reload, Restart, and installing an
/// update. They all have the same two obligations and it is not obvious
/// that they do, which is why this is one function and not four exits.
///
/// Give up the endpoint claim, because the named pipe outlives the process
/// by about half a minute on Windows and the claim file is what tells the
/// next launch nobody is behind it (`infiniterm_core::instance`).
///
/// Then leave WITHOUT running teardown. `std::process::exit` runs every
/// DLL's detach and libcef's does not return, which hung a Restart with the
/// waiter already running and the app still on screen (2026-09-26). The job
/// object takes the subprocesses; see `infiniterm_core::job::exit_now`.
///
/// Callers must have flushed anything they care about first: this does not
/// come back.
pub fn leave_now() -> ! {
    #[cfg(windows)]
    {
        infiniterm_core::instance::release(&infiniterm_core::paths::app_support_dir());
        infiniterm_core::job::exit_now()
    }
    #[cfg(not(windows))]
    std::process::exit(0)
}

/// Starts a script that must OUTLIVE this process: the waiter that reopens
/// the app, or the one that swaps an update in once the exe is unlocked.
///
/// The one place that knowledge lives, because getting it wrong is silent
/// on both platforms — the waiter dies and Restart simply does nothing.
#[cfg(not(windows))]
fn spawn_waiter(script: &str) -> Result<(), String> {
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// PowerShell, the only shell Windows is sure to have.
///
/// CREATE_BREAKAWAY_FROM_JOB is what makes this work at all: every process
/// infiniterm starts is in the job that kills the tree when the app ends
/// (`infiniterm_core::job`), and a waiter for the app's exit that dies with
/// the app is a waiter that never runs. The job allows the breakaway; this
/// asks for it. CREATE_NO_WINDOW keeps the console from flashing over
/// whatever is on screen, which `-WindowStyle Hidden` alone does not.
#[cfg(windows)]
fn spawn_waiter(script: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new(infiniterm_core::shell_cmd::default_shell())
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", script])
        .creation_flags(CREATE_BREAKAWAY_FROM_JOB | CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn relaunch_after_exit() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundle = bundle_of(&exe).ok_or("not running from an .app bundle")?;
    spawn_waiter(&format!(
        "while kill -0 {pid} 2>/dev/null; do sleep 0.1; done; exec open {bundle}",
        pid = std::process::id(),
        bundle = infiniterm_core::drop::posix_quote(&bundle.to_string_lossy()),
    ))
}

#[cfg(windows)]
fn relaunch_after_exit() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    spawn_waiter(&format!(
        "Wait-Process -Id {pid} -ErrorAction SilentlyContinue; Start-Process -FilePath {exe}",
        pid = std::process::id(),
        exe = infiniterm_core::drop::quote_for(
            &exe.to_string_lossy(),
            infiniterm_core::shell_cmd::ShellKind::PowerShell
        ),
    ))
}

#[cfg(test)]
mod tests {

    // What an update replaces on Windows: the folder somebody unpacked,
    // whatever it is called and wherever it is.
    #[cfg(windows)]
    #[test]
    fn the_folder_is_the_one_the_executable_sits_in() {
        use std::path::Path;
        assert_eq!(
            bundle_of(Path::new(r"C:\Program Files\infiniterm\infiniterm.exe")),
            Some(Path::new(r"C:\Program Files\infiniterm"))
        );
        assert_eq!(bundle_of(Path::new("infiniterm.exe")), Some(Path::new("")));
    }
    use super::*;

    // Three up and it must be the bundle, or `open` gets handed a path
    // that is not an app and the restart quits into nothing. macOS only:
    // Windows has no bundle to walk up to and `bundle_of` is the exe's own
    // folder there.
    #[cfg(not(windows))]
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
