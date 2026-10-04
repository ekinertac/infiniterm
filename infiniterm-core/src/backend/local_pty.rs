//! portable-pty reader handles are BLOCKING, so each pane gets a dedicated OS
//! thread that funnels into one shared mpsc Sender. N producer threads, one
//! consumer — which is exactly the shape the tagged-stream contract requires.
//!
//! Each reader is on a CREDIT: it stops reading once `HIGH_WATER` bytes of its
//! pane's output are sent but not yet acknowledged by the consumer, and resumes
//! when acks bring that back down. Without it the whole path was a queue with
//! no bound: a pane running `yes` produced faster than the webview could parse,
//! the mpsc channel and the IPC channel behind it grew without limit, and the
//! app fell from 60fps to 20 with one such pane and to 0.2 with twenty-five.
//! Blocking the READ is what makes the child itself wait, because the kernel's
//! pty buffer is finite — this is the same mechanism a slow physical terminal
//! has always applied to a fast program.
//!
//! Moved from the Tauri app unchanged. `HIGH_WATER` here bounds memory; the
//! per-frame parse budget in `infiniterm-term::scheduler` bounds time, and
//! both are needed (the term-zoom spike measured 1 fps without the budget).
//!
//! `INHERITED_TERMINAL_VARS`, `terminal_identity` and `default_shell` are
//! `pub`: `infiniterm-session`'s `iftd` spawns a card's shell now instead of
//! this process, so the env scrubbing has to live here and be used from
//! there too, or a daemon-backed card claims to be whatever terminal
//! launched the app.

use super::{PaneEvent, PaneId, SessionBackend};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

/// Unacknowledged bytes a pane may have in flight before its reader waits.
///
/// Sized against the consumer, not the producer: with every pane flooding, the
/// webview holds at most this much per pane un-parsed, so 25 panes is a bounded
/// ~6 MiB rather than whatever the last second produced. Large enough that a
/// burst — a screenful of build output — never waits on a round trip.
pub const HIGH_WATER: usize = 256 * 1024;

/// What the reader thread waits on. `closed` is how a kill reaches a reader that
/// is parked here rather than in `read()`, where the pty going away would have
/// woken it.
#[derive(Default)]
struct Credit {
    state: Mutex<CreditState>,
    changed: Condvar,
}

#[derive(Default)]
struct CreditState {
    in_flight: usize,
    closed: bool,
}

impl Credit {
    /// Books `n` bytes just sent. Called by the reader alone.
    fn sent(&self, n: usize) {
        self.state.lock().unwrap().in_flight += n;
    }

    /// Blocks until the pane is under the mark or has been closed. Returns
    /// false when closed, so the reader can stop.
    fn wait_for_room(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        while s.in_flight > HIGH_WATER && !s.closed {
            s = self.changed.wait(s).unwrap();
        }
        !s.closed
    }

    fn ack(&self, n: usize) {
        let mut s = self.state.lock().unwrap();
        s.in_flight = s.in_flight.saturating_sub(n);
        self.changed.notify_one();
    }

    fn close(&self) {
        self.state.lock().unwrap().closed = true;
        self.changed.notify_one();
    }
}

struct Pane {
    master: Box<dyn MasterPty + Send>,
    // Wrapped separately from `panes` so a write that blocks (child not
    // reading stdin, e.g. a large paste) only stalls this one pane, not
    // every card's write/resize/kill/spawn via the shared map lock.
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    // A detached handle to signal the child independently of the thread
    // blocked in `child.wait()`. This is what actually terminates the
    // child on kill() — dropping `master` alone does not: the reader
    // thread holds its own dup'd fd to the master via try_clone_reader(),
    // so the pty is never fully closed and no HUP is generated.
    killer: Box<dyn ChildKiller + Send + Sync>,
    // The shell's own pid, used to find what it is running — an ssh session, say.
    // Optional because portable_pty does not promise one on every platform.
    pid: Option<u32>,
    credit: Arc<Credit>,
}

pub struct LocalPtyBackend {
    panes: Arc<Mutex<HashMap<PaneId, Pane>>>,
    tx: Sender<(PaneId, PaneEvent)>,
    next_id: AtomicU32,
}

