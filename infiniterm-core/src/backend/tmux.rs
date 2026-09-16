//! Cards backed by tmux, so the shells outlive the window.
//!
//! One `tmux -C` client on the DEFAULT socket, attached to one session named
//! `infiniterm`. That is the whole argument for tmux over a daemon of our
//! own: `tmux attach -t infiniterm` from any terminal reaches the same
//! sessions, and the rest of somebody's tooling sees them too.
//!
//! A card is a tmux WINDOW holding one pane, never a pane inside a shared
//! window: panes tile inside a window and would have to share its size,
//! while `refresh-client -C '@0:100x30'` gives every window its own. Proved
//! in spikes/tmux/NOTES.md against tmux 3.7c.
//!
//! tmux draws nothing here. It reports bytes, our emulator renders them, and
//! scrollback, selection and the mouse stay exactly what they already are.
//!
//! ADDRESS BY ID, never by index or name: `%0` a pane, `@0` a window. An
//! index is somebody's `base-index` and a name is their `automatic-rename`.
//! An afternoon went into learning that; see the spike notes.
//!
//! Related: tmux_protocol.rs (the reading), local_pty.rs (the same surface
//! without the persistence), backend/mod.rs for `SessionBackend`.
use super::{PaneEvent, PaneId, SessionBackend};
use crate::backend::tmux_protocol::{Notice, Reader};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// The session every infiniterm window lives in. One name, so a second
/// launch attaches rather than growing a second session.
pub const SESSION: &str = "infiniterm";
/// What a scratch instance uses instead. A driver run or a `make run` on a
/// copied canvas must not attach to, resize or kill the windows the real app
/// is holding: the same reason the driver addresses the app by pid and the
/// scenarios scope `ift` to their own data dir.
pub const DEV_SESSION: &str = "infiniterm-dev";

/// The session this instance may touch.
pub fn session_name() -> &'static str {
    if crate::paths::data_dir_overridden() {
        DEV_SESSION
    } else {
        SESSION
    }
}

/// What tmux is asked for when a card has no size yet. The real size follows
/// within a frame, but a shell that starts at 80x24 and is corrected is
/// better than one that starts at 0.
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;

/// A card's window and pane, as tmux names them.
#[derive(Clone, Debug)]
struct Window {
    /// `@7`
    id: String,
    /// `%7`. Learned from the first `%output` for the window, because tmux
    /// reports output by PANE and commands take either.
    pane: Option<String>,
}

pub struct TmuxBackend {
    /// Our pane ids to tmux's window. The model never sees a tmux id.
    windows: Arc<Mutex<HashMap<PaneId, Window>>>,
    stdin: Arc<Mutex<ChildStdin>>,
    child: Arc<Mutex<Child>>,
    next_id: AtomicU32,
    /// Panes whose `new-window` has been sent and whose window id has not
    /// come back yet, oldest first. tmux answers commands in order, so the
    /// first `@N` reply belongs to the first pane still waiting.
    awaiting_window: Arc<Mutex<VecDeque<PaneId>>>,
}

