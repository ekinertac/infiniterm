//! A canvas host with no window (#438): the same model, backend and join
//! server the app runs, driven by one `tick` instead of gpui's frame and poll.
//!
//! Why it exists: a viewer (`ift join`) can only join a host that serves
//! `j.sock`, and only the running app did. With the app closed, the cards still
//! run (each is an `iftd` session), so `ift serve` runs this loop to hold the
//! canvas and serve the viewers until the app starts again.
//!
//! The rule that keeps it honest: nothing here decides what a canvas edit, a
//! hook, a parked card or an agent state MEANS. Every such step is the model's
//! own function (`apply_pane_event`, `apply_hook`, `apply_remote_layout`,
//! `record_published`, `save_text`, `sweep_stale`, `run_ift`, `tick`), so the
//! app and the host cannot drift. What is written here is only the glue the app
//! keeps in gpui land: which channel to drain, when to save, and starting or
//! adopting a shell for a card that has none.
//!
//! What the app does on a tick that this does NOT (each has a reason; the PR
//! that added this lists them too):
//! - parse bytes into a terminal grid, so `ift read` has no screen to read and
//!   the kitty keyboard flag and the program's window title are not learned
//!   (a grid needs a font and a window; a viewer draws its own from the ring);
//! - the effects that are about a window: animations, the palette and menus,
//!   fullscreen, editors, browsers, pickers, the clipboard, the character
//!   palette, updates and relaunching;
//! - the agent log (`agent.log`), the usage log, drafts, the omnibox history,
//!   what's-new, the welcome and docs files (writing them is the app's launch);
//! - the dead-session scrollback from disk (`take_ring`) and the orphan sweep:
//!   a host never kills a session the canvas file does not name;
//! - the window frame, find, selection, focus trail seen-time and hover state.
//!
//! Called by `ift serve` (infiniterm-cli). Related: app.rs (`Backend`, the
//! channels this drains), model/ (every rule), backend/join.rs (`JoinHub`),
//! runtime.rs in the ui crate (the app's own tick, which this mirrors).

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use crate::app::Backend;
use crate::backend::join::{JoinHub, RemoteUpdate, KIND_KEYBINDINGS, KIND_SETTINGS};
use crate::backend::PaneEvent;
use crate::commands::CommandRegistry;
use crate::config::TerminalBackend;
use crate::config_files::{config_read, ConfigFile};
use crate::model::persist::RemoteApply;
use crate::model::register::{describe_context, register_commands, run_with_effects};
use crate::model::{Effect, Model};

/// Where a running `ift serve` writes its pid, beside the canvas file.
pub fn serve_pid_file() -> PathBuf {
    crate::paths::socket_path().with_file_name("serve.pid")
}

/// Whether a process's command line is `ift serve ...`: the first word's file
/// name is `ift` and the next word is `serve`. Pure, so a stale pid file that
/// now names an unrelated process is told apart in a test.
pub fn is_serve_command(command: &str) -> bool {
    let mut words = command.split_whitespace();
    let program = words.next().unwrap_or("");
    let name = program.rsplit('/').next().unwrap_or("");
    name == "ift" && words.next() == Some("serve")
}

/// The pid in `pid_file`, when that process is really an `ift serve`. A pid
/// file outlives a crash, and the number may belong to anything by now: nobody
/// is signalled on its say-so alone.
pub fn live_serve(pid_file: &Path) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(pid_file)
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()?;
    is_serve_command(&String::from_utf8_lossy(&out.stdout)).then_some(pid)
}

/// What `stop_running_serve` found.
#[derive(Debug, PartialEq, Eq)]
pub enum ServeStop {
    /// No `ift serve` here (or a stale pid file): nothing to do.
    NotRunning,
    /// It flushed the canvas and exited.
    Stopped,
    /// It was asked and is still there after the wait; carries its pid.
    Stuck(u32),
}