impl LocalPtyBackend {
    /// A handle to the live pane pids that outlives any borrow of the backend.
    ///
    /// The remote-session poller runs on its own thread and cannot hold a `State`
    /// borrow, which is not `'static`; the pane map behind this is already an Arc,
    /// so handing out a closure over it costs nothing.
    ///
    /// Deliberately NOT on `SessionBackend`: a pid is not universally answerable.
    /// A tmux pane's process lives wherever the server does, which may be another
    /// machine, so v2 answers "is this remote" from tmux itself rather than by
    /// pretending to have a local pid.
    pub fn pids_source(&self) -> impl Fn() -> Vec<(PaneId, u32)> + Send + 'static {
        let panes = self.panes.clone();
        move || {
            panes
                .lock()
                .unwrap()
                .iter()
                .filter_map(|(id, p)| p.pid.map(|pid| (*id, pid)))
                .collect()
        }
    }

    pub fn new() -> (Self, Receiver<(PaneId, PaneEvent)>) {
        let (tx, rx) = channel();
        let backend = Self {
            panes: Arc::new(Mutex::new(HashMap::new())),
            tx,
            next_id: AtomicU32::new(1),
        };
        (backend, rx)
    }
}

impl LocalPtyBackend {
    /// Kills every pane. Used when the webview reloads: the frontend that owned
    /// those cards is gone, so the shells would otherwise linger with nothing
    /// reading them. Inherent rather than on the trait — v2's tmux backend ends a
    /// whole session differently.
    pub fn kill_all(&self) {
        let ids: Vec<PaneId> = self.panes.lock().unwrap().keys().copied().collect();
        for id in ids {
            self.kill(id);
        }
    }
}