/// Is there a tmux to talk to at all? The backend falls back to local PTYs
/// when there is not: a missing binary must not mean no terminal.
pub fn available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// `@7` and nothing else: a reply line that is a window id.
fn is_window_id(line: &str) -> bool {
    line.strip_prefix('@')
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

impl TmuxBackend {
    /// Attaches to the session, creating it if it is not there (`-A`).
    /// `None` when tmux will not start, and the caller uses local PTYs.
    pub fn start() -> Option<(TmuxBackend, Receiver<(PaneId, PaneEvent)>)> {
        let mut child = Command::new("tmux")
            .args([
                // A test points this at a socket of its own; the app leaves
                // it unset and uses the default, which is the whole argument
                // for tmux: `tmux attach -t infiniterm` from any terminal.
                "-L",
                &std::env::var("INFINITERM_TMUX_SOCKET").unwrap_or_else(|_| "default".into()),
                "-C",
                "new-session",
                "-A",
                "-s",
                session_name(),
                "-x",
                &INITIAL_COLS.to_string(),
                "-y",
                &INITIAL_ROWS.to_string(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        let (tx, rx) = channel();
        let backend = TmuxBackend {
            windows: Arc::new(Mutex::new(HashMap::new())),
            stdin: Arc::new(Mutex::new(stdin)),
            child: Arc::new(Mutex::new(child)),
            next_id: AtomicU32::new(1),
            awaiting_window: Arc::new(Mutex::new(VecDeque::new())),
        };
        // The status line is a row of the card, not decoration: without this
        // every pane is one row shorter than the card it fills.
        backend.command("set -g status off");
        // Ours, and only ours: a window somebody made from another terminal
        // is theirs and we do not adopt it.
        backend.command("set -g allow-rename off");
        backend.read_thread(stdout, tx);
        Some((backend, rx))
    }

    /// The reader: one thread, one line at a time, turning notices into pane
    /// events on the same channel the local backend uses.
    fn read_thread(&self, stdout: std::process::ChildStdout, tx: Sender<(PaneId, PaneEvent)>) {
        let windows = self.windows.clone();
        let awaiting = self.awaiting_window.clone();
        std::thread::spawn(move || {
            let mut reader = Reader::new();
            let mut lines = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                // Bytes, not chars: a pane's output is arbitrary and a lone
                // invalid byte must not end the session.
                let mut raw = Vec::new();
                match lines.read_until(b'\n', &mut raw) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                while raw.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
                    raw.pop();
                }
                let text = String::from_utf8_lossy(&raw);
                match reader.line(&text) {
                    Notice::Output { pane, bytes } => {
                        let id = Self::id_for_pane(&windows, &pane);
                        if let Some(id) = id {
                            let _ = tx.send((id, PaneEvent::Output(bytes)));
                        }
                    }
                    Notice::WindowClose(window) => {
                        let gone: Vec<PaneId> = windows
                            .lock()
                            .unwrap()
                            .iter()
                            .filter(|(_, w)| w.id == window)
                            .map(|(id, _)| *id)
                            .collect();
                        for id in gone {
                            windows.lock().unwrap().remove(&id);
                            let _ = tx.send((id, PaneEvent::Exited { code: 0 }));
                        }
                    }
                    // The window id for a `new-window` we sent. tmux answers
                    // in order, so it belongs to the oldest pane still
                    // waiting for one. Anything else in a reply block is a
                    // command we did not ask about.
                    Notice::Reply(text) if is_window_id(&text) => {
                        if let Some(id) = awaiting.lock().unwrap().pop_front() {
                            if let Some(w) = windows.lock().unwrap().get_mut(&id) {
                                w.id = text;
                            }
                        }
                    }
                    // tmux is going away: every card's shell went with it.
                    Notice::Exit(_) => {
                        let all: Vec<PaneId> = windows.lock().unwrap().keys().copied().collect();
                        for id in all {
                            let _ = tx.send((id, PaneEvent::Exited { code: 0 }));
                        }
                        break;
                    }
                    _ => {}
                }
            }
        });
    }

    /// tmux reports output by PANE and we hold windows, so the first line a
    /// window produces teaches us its pane id. `list-panes` would be a round
    /// trip per window for something the output itself carries.
    fn id_for_pane(windows: &Arc<Mutex<HashMap<PaneId, Window>>>, pane: &str) -> Option<PaneId> {
        let mut map = windows.lock().unwrap();
        if let Some((id, _)) = map.iter().find(|(_, w)| w.pane.as_deref() == Some(pane)) {
            return Some(*id);
        }
        // A pane we have not seen: it belongs to the window we most recently
        // made and have no pane for. Anything else is somebody else's window
        // and is not ours to adopt.
        let waiting = map
            .iter()
            .find(|(_, w)| w.pane.is_none())
            .map(|(id, _)| *id)?;
        if let Some(w) = map.get_mut(&waiting) {
            w.pane = Some(pane.to_string());
        }
        Some(waiting)
    }

    /// One command, one line. tmux answers asynchronously and we do not wait:
    /// the answers that matter arrive as notices.
    fn command(&self, line: &str) {
        if let Ok(mut stdin) = self.stdin.lock() {
            let _ = writeln!(stdin, "{line}");
            let _ = stdin.flush();
        }
    }

    fn window_of(&self, pane: PaneId) -> Option<Window> {
        self.windows.lock().unwrap().get(&pane).cloned()
    }

    /// The tmux target for a pane: its pane id once known, else its window.
    fn target(&self, pane: PaneId) -> Option<String> {
        self.window_of(pane).map(|w| w.pane.unwrap_or(w.id))
    }

    /// Everything tmux holds for this app, ended. Not `kill-server`: the
    /// user's own sessions live on the same socket.
    pub fn kill_all(&self) {
        self.command(&format!("kill-session -t {}", session_name()));
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }

    /// Detaches without killing anything: the whole point. Called when the
    /// app quits, so the shells keep running.
    pub fn detach(&self) {
        self.command("detach-client");
        if let Ok(mut child) = self.child.lock() {
            let _ = child.wait();
        }
    }
}

impl TmuxBackend {
    /// The same as the trait's `spawn`, without the async: nothing here
    /// waits, and the ui calls it from a frame.
    pub fn spawn_now(
        &self,
        cwd: &Path,
        cmd: Option<&str>,
        env: Vec<(String, String)>,
    ) -> anyhow::Result<PaneId> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        // The environment reaches the pane through tmux's own, set for this
        // window only: a tmux window inherits the SERVER's environment, not
        // this client's, so INFINITERM_CARD_ID has to be handed over
        // explicitly or the hooks would report the wrong card.
        let mut prefix = String::new();
        for (key, value) in super::local_pty::terminal_identity().into_iter().chain(env) {
            prefix.push_str(&format!(
                "set-environment -t {} {} {} ; ",
                session_name(),
                key,
                crate::drop::shell_quote(&value)
            ));
        }
        let start = cmd
            .map(|c| format!(" {}", crate::drop::shell_quote(c)))
            .unwrap_or_default();
        self.command(&format!(
            "{prefix}new-window -d -P -F '#{{window_id}}' -c {}{}",
            crate::drop::shell_quote(&cwd.to_string_lossy()),
            start
        ));
        // The window id arrives in the command's reply and the pane id in the
        // window's first output. Neither is waited on: a command issued
        // before they land targets nothing, which tmux ignores, and the
        // ui reissues size on the next frame anyway.
        self.awaiting_window.lock().unwrap().push_back(id);
        self.windows.lock().unwrap().insert(
            id,
            Window {
                id: String::new(),
                pane: None,
            },
        );
        Ok(id)
    }

    pub fn write_now(&self, pane: PaneId, bytes: &[u8]) {
        let Some(target) = self.target(pane) else {
            return;
        };
        // send-keys -H takes hex, which is the only encoding that survives
        // arbitrary bytes: a paste can hold anything, including the
        // semicolon that would otherwise end the command.
        let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
        if hex.is_empty() {
            return;
        }
        self.command(&format!("send-keys -H -t {target} {}", hex.join(" ")));
    }

    pub fn resize_now(&self, pane: PaneId, cols: u16, rows: u16) {
        let Some(window) = self.window_of(pane).filter(|w| !w.id.is_empty()) else {
            return;
        };
        // Per WINDOW, which is what lets two cards be two sizes.
        self.command(&format!(
            "refresh-client -C '{}:{}x{}'",
            window.id, cols, rows
        ));
    }

    pub fn kill_now(&self, pane: PaneId) {
        let Some(window) = self.window_of(pane) else {
            return;
        };
        self.windows.lock().unwrap().remove(&pane);
        if !window.id.is_empty() {
            self.command(&format!("kill-window -t {}", window.id));
        }
    }

    /// Flow control is tmux's `refresh-client -A`, driven by the ledger in
    /// the ui rather than by a byte count here; see `pause` and `unpause`.
    pub fn ack_now(&self, _pane: PaneId, _bytes: usize) {}
}

impl SessionBackend for TmuxBackend {
    async fn spawn(
        &self,
        cwd: &Path,
        cmd: Option<&str>,
        env: Vec<(String, String)>,
    ) -> anyhow::Result<PaneId> {
        self.spawn_now(cwd, cmd, env)
    }

    fn write(&self, pane: PaneId, bytes: &[u8]) {
        self.write_now(pane, bytes)
    }

    fn resize(&self, pane: PaneId, cols: u16, rows: u16) {
        self.resize_now(pane, cols, rows)
    }

    fn kill(&self, pane: PaneId) {
        self.kill_now(pane)
    }

    fn ack(&self, pane: PaneId, bytes: usize) {
        self.ack_now(pane, bytes)
    }
}

impl TmuxBackend {
    /// Stops tmux sending this pane's output. Measured at exactly zero lines
    /// per second while paused, against 18,700 unpaused.
    pub fn pause(&self, pane: PaneId) {
        if let Some(target) = self.target(pane) {
            self.command(&format!("refresh-client -A '{target}:pause'"));
        }
    }

    pub fn unpause(&self, pane: PaneId) {
        if let Some(target) = self.target(pane) {
            self.command(&format!("refresh-client -A '{target}:continue'"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every command this backend sends addresses tmux by id. An index is
    // somebody's base-index and a name is their automatic-rename; the spike
    // lost an afternoon to `spike:0` silently matching nothing.
    // A scratch instance gets its own session, or a driver run would resize
    // and kill the windows the real app is holding.
    #[test]
    fn a_scratch_instance_never_touches_the_real_session() {
        assert_ne!(SESSION, DEV_SESSION);
        // Whichever this process is, it is one of the two and nothing else.
        assert!(matches!(session_name(), SESSION | DEV_SESSION));
    }

    #[test]
    fn the_session_is_named_and_the_targets_are_ids() {
        assert_eq!(SESSION, "infiniterm");
        let source = include_str!("tmux.rs");
        for line in source.lines() {
            let command = line.trim();
            if !command.starts_with("self.command(") && !command.contains("format!(") {
                continue;
            }
            assert!(
                !command.contains(":0\"") && !command.contains(":0 "),
                "a target by index crept in: {command}"
            );
        }
    }

    #[test]
    fn a_window_id_is_told_from_any_other_reply() {
        assert!(is_window_id("@0"));
        assert!(is_window_id("@17"));
        assert!(!is_window_id("@"));
        assert!(!is_window_id("@0 80x24"), "a list-windows row is not an id");
        assert!(!is_window_id("%0"), "that is a pane");
        assert!(!is_window_id("no such window"));
    }

    /// A tmux on a socket of its own, killed when the test ends. Nothing
    /// here can see, resize or kill a real session.
    struct Sandbox(String);

    impl Sandbox {
        fn new(name: &str) -> Option<Sandbox> {
            if !available() {
                return None;
            }
            let socket = format!("infiniterm-test-{name}");
            std::env::set_var("INFINITERM_TMUX_SOCKET", &socket);
            let _ = Command::new("tmux")
                .args(["-L", &socket, "kill-server"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            Some(Sandbox(socket))
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .args(["-L", &self.0, "kill-server"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }

    fn drain(rx: &Receiver<(PaneId, PaneEvent)>, seconds: f64) -> Vec<u8> {
        let end = std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds);
        let mut out = vec![];
        while std::time::Instant::now() < end {
            while let Ok((_, event)) = rx.try_recv() {
                if let PaneEvent::Output(bytes) = event {
                    out.extend(bytes);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        out
    }

    // The whole backend against a real tmux: a window is made, what is
    // written to it runs, and what it prints comes back as pane output on
    // the same channel a local pty would use.
    #[test]
    fn a_window_runs_what_is_written_to_it_and_reports_back() {
        let Some(_sandbox) = Sandbox::new("roundtrip") else {
            return; // no tmux here; the pure tests still cover the protocol
        };
        let Some((backend, rx)) = TmuxBackend::start() else {
            panic!("tmux is available but would not start");
        };
        let pane = futures_lite_block(backend.spawn(
            Path::new("/tmp"),
            None,
            vec![("INFINITERM_CARD_ID".into(), "test-card".into())],
        ))
        .expect("a window");
        // The shell has to be up before it can be typed at.
        drain(&rx, 1.5);
        backend.write(pane, b"echo the-roundtrip-worked\n");
        let seen = String::from_utf8_lossy(&drain(&rx, 2.5)).to_string();
        assert!(
            seen.contains("the-roundtrip-worked"),
            "the pane never echoed it back; saw {seen:?}"
        );
        backend.kill_all();
    }

    /// The trait is async only because tmux assigns ids asynchronously; this
    /// one never yields, so a two-line executor is enough for a test.
    fn futures_lite_block<T>(future: impl std::future::Future<Output = T>) -> T {
        let mut future = Box::pin(future);
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        loop {
            if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }
        }
    }

    #[test]
    fn bytes_go_out_as_hex_so_anything_survives() {
        // The encoding matters more than it looks: a semicolon in a paste
        // would otherwise end the tmux command and run the rest.
        let bytes = b"a;b\x1b\n";
        let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex.join(" "), "61 3b 62 1b 0a");
    }
}