/// What the app does first at its launch (#438): ask a running `ift serve` of
/// this data dir to save the canvas and exit, and wait for it, so the app
/// loads that canvas, binds the sockets and adopts the sessions. Only a pid
/// that `live_serve` confirms is signalled. The viewers see their connection
/// end and join the app by themselves (#442).
pub fn stop_running_serve(pid_file: &Path, wait: std::time::Duration) -> ServeStop {
    let Some(pid) = live_serve(pid_file) else {
        return ServeStop::NotRunning;
    };
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
    let deadline = std::time::Instant::now() + wait;
    while std::time::Instant::now() < deadline {
        if live_serve(pid_file) != Some(pid) {
            return ServeStop::Stopped;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    ServeStop::Stuck(pid)
}

/// The save debounce, as the app's (`runtime.rs` `SAVE_DEBOUNCE_MS`): a burst
/// of edits is one write.
pub const SAVE_DEBOUNCE_MS: f64 = 500.;

/// How often the stale-agent sweep runs, as the app's `SWEEP_MS`.
const SWEEP_MS: f64 = 10_000.;

/// How the canvas file is read and written.
pub type ReadLayout = Box<dyn Fn() -> Option<String> + Send>;
pub type WriteLayout = Box<dyn Fn(&str) -> Result<(), String> + Send>;

/// Where the host reads and writes what is outside the model, so a test can
/// point it at a temp folder.
pub struct HostIo {
    pub socket: PathBuf,
    pub join_socket: PathBuf,
    pub read_layout: ReadLayout,
    pub write_layout: WriteLayout,
    pub home: PathBuf,
}

impl HostIo {
    /// The real paths: this data dir's sockets and canvas file.
    pub fn real() -> HostIo {
        HostIo {
            socket: crate::paths::socket_path(),
            join_socket: crate::paths::join_socket(),
            read_layout: Box::new(crate::layout_file::read_layout),
            write_layout: Box::new(crate::layout_file::write_layout),
            home: crate::paths::home_dir(),
        }
    }
}

pub struct Host {
    pub model: Model,
    pub backend: Backend,
    registry: CommandRegistry<Model>,
    hub: JoinHub,
    updates: Option<Receiver<RemoteUpdate>>,
    /// A viewer's clicks on paths, resolved on this disk (#555).
    opens: Option<Receiver<crate::backend::join::OpenOnHost>>,
    io: HostIo,
    /// Sessions the daemon lists as running and no pane holds yet.
    live_sessions: Vec<String>,
    save_due: Option<f64>,
    last_agents: String,
    last_sweep: f64,
    /// The size each card's shell was last given, so a card resized in a
    /// viewer (a move on the canvas) refits its pty.
    sized: std::collections::HashMap<String, (u16, u16)>,
}

impl Host {
    /// Loads the canvas, starts the backend and the join server. A socket
    /// that cannot bind is an error here, unlike in the app: a host nobody can
    /// join has no job.
    pub fn start(io: HostIo) -> Result<Host, String> {
        let mut registry = CommandRegistry::new(|line| eprintln!("[infiniterm] {line}"));
        register_commands(&mut registry);
        registry.set_context(describe_context);

        let mut model = Model::new();
        let home = io.home.to_string_lossy().into_owned();
        model.home = home.clone();
        model.start_dir = home;
        model.tick(now_ms());

        // A missing or empty file is the defaults the model starts with; the
        // app writes its own on first launch, and a host with no app has
        // none of them.
        // The look keys an older settings.json holds move out first (#511),
        // the same move the app makes, so the text sent to viewers is clean.
        let _ = crate::config_files::migrate_look_keys();
        let settings = config_read(ConfigFile::Settings).unwrap_or_default();
        let keys = config_read(ConfigFile::Keybindings).unwrap_or_default();
        if !settings.trim().is_empty() {
            model.apply_settings_text(&settings);
        }
        if !keys.trim().is_empty() {
            model.apply_keymap_text(&keys);
        }

        // Always the daemon: the cards are its sessions, and the pty or tmux
        // backends have none a viewer could join.
        let buffer = model.config.terminal.session_buffer as usize;
        let backend = Backend::start(&io.socket, TerminalBackend::Daemon, buffer);
        if !backend.socket_ok {
            return Err(format!("could not bind {}", io.socket.display()));
        }
        model.can_park = backend.pty.can_park();
        let live_sessions = backend.pty.live_sessions();
        model.load_layout((io.read_layout)().as_deref());
        model.first_run = false;
        // No canvas yet (a Linux host has never had an app; a Mac may never
        // have saved one) but shells are running: each becomes a card, so a
        // viewer sees what is there instead of an empty canvas.
        if model.cards.is_empty() {
            seed_from_sessions(&mut model, &live_sessions);
        }
        // The zsh integration every new card's shell loads, as at the app's
        // launch.
        if let Err(e) = crate::shell_integration::install(&crate::paths::shell_integration_dir()) {
            eprintln!("[infiniterm/warn] could not write the zsh integration: {e}");
        }

        let (relay, write, nudge) = backend
            .pty
            .join_parts()
            .ok_or("the daemon backend is not available")?;
        let hub = JoinHub::new(relay, write).with_nudge(nudge);
        if !settings.trim().is_empty() {
            hub.publish_config(KIND_SETTINGS, &settings);
        }
        if !keys.trim().is_empty() {
            hub.publish_config(KIND_KEYBINDINGS, &keys);
        }
        hub.serve(&io.join_socket)
            .map_err(|e| format!("could not serve {}: {e}", io.join_socket.display()))?;
        let updates = hub.take_updates();
        let opens = hub.take_opens();

        let mut host = Host {
            model,
            backend,
            registry,
            hub,
            updates,
            opens,
            io,
            live_sessions,
            save_due: None,
            last_agents: String::new(),
            last_sweep: 0.,
            sized: Default::default(),
        };
        // Viewers that join before the first change still get a canvas.
        host.write_layout();
        Ok(host)
    }

    /// One turn: what the app's 16 ms poll and frame do, without a window.
    pub fn tick(&mut self, now: f64) {
        self.drain_backend();
        self.drain_config();
        self.drain_hooks();
        while let Ok(statuses) = self.backend.pane_status.try_recv() {
            self.model.apply_pane_statuses(&statuses);
        }
        self.drain_cli();
        self.model.tick(now);
        self.perform_effects();
        self.reconcile_terminals();
        if self.apply_remote_updates() | self.apply_remote_opens() {
            self.perform_effects();
            self.reconcile_terminals();
        }
        self.publish_agents();
        self.schedule_save(now);
        if now - self.last_sweep >= SWEEP_MS {
            self.last_sweep = now;
            self.model.sweep_stale();
        }
    }

    /// Saves now (a stop, a quit): anything in the last half second would
    /// otherwise be lost.
    pub fn flush(&mut self) {
        self.save_due = None;
        self.write_layout();
    }

    /// Leaves the sessions running and the join socket closed. The cards live
    /// on in their `iftd`s for the app (or the next host) to adopt.
    pub fn stop(&mut self) {
        self.flush();
        self.backend.pty.leave();
        let _ = std::fs::remove_file(&self.io.join_socket);
        // The hook socket too, so the app that takes over binds it at once.
        let _ = std::fs::remove_file(&self.io.socket);
    }

    fn drain_backend(&mut self) {
        while let Ok((pane, event)) = self.backend.pane_events.try_recv() {
            // The reader stalls at 256 KiB unacknowledged; with no grid to
            // parse, the bytes count as consumed the moment they arrive.
            match &event {
                PaneEvent::Output(bytes) | PaneEvent::Replay(bytes) => {
                    self.backend.pty.ack_now(pane, bytes.len());
                }
                _ => {}
            }
            self.model.apply_pane_event(pane, &event);
        }
    }

    /// The settings and keys, applied as the app applies them, and sent on to
    /// viewers (#433).
    fn drain_config(&mut self) {
        while let Ok(change) = self.backend.config_changes.try_recv() {
            match change.file {
                ConfigFile::Settings => {
                    self.model.apply_settings_text(&change.contents);
                    self.hub.publish_config(KIND_SETTINGS, &change.contents);
                }
                // A host with no window has no look to apply (#511).
                ConfigFile::UiTheme => {}
                _ => {
                    self.model.apply_keymap_text(&change.contents);
                    self.hub.publish_config(KIND_KEYBINDINGS, &change.contents);
                }
            }
        }
    }

    fn drain_hooks(&mut self) {
        while let Ok(report) = self.backend.hook_reports.try_recv() {
            self.model.apply_hook(&report);
        }
    }

    /// `ift` verbs on the host's socket, answered by the model. `ift read`
    /// needs a screen this host does not have and says so.
    fn drain_cli(&mut self) {
        while let Ok(req) = self.backend.cli_requests.try_recv() {
            let commands: Vec<(String, String)> = self
                .registry
                .all()
                .iter()
                .map(|c| (c.id.clone(), c.label.clone()))
                .collect();
            let commands: Vec<(&str, &str)> = commands
                .iter()
                .map(|(i, l)| (i.as_str(), l.as_str()))
                .collect();
            let reply = self.model.run_ift(&req, &commands);
            if !std::mem::take(&mut self.model.reply_deferred) {
                self.backend.cli.reply(req.id, reply.ok, reply.text);
            }
        }
    }

    fn perform_effects(&mut self) {
        for effect in self.model.take_effects() {
            match effect {
                Effect::KillPane(pane) => self.backend.pty.kill(pane),
                // A parked card reopened: the session keeps running and is
                // listed as live, so the card adopts it again.
                Effect::ReleasePane(pane) => {
                    let session = self.backend.pty.session_id(pane);
                    if self.backend.pty.release(pane) {
                        if let Some(s) = session {
                            self.live_sessions.push(s);
                        }
                    }
                }
                Effect::KillAllPanes => self.backend.pty.kill_all(),
                Effect::WritePane(pane, bytes) => self.backend.pty.write(pane, &bytes),
                Effect::CliReply { id, ok, text } => self.backend.cli.reply(id, ok, text),
                Effect::ReadCard { request_id, .. } => self.backend.cli.reply(
                    request_id,
                    false,
                    "this host has no window: there is no screen to read".into(),
                ),
                Effect::RunCommand(id) => {
                    run_with_effects(&mut self.model, &self.registry, &id);
                }
                Effect::Log(line) => eprintln!("[infiniterm] {line}"),
                Effect::Warn(line) => eprintln!("[infiniterm/warn] {line}"),
                // Everything else is about a window: see the module header.
                _ => {}
            }
        }
    }

    /// A shell for every terminal card that has none: its own session back
    /// when the daemon still has it, else a new one in the card's directory.
    fn reconcile_terminals(&mut self) {
        use crate::saved_layout::CardKind;
        let shell = Some(self.model.config.terminal.shell.clone()).filter(|s| !s.is_empty());
        let extra_env: Vec<(String, String)> = self
            .model
            .config
            .terminal
            .env
            .iter()
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.to_string()))
            .collect();
        let cards: Vec<_> = self
            .model
            .cards
            .iter()
            .filter(|c| c.kind == CardKind::Terminal && c.error.is_none())
            .cloned()
            .collect();
        for card in cards {
            let (cols, rows) = nominal_cells(card.rect.w, card.rect.h, &self.model.config.terminal);
            if card.pane_id.is_none() {
                let adopted = card
                    .session
                    .clone()
                    .filter(|s| self.live_sessions.iter().any(|live| live == s))
                    .and_then(|s| self.backend.pty.adopt(&s));
                let pane = match adopted {
                    Some(pane) => Ok(pane),
                    None => {
                        let history = crate::shell_history::history_file(&card.id);
                        let _ = crate::shell_history::ensure_file(&history);
                        let command = card.command.clone().or_else(|| shell.clone());
                        let mut env = extra_env.clone();
                        env.extend([
                            ("INFINITERM_CARD_ID".to_string(), card.id.clone()),
                            (
                                "INFINITERM_HISTFILE".to_string(),
                                history.to_string_lossy().to_string(),
                            ),
                        ]);
                        // Command marks for a shell card, as the app does.
                        let integration = crate::paths::shell_integration_dir();
                        if card.command.is_none() && integration.join(".zshenv").exists() {
                            let sh = shell
                                .clone()
                                .unwrap_or_else(crate::shell_cmd::default_shell);
                            env.extend(crate::shell_integration::zsh_env(
                                &sh,
                                &integration,
                                std::env::var("ZDOTDIR").ok(),
                            ));
                        }
                        self.backend
                            .pty
                            .spawn_now(Path::new(&card.cwd), command.as_deref(), env)
                    }
                };
                match pane {
                    Ok(pane) => {
                        self.backend.pty.resize_now(pane, cols, rows);
                        self.sized.insert(card.id.clone(), (cols, rows));
                        if let Some(c) = self.model.card_mut(&card.id) {
                            c.pane_id = Some(pane);
                        }
                    }
                    Err(e) => {
                        let text = format!("could not start a shell in {}: {e}", card.cwd);
                        if let Some(c) = self.model.card_mut(&card.id) {
                            c.error = Some(text);
                        }
                    }
                }
            } else if let Some(pane) = card.pane_id {
                // A card resized in a viewer: its shell follows.
                if self.sized.get(&card.id) != Some(&(cols, rows)) {
                    self.backend.pty.resize_now(pane, cols, rows);
                    self.sized.insert(card.id.clone(), (cols, rows));
                }
            }
            // The session the backend calls this card's, learned a moment
            // after the card was made. Saved, so the next start finds it.
            if let Some(pane) = card.pane_id {
                let session = self.backend.pty.session_id(pane);
                if session.is_some() && card.session != session {
                    if let Some(c) = self.model.card_mut(&card.id) {
                        c.session = session;
                        self.model.dirty_layout = true;
                    }
                }
            }
        }
    }

    /// What viewers changed on this canvas: the model's own merge. An edit on
    /// a canvas it no longer remembers makes the host republish, which is how
    /// that viewer finds out. True when the canvas changed.
    fn apply_remote_updates(&mut self) -> bool {
        let Some(rx) = &self.updates else {
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

    /// A viewer's click on a path, already found on this disk: opened beside
    /// the card it came from, as a Cmd+click in this canvas's own window is
    /// (the model's `open_in_card`). True when a card was made.
    fn apply_remote_opens(&mut self) -> bool {
        let Some(rx) = &self.opens else {
            return false;
        };
        let opens: Vec<_> = rx.try_iter().collect();
        let mut changed = false;
        for o in opens {
            let kind = if o.directory {
                crate::ift::PathKind::Directory
            } else {
                crate::ift::PathKind::File
            };
            let plan = crate::ift::open_plan(&o.path, kind, o.line.map(|l| l as f64));
            changed |= self.model.open_in_card(plan, Some(&o.card)).is_some();
        }
        changed
    }

    fn publish_agents(&mut self) {
        let text = self.model.agent_states_text();
        if text != self.last_agents {
            self.hub.publish_agents(&text);
            self.last_agents = text;
        }
    }

    fn schedule_save(&mut self, now: f64) {
        if self.model.dirty_layout && self.save_due.is_none() {
            self.save_due = Some(now + SAVE_DEBOUNCE_MS);
        }
        if self.save_due.is_some_and(|due| now >= due) {
            self.save_due = None;
            self.write_layout();
        }
    }

    /// The canvas to its file and to every viewer, with its revision.
    fn write_layout(&mut self) {
        if let Some(text) = self.model.save_text() {
            if let Err(e) = (self.io.write_layout)(&text) {
                eprintln!("[infiniterm] could not save the layout: {e}");
            }
            let text = self.model.publish_text().unwrap_or(text);
            let rev = self.model.record_published(&text);
            self.hub.publish_layout(&text, rev);
        }
    }
}

/// One card for every running session, oldest first, each in the folder its
/// daemon was started in (its meta file), else the host's home. The card keeps
/// the session id, so the first tick adopts the shell instead of starting one.
/// Cards are laid out by the model's own placement (the block grid).
pub fn seed_from_sessions(model: &mut Model, sessions: &[String]) {
    let dir = crate::paths::sessions_dir();
    let mut found: Vec<(String, String, String)> = sessions
        .iter()
        .map(|id| {
            let (cwd, started) = read_meta(&dir, id);
            (started, id.clone(), cwd)
        })
        .collect();
    found.sort();
    for (_, id, cwd) in found {
        let cwd = if cwd.is_empty() || !Path::new(&cwd).is_dir() {
            model.home.clone()
        } else {
            cwd
        };
        let ws = model.active_workspace.clone().unwrap_or_default();
        let card = model.add_card(
            &cwd,
            crate::model::NewCard {
                avoid: model.other_frames(None, &ws),
                workspace_id: Some(ws),
                ..Default::default()
            },
        );
        if let Some(c) = model.card_mut(&card) {
            c.session = Some(id);
        }
    }
    model.dirty_layout = true;
}

/// The folder and start time in a session's meta file; empty when it cannot
/// be read (a daemon that has not written it yet).
fn read_meta(dir: &Path, id: &str) -> (String, String) {
    let text = std::fs::read_to_string(dir.join(format!("{id}.meta"))).unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let get = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    (get("cwd"), get("started"))
}

/// A card's columns and rows with no font to measure: the nominal cell is
/// 0.6 of the font size wide (what a monospace face is) and the line height
/// setting tall, less the padding. The app measures the real font; a viewer
/// draws the host's cells, so a host started headless is a little off the
/// app's grid until the app takes the cards back and refits them.
pub fn nominal_cells(w: f64, h: f64, t: &crate::config::Terminal) -> (u16, u16) {
    let cell_w = (t.font_size * 0.6).max(1.);
    let cell_h = (t.font_size * t.line_height).max(1.);
    let cols = ((w - 2. * t.padding) / cell_w).floor();
    let rows = ((h - 2. * t.padding) / cell_h).floor();
    (cols.clamp(20., 500.) as u16, rows.clamp(5., 200.) as u16)
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.)
        .unwrap_or(0.)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_ift_serve_command_line_is_a_serve() {
        assert!(is_serve_command(
            "/Applications/infiniterm.app/Contents/MacOS/ift serve"
        ));
        assert!(is_serve_command("ift serve --detach"));
        assert!(!is_serve_command("/usr/bin/ssh host ift serve"));
        assert!(!is_serve_command(
            "/Applications/infiniterm.app/Contents/MacOS/ift sessions"
        ));
        assert!(!is_serve_command(
            "/Applications/infiniterm.app/Contents/MacOS/infiniterm"
        ));
        assert!(!is_serve_command(""));
    }

    #[test]
    fn a_stale_pid_file_never_names_a_process_to_signal() {
        let dir = std::env::temp_dir().join(format!("hl-pid-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("serve.pid");
        // This very test process is a live pid and not an `ift serve`.
        std::fs::write(&file, std::process::id().to_string()).unwrap();
        assert_eq!(live_serve(&file), None);
        std::fs::write(&file, "not a number").unwrap();
        assert_eq!(live_serve(&file), None);
        std::fs::write(&file, "4000000000").unwrap(); // no such process
        assert_eq!(live_serve(&file), None);
        assert_eq!(live_serve(&dir.join("missing")), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_card_with_no_font_to_measure_still_gets_a_sane_grid() {
        let t = crate::config::default_config().terminal;
        let (c, r) = nominal_cells(1725., 1000., &t);
        assert!((20..=500).contains(&c) && (5..=200).contains(&r), "{c}x{r}");
        // Tiny and huge cards are clamped, never zero and never absurd.
        assert_eq!(nominal_cells(1., 1., &t), (20, 5));
        assert_eq!(nominal_cells(1e6, 1e6, &t), (500, 200));
    }
}