impl SessionBackend for LocalPtyBackend {
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

/// The same operations as inherent, synchronous methods: a local PTY
/// answers at once, and the ui thread (gpui's, with no executor to block
/// on) calls these directly. The trait keeps its async shape for v2's tmux
/// backend; both paths are the one implementation.
impl LocalPtyBackend {
    pub fn spawn_now(
        &self,
        cwd: &Path,
        cmd: Option<&str>,
        env: Vec<(String, String)>,
    ) -> anyhow::Result<PaneId> {
        let pair = native_pty_system().openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut builder = match cmd {
            // -lc so the user's profile is loaded, matching what a terminal app does.
            Some(c) => {
                let mut b = CommandBuilder::new(default_shell());
                b.args(["-lc", c]);
                b
            }
            None => CommandBuilder::new(default_shell()),
        };
        builder.cwd(cwd);
        // The shell must know it is in infiniterm, not in whatever launched
        // the app. Launched from a terminal (`open`, `ift`), the app inherits
        // that terminal's identity variables and every card claimed to be a
        // WezTerm pane; fastfetch, tmux and shell prompts read these.
        for k in INHERITED_TERMINAL_VARS {
            builder.env_remove(k);
        }
        for (k, v) in terminal_identity() {
            builder.env(k, v);
        }
        for (k, v) in env {
            builder.env(k, v);
        }

        let mut child = pair.slave.spawn_command(builder)?;
        // Cloned before `child` moves into the exit-watcher thread below, so
        // kill() can still signal the child from another thread.
        let killer = child.clone_killer();
        // Read before `child` moves into the exit watcher, like the killer above.
        let pid = child.process_id();
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let credit = Arc::new(Credit::default());

        self.panes.lock().unwrap().insert(
            id,
            Pane {
                master: pair.master,
                writer: Arc::new(Mutex::new(writer)),
                killer,
                pid,
                credit: credit.clone(),
            },
        );

        // Reader thread: blocking reads, tagged sends, and a wait for credit
        // between them — see the module header.
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 8192];
            loop {
                if !credit.wait_for_room() {
                    break;
                }
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    // A signal delivered to this thread (e.g. during the kill()
                    // path) can interrupt the blocking read without the pty
                    // actually being at EOF. Retry rather than treating it as
                    // closed, or a killed pane would look identical to a
                    // healthy one that simply hasn't produced output yet.
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                    Ok(n) => {
                        credit.sent(n);
                        if tx.send((id, PaneEvent::Output(buf[..n].to_vec()))).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        // Exit watcher, separate thread so a long-lived shell never blocks reads.
        let tx_exit = self.tx.clone();
        let panes = self.panes.clone();
        std::thread::spawn(move || {
            let code = child.wait().map(|s| s.exit_code() as i32).unwrap_or(-1);
            if let Some(p) = panes.lock().unwrap().remove(&id) {
                // Frees a reader parked on credit; nothing will ack a dead pane.
                p.credit.close();
            }
            let _ = tx_exit.send((id, PaneEvent::Exited { code }));
        });

        Ok(id)
    }

    pub fn write_now(&self, pane: PaneId, bytes: &[u8]) {
        // Clone the per-pane writer handle and drop the shared map lock
        // before writing. write_all can block indefinitely (a paste into a
        // pane whose child isn't reading stdin), and every other method
        // shares this same map lock, so holding it across the write would
        // freeze pane management for every other card.
        let writer = self
            .panes
            .lock()
            .unwrap()
            .get(&pane)
            .map(|p| p.writer.clone());
        if let Some(writer) = writer {
            let mut w = writer.lock().unwrap();
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }
    }

    pub fn resize_now(&self, pane: PaneId, cols: u16, rows: u16) {
        if let Some(p) = self.panes.lock().unwrap().get(&pane) {
            let _ = p.master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    pub fn kill_now(&self, pane: PaneId) {
        // Signal the child directly. Dropping `master` here does NOT stop
        // the child: the reader thread holds its own dup'd fd to the master
        // (from try_clone_reader() in spawn), so the pty never fully closes
        // and no HUP reaches the child — it, and the reader and exit-watcher
        // threads blocked on it, would run forever. The pane is removed from
        // the map by the exit watcher once `child.wait()` actually returns,
        // not here, so `Exited` is still emitted exactly once.
        if let Some(p) = self.panes.lock().unwrap().get_mut(&pane) {
            let _ = p.killer.kill();
            // The reader may be parked on credit rather than in read(), where
            // the kill would have reached it; wake it so the thread can end.
            p.credit.close();
        }
    }

    pub fn ack_now(&self, pane: PaneId, bytes: usize) {
        // Cloned out so the map lock is not held while notifying.
        let credit = self
            .panes
            .lock()
            .unwrap()
            .get(&pane)
            .map(|p| p.credit.clone());
        if let Some(credit) = credit {
            credit.ack(bytes);
        }
    }
}

/// Variables a parent terminal leaves in the environment; a shell reading
/// them would take that terminal for its own.
///
/// `pub`: `iftd` (infiniterm-session) spawns the child now, so the scrubbing
/// lives here and is used from there too. Without it every card claims to be
/// a WezTerm pane again once a card's shell is a daemon's child rather than
/// this process's — see the WezTerm trap in CLAUDE.md.
pub const INHERITED_TERMINAL_VARS: [&str; 12] = [
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "TERM_SESSION_ID",
    "WEZTERM_PANE",
    "WEZTERM_UNIX_SOCKET",
    "WEZTERM_EXECUTABLE",
    "WEZTERM_EXECUTABLE_DIR",
    "WEZTERM_CONFIG_FILE",
    "WEZTERM_CONFIG_DIR",
    "ITERM_SESSION_ID",
    "ITERM_PROFILE",
    "TMUX",
];

/// What every card's shell is told about the terminal it runs in.
pub fn terminal_identity() -> Vec<(String, String)> {
    vec![
        ("TERM".into(), "xterm-256color".into()),
        ("COLORTERM".into(), "truecolor".into()),
        ("TERM_PROGRAM".into(), "infiniterm".into()),
        (
            "TERM_PROGRAM_VERSION".into(),
            env!("CARGO_PKG_VERSION").into(),
        ),
    ]
}

/// `pub`: `iftd` builds the same command line this backend does, and must
/// fall back to the same shell when `$SHELL` is unset.
pub fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Drains events for `pane` until `needle` is seen or the deadline passes.
    fn wait_for_output(
        rx: &std::sync::mpsc::Receiver<(PaneId, PaneEvent)>,
        pane: PaneId,
        needle: &str,
    ) -> bool {
        // Generous on purpose. These spawn the REAL login shell, so the
        // user's own profile runs first, and this suite now spawns shells
        // in several crates at once: five seconds was enough alone and
        // intermittently was not under a full `cargo test`. The deadline
        // only costs this long when a test is failing anyway.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut acc = Vec::new();
        while std::time::Instant::now() < deadline {
            if let Ok((p, PaneEvent::Output(bytes))) = rx.recv_timeout(Duration::from_millis(200)) {
                if p == pane {
                    acc.extend_from_slice(&bytes);
                    if String::from_utf8_lossy(&acc).contains(needle) {
                        return true;
                    }
                }
            }
        }
        false
    }

    #[tokio::test]
    async fn spawns_a_command_and_streams_its_output() {
        let (backend, rx) = LocalPtyBackend::new();
        let pane = backend
            .spawn(
                std::path::Path::new("/tmp"),
                Some("echo hello-infiniterm"),
                vec![],
            )
            .await
            .unwrap();
        assert!(wait_for_output(&rx, pane, "hello-infiniterm"));
    }

    #[tokio::test]
    async fn write_reaches_the_shell() {
        let (backend, rx) = LocalPtyBackend::new();
        let pane = backend
            .spawn(std::path::Path::new("/tmp"), None, vec![])
            .await
            .unwrap();
        backend.write(pane, b"echo written-ok\n");
        assert!(wait_for_output(&rx, pane, "written-ok"));
    }

    #[tokio::test]
    async fn env_vars_reach_the_child() {
        let (backend, rx) = LocalPtyBackend::new();
        let pane = backend
            .spawn(
                std::path::Path::new("/tmp"),
                Some("echo card=$INFINITERM_CARD_ID"),
                vec![("INFINITERM_CARD_ID".into(), "card-42".into())],
            )
            .await
            .unwrap();
        assert!(wait_for_output(&rx, pane, "card=card-42"));
    }

    // The app launched from a WezTerm shell must not hand WezTerm's identity
    // to every card.
    #[tokio::test]
    async fn the_shell_is_told_it_runs_in_infiniterm() {
        std::env::set_var("WEZTERM_PANE", "19");
        std::env::set_var("TERM_PROGRAM", "WezTerm");
        let (backend, rx) = LocalPtyBackend::new();
        let pane = backend
            .spawn(
                std::path::Path::new("/tmp"),
                Some("echo prog=$TERM_PROGRAM pane=${WEZTERM_PANE:-none}"),
                vec![],
            )
            .await
            .unwrap();
        assert!(wait_for_output(&rx, pane, "prog=infiniterm pane=none"));
    }

    #[tokio::test]
    async fn reports_exit() {
        let (backend, rx) = LocalPtyBackend::new();
        let pane = backend
            .spawn(std::path::Path::new("/tmp"), Some("exit 3"), vec![])
            .await
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut saw = None;
        while std::time::Instant::now() < deadline && saw.is_none() {
            if let Ok((p, PaneEvent::Exited { code })) = rx.recv_timeout(Duration::from_millis(200))
            {
                if p == pane {
                    saw = Some(code);
                }
            }
        }
        assert_eq!(saw, Some(3));
    }

    #[tokio::test]
    async fn resize_is_visible_to_the_child() {
        let (backend, rx) = LocalPtyBackend::new();
        // TERM is passed explicitly so `tput` has a valid terminfo entry
        // regardless of the ambient environment the test runs in — an unset
        // or unusual TERM would otherwise burn the deadline and fail flakily.
        let pane = backend
            .spawn(
                std::path::Path::new("/tmp"),
                None,
                vec![("TERM".into(), "xterm-256color".into())],
            )
            .await
            .unwrap();
        backend.resize(pane, 120, 40);
        // Marked, not a bare "120": the shell's own startup prints plenty
        // of numbers and a loose match could pass without the resize ever
        // having been seen.
        backend.write(pane, b"echo COLS=$(tput cols)\n");
        assert!(wait_for_output(&rx, pane, "COLS=120"));
    }

    #[tokio::test]
    async fn kill_terminates_the_child() {
        let (backend, rx) = LocalPtyBackend::new();
        let pane = backend
            .spawn(std::path::Path::new("/tmp"), Some("sleep 30"), vec![])
            .await
            .unwrap();
        backend.kill(pane);

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut exited = false;
        while std::time::Instant::now() < deadline && !exited {
            if let Ok((p, PaneEvent::Exited { .. })) = rx.recv_timeout(Duration::from_millis(200)) {
                if p == pane {
                    exited = true;
                }
            }
        }
        assert!(
            exited,
            "kill() did not produce an Exited event within the deadline"
        );
    }

    /// The backpressure contract: a flooding pane stops being read once
    /// HIGH_WATER bytes are unacknowledged, and resumes on ack.
    #[tokio::test]
    async fn a_flooding_pane_waits_for_acks() {
        let (backend, rx) = LocalPtyBackend::new();
        let pane = backend
            .spawn(std::path::Path::new("/tmp"), Some("yes"), vec![])
            .await
            .unwrap();

        // Drain without acking: the stream must stop by itself, somewhere just
        // past the mark (one read of slack, since the check precedes the read).
        // The idle count starts at the first byte: a shell that is slow to
        // start (a busy CI runner, #171) is not a pane that has stopped.
        let mut got = 0usize;
        let mut idle = 0;
        let started = std::time::Instant::now();
        while idle < 5 && (got > 0 || started.elapsed() < Duration::from_secs(20)) {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok((p, PaneEvent::Output(b))) if p == pane => {
                    got += b.len();
                    idle = 0;
                }
                Ok(_) => {}
                Err(_) => {
                    if got > 0 {
                        idle += 1;
                    }
                }
            }
        }
        assert!(got > HIGH_WATER, "never reached the mark: {got}");
        assert!(
            got <= HIGH_WATER + 8192,
            "kept reading past the mark: {got}"
        );

        // Acking everything lets it flow again.
        backend.ack(pane, got);
        let more = matches!(
            rx.recv_timeout(Duration::from_secs(2)),
            Ok((p, PaneEvent::Output(_))) if p == pane
        );
        assert!(more, "did not resume after ack");

        // And a kill frees a reader parked on credit, so the pane still exits.
        backend.kill(pane);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok((p, PaneEvent::Exited { .. })) = rx.recv_timeout(Duration::from_millis(200)) {
                if p == pane {
                    return;
                }
            }
        }
        panic!("killed pane never exited");
    }

    /// The contract later tasks rely on: two panes, one stream, correct tags.
    #[tokio::test]
    async fn tags_events_by_pane() {
        let (backend, rx) = LocalPtyBackend::new();
        let a = backend
            .spawn(std::path::Path::new("/tmp"), Some("echo from-a"), vec![])
            .await
            .unwrap();
        let b = backend
            .spawn(std::path::Path::new("/tmp"), Some("echo from-b"), vec![])
            .await
            .unwrap();
        assert_ne!(a, b);

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut acc: HashMap<PaneId, String> = HashMap::new();
        while std::time::Instant::now() < deadline {
            if let Ok((p, PaneEvent::Output(bytes))) = rx.recv_timeout(Duration::from_millis(200)) {
                acc.entry(p)
                    .or_default()
                    .push_str(&String::from_utf8_lossy(&bytes));
            }
            let a_ok = acc.get(&a).is_some_and(|s| s.contains("from-a"));
            let b_ok = acc.get(&b).is_some_and(|s| s.contains("from-b"));
            if a_ok && b_ok {
                // Neither pane's output leaked into the other's tag.
                assert!(!acc[&a].contains("from-b"));
                assert!(!acc[&b].contains("from-a"));
                return;
            }
        }
        panic!("did not see tagged output from both panes: {acc:?}");
    }
}
