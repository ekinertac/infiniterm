//! The app's end of the socket to `iftd`. Mirrors `local_pty.rs`'s shape on
//! purpose: a `Pane` struct, an `Arc<Mutex<HashMap<PaneId, Pane>>>`, the same
//! `Credit` backpressure and the same `_now` method names, so nothing above
//! `Panes` learns that a socket replaced a pty.
//!
//! Where it fits: `Panes` (backend/mod.rs, wired up in Task 4) will grow a
//! third arm around this type. Until then this module stands alone, tested
//! directly against a real `iftd` (infiniterm-session, Task 2).
//!
//! Related: `session_protocol.rs` for the wire (`Frame`, `FrameReader`,
//! `Ring`), `infiniterm-session/src/main.rs` for the daemon this talks to,
//! `local_pty.rs` for the backend this backend's shell would otherwise be,
//! docs/superpowers/specs/2026-09-17-session-daemon-design.md for why any of
//! this exists instead of tmux.
//!
//! Non-obvious constraints:
//! - `spawn_now` runs `iftd` with `Command::status()`, not `spawn()`: iftd
//!   binds its listener before it forks, so THAT exit returning success is
//!   the readiness signal. There is nothing to poll.
//! - After `Frame::ReplayEnd`, the pane is resized a column and back. See
//!   `nudge_resize` below — this is the safety net the design's risk 1
//!   depends on, not an optimisation to simplify away.
//! - `live_sessions` both answers "what can be adopted" and sweeps: a
//!   `.sock` file nothing answers is a daemon that died without cleaning up
//!   (`ECONNREFUSED`, or "not a socket" for a stray file), and is unlinked
//!   on the spot so a stale entry does not haunt every later listing.
//! - `detach()` shuts down our sockets and returns; it does not touch a
//!   single `iftd` process. That asymmetry (`Kill` writes a frame, `detach`
//!   closes a handle) is the entire feature this backend exists to add.

use super::local_pty::HIGH_WATER;
use super::relay::Relay;
use super::session_protocol::{Frame, FrameReader};
use super::{PaneEvent, PaneId, Redraw};
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// `sizeof(sockaddr_un.sun_path)` on macOS, NUL included. AF_UNIX simply
/// cannot name a path longer than this — it is not a big ask that might
/// still work, it is not a socket at all. Checked in `spawn_now` before
/// `iftd` ever runs, so the failure names the actual path and the actual
/// limit instead of iftd's own bind failing three processes away with
/// nothing but exit code 4 to explain it. `paths::sessions_dir` already
/// spends as little of this budget as it reasonably can; this is the net
/// that catches whatever is left, such as a longer home directory.
const SUN_PATH_MAX: usize = 104;

/// How long `spawn_now`/`adopt` will wait for iftd's `Hello` before giving
/// up. Generous: this covers process start plus `openpty` plus a shell
/// fork, not a network round trip.
const HELLO_DEADLINE: Duration = Duration::from_secs(5);

/// The same wait over ssh: a connection, a login and a remote daemon start
/// before `Hello`, so a network round trip or three is in it.
const REMOTE_HELLO_DEADLINE: Duration = Duration::from_secs(20);

/// Socket read granularity while polling for `Hello`. Short enough that
/// `HELLO_DEADLINE` is honoured to within a fraction of a second.
const HELLO_POLL: Duration = Duration::from_millis(100);

/// Identical in shape and purpose to `local_pty::Credit`: the reader thread
/// below stops reading the SOCKET once `HIGH_WATER` bytes are sent but not
/// yet acknowledged, and resumes on ack. Not shared with `local_pty` because
/// its `Credit` is private to that module and the two have no other reason
/// to depend on each other; the constant they gate on (`HIGH_WATER`) is
/// shared instead, so the two backends agree on the actual number.
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
    /// The session id, i.e. the socket's filename stem. Kept so a card can
    /// save it and `adopt` it back after a relaunch.
    session_id: String,
    /// A clone of the connected socket, used for writes (`Data`, `Resize`,
    /// `Kill`). Wrapped separately from the pane map so a write that blocks
    /// only stalls this one pane, exactly as `local_pty::Pane::writer` does
    /// for the same reason.
    stream: Arc<Mutex<UnixStream>>,
    /// The last size this pane was told about, seeded from `Hello`. Needed
    /// so `nudge_resize` knows what "back" means; iftd itself does not
    /// answer a "what size are you" query.
    size: Mutex<(u16, u16)>,
    /// The child's pid, learned from `Hello`. Always `Some` for a daemon
    /// pane: unlike `local_pty`, iftd always knows its child's pid before
    /// it ever says `Hello`.
    pid: Option<u32>,
    credit: Arc<Credit>,
    /// A viewer's pane (#423): its session is the host's, reached through the
    /// host app. Nothing in this backend owns it, so a close here only lets
    /// go of the connection; the host closes the card by its own path.
    viewer: bool,
}

fn send(stream: &Arc<Mutex<UnixStream>>, frame: &Frame) {
    let _ = stream.lock().unwrap().write_all(&frame.encode());
}

pub struct DaemonBackend {
    /// Where a card's socket lives: `<sessions_dir>/<id>.sock`. Created on
    /// first spawn rather than at construction, so building a `DaemonBackend`
    /// never touches the filesystem by itself.
    sessions_dir: PathBuf,
    /// `iftd --buffer <buffer_mib>` for every pane this backend spawns.
    buffer_mib: usize,
    panes: Arc<Mutex<HashMap<PaneId, Pane>>>,
    tx: Sender<(PaneId, PaneEvent)>,
    next_id: AtomicU32,
    /// Set in a remote instance (#118): every spawn and adopt goes to this
    /// host over ssh and the sessions are listed by asking it.
    remote: Option<super::remote::RemoteHost>,
    /// What viewers that joined this canvas (#423, `ift join`) are sent: this
    /// backend is the only client iftd sees, so it is the one place every
    /// session's bytes pass, and it fans them out from here.
    relay: Arc<Mutex<Relay>>,
    /// Set for the window that `ift join` opens (#423): the sessions are the
    /// host's, reached through the host app, and this backend never starts,
    /// resizes or ends one of them. Every way a session could die or change
    /// is a no-op here: kill, resize, spawn, the orphan sweep, the parked end.
    viewer: bool,
    /// Connections a viewer's reconnect threads finished and the app has not
    /// taken yet (`take_reconnected`): the connecting waits on ssh, so it
    /// happens on a thread, and the cheap `register` on the app's.
    reconnected: Arc<Mutex<Vec<Reconnected>>>,
}

/// A viewer card's new connection to its session, or why there is none.
pub type Reconnected = (String, Result<JoinedSession, super::remote::JoinFailure>);

/// What a finished join of one session holds until it is registered.
pub struct JoinedSession {
    stream: UnixStream,
    reader: FrameReader,
    size: (u16, u16),
}

impl DaemonBackend {
    pub fn new(sessions_dir: PathBuf, buffer_mib: usize) -> (Self, Receiver<(PaneId, PaneEvent)>) {
        let (tx, rx) = channel();
        let backend = Self {
            sessions_dir,
            buffer_mib,
            panes: Arc::new(Mutex::new(HashMap::new())),
            tx,
            next_id: AtomicU32::new(1),
            remote: None,
            relay: Arc::new(Mutex::new(Relay::new())),
            viewer: false,
            reconnected: Arc::new(Mutex::new(Vec::new())),
        };
        (backend, rx)
    }

    /// Joins a viewer card's session again (#442) on a thread of its own: an
    /// unreachable host would otherwise hold a frame for the whole handshake.
    /// The answer is waiting in `take_reconnected` when it ends. Not a viewer:
    /// nothing happens.
    pub fn begin_reconnect(&self, session_id: &str) {
        let Some(host) = self.remote.clone().filter(|_| self.viewer) else {
            return;
        };
        let (id, done) = (session_id.to_string(), self.reconnected.clone());
        std::thread::spawn(move || {
            let result =
                super::remote::connect_join(&host, &id).and_then(|(mut stream, stderr)| {
                    match read_hello(&mut stream, REMOTE_HELLO_DEADLINE) {
                        Ok((_pid, cols, rows, reader)) => Ok(JoinedSession {
                            stream,
                            reader,
                            size: (cols, rows),
                        }),
                        Err(e) => {
                            let said = stderr.text();
                            Err(super::remote::JoinFailure::Retry(if said.is_empty() {
                                e.to_string()
                            } else {
                                said
                            }))
                        }
                    }
                });
            done.lock().unwrap().push((id, result));
        });
    }

    /// The reconnects that ended since the last call: a new pane for each
    /// that worked, the failure for each that did not.
    pub fn take_reconnected(&self) -> Vec<(String, Result<PaneId, super::remote::JoinFailure>)> {
        let done: Vec<Reconnected> = std::mem::take(&mut *self.reconnected.lock().unwrap());
        done.into_iter()
            .map(|(id, result)| {
                let pane =
                    result.map(|j| self.register(id.clone(), j.stream, j.reader, None, j.size));
                (id, pane)
            })
            .collect()
    }

    /// A backend for a viewer window (#423): its cards are cards of the app
    /// running on `host`, joined through that app, never through `iftd`.
    pub fn new_viewer(
        host: super::remote::RemoteHost,
        buffer_mib: usize,
    ) -> (Self, Receiver<(PaneId, PaneEvent)>) {
        let (mut backend, rx) = Self::new_remote(host, buffer_mib);
        backend.viewer = true;
        (backend, rx)
    }

    /// A backend whose sessions live on `host`. The local sessions directory
    /// is still named, because the pieces that read it (the orphan sweep, the
    /// ring from disk, `attached`) find nothing there and do nothing.
    pub fn new_remote(
        host: super::remote::RemoteHost,
        buffer_mib: usize,
    ) -> (Self, Receiver<(PaneId, PaneEvent)>) {
        let (mut backend, rx) = Self::new(crate::paths::sessions_dir(), buffer_mib);
        backend.remote = Some(host);
        (backend, rx)
    }

    /// Which sessions can be adopted at launch: the server's (one ssh), or the
    /// local sockets. A server that cannot be asked yields an empty list and
    /// the reason on stderr; `ift connect` checks reachability before it
    /// opens the app, so this is the rare case.
    pub fn live_sessions_now(&self) -> Vec<String> {
        match &self.remote {
            Some(host) => super::remote::list_sessions(host).unwrap_or_else(|e| {
                eprintln!("[infiniterm] could not list the sessions on {}", e);
                Vec::new()
            }),
            None => Self::live_sessions(&self.sessions_dir),
        }
    }

    pub fn spawn_now(
        &self,
        cwd: &Path,
        cmd: Option<&str>,
        env: Vec<(String, String)>,
    ) -> anyhow::Result<PaneId> {
        if self.viewer {
            anyhow::bail!(
                "a viewer starts no shell here: a card made in a viewer is started by the host"
            );
        }
        if let Some(host) = &self.remote {
            return self.spawn_remote(host, &cwd.to_string_lossy(), cmd, env);
        }
        let session_id = new_session_id();
        let socket = self.sessions_dir.join(format!("{session_id}.sock"));
        check_socket_path(&socket)?;
        std::fs::create_dir_all(&self.sessions_dir)?;
        let iftd = find_iftd()?;

        let mut command = std::process::Command::new(&iftd);
        command
            .arg("--socket")
            .arg(&socket)
            .arg("--cwd")
            .arg(cwd)
            .arg("--buffer")
            .arg(self.buffer_mib.to_string());
        if let Some(c) = cmd {
            command.arg("--cmd").arg(c);
        }
        for (k, v) in &env {
            command.arg("--env").arg(format!("{k}={v}"));
        }

        // iftd binds its listener before it forks and only then exits 0 (see
        // its own header): this status IS the readiness signal, and there is
        // nothing to poll for.
        let status = command
            .status()
            .map_err(|e| anyhow::anyhow!("could not run {}: {e}", iftd.display()))?;
        if !status.success() {
            anyhow::bail!(
                "{} exited with {status}; see its .meta file",
                iftd.display()
            );
        }

        let mut stream = UnixStream::connect(&socket).map_err(|e| {
            anyhow::anyhow!(
                "{} exited 0 but its socket refused a connection: {e}",
                iftd.display()
            )
        })?;
        let (pid, cols, rows, reader) = read_hello(&mut stream, HELLO_DEADLINE)?;
        Ok(self.register(session_id, stream, reader, Some(pid), (cols, rows)))
    }

    /// Takes over a session an earlier launch left running. `None` when the
    /// socket refuses a connection (a daemon that already exited) or `Hello`
    /// never arrives; the caller spawns a fresh shell instead, same as a
    /// `local_pty` card that never had a session to come back to.
    pub fn adopt(&self, session_id: &str) -> Option<PaneId> {
        if self.viewer {
            return self.adopt_join(session_id);
        }
        if let Some(host) = &self.remote {
            return self.adopt_remote(host, session_id);
        }
        let socket = self.sessions_dir.join(format!("{session_id}.sock"));
        let mut stream = UnixStream::connect(&socket).ok()?;
        let (pid, cols, rows, reader) = read_hello(&mut stream, HELLO_DEADLINE).ok()?;
        Some(self.register(
            session_id.to_string(),
            stream,
            reader,
            Some(pid),
            (cols, rows),
        ))
    }

    /// Joins one of the host's cards through the host app (#423), never
    /// through its `iftd`, which serves one client and would drop the host's
    /// own window. The pane has no local pid and no relay of its own.
    fn adopt_join(&self, session_id: &str) -> Option<PaneId> {
        let host = self.remote.as_ref()?;
        let (mut stream, stderr) = match super::remote::connect_join(host, session_id) {
            Ok(c) => c,
            Err(why) => {
                eprintln!(
                    "[infiniterm] could not join {session_id}: {}",
                    why.message()
                );
                return None;
            }
        };
        match read_hello(&mut stream, REMOTE_HELLO_DEADLINE) {
            Ok((_host_pid, cols, rows, reader)) => {
                Some(self.register(session_id.to_string(), stream, reader, None, (cols, rows)))
            }
            Err(e) => {
                let said = stderr.text();
                eprintln!("[infiniterm] could not join {session_id}: {e} {said}");
                None
            }
        }
    }

    /// A new shell on another machine: `ift proxy session --spawn` over ssh
    /// starts the daemon there and this attaches to it (#118). The pane has no
    /// local pid, so the inspect poller never looks up a process on THIS
    /// machine that happens to share the remote's pid.
    pub fn spawn_remote(
        &self,
        host: &super::remote::RemoteHost,
        cwd: &str,
        cmd: Option<&str>,
        env: Vec<(String, String)>,
    ) -> anyhow::Result<PaneId> {
        let session_id = new_session_id();
        let spec = super::remote::SpawnSpec {
            cwd: Some(cwd.to_string()),
            cmd: cmd.map(str::to_string),
            env,
            buffer_mib: Some(self.buffer_mib),
        };
        self.attach_remote(host, session_id, Some(&spec))
    }

    /// Takes over a session the server still holds. `None` when it is gone
    /// or the connection fails.
    pub fn adopt_remote(
        &self,
        host: &super::remote::RemoteHost,
        session_id: &str,
    ) -> Option<PaneId> {
        self.attach_remote(host, session_id.to_string(), None).ok()
    }

    fn attach_remote(
        &self,
        host: &super::remote::RemoteHost,
        session_id: String,
        spawn: Option<&super::remote::SpawnSpec>,
    ) -> anyhow::Result<PaneId> {
        let (mut stream, stderr) = super::remote::connect(host, &session_id, spawn)
            .map_err(|e| anyhow::anyhow!("could not run {}: {e}", host.ssh))?;
        match read_hello(&mut stream, REMOTE_HELLO_DEADLINE) {
            Ok((_remote_pid, cols, rows, reader)) => {
                Ok(self.register(session_id, stream, reader, None, (cols, rows)))
            }
            Err(e) => {
                // Give ssh a moment to say why before reporting it.
                std::thread::sleep(Duration::from_millis(150));
                let why = stderr.text();
                if why.is_empty() {
                    Err(e)
                } else {
                    Err(anyhow::anyhow!("{}: {why}", host.target))
                }
            }
        }
    }

    /// Common tail of `spawn_now` and `adopt`: the socket is connected and
    /// `Hello` has already been read (any `Replay`/`ReplayEnd` bytes that
    /// arrived in the same read are still sitting in `reader`, unparsed).
    /// Only from here is the pane visible to `write_now` and friends.
    fn register(
        &self,
        session_id: String,
        stream: UnixStream,
        reader: FrameReader,
        pid: Option<u32>,
        size: (u16, u16),
    ) -> PaneId {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let credit = Arc::new(Credit::default());
        // Cloned before `stream` moves into the reader thread: one socket,
        // one half kept here for writes, the other read from a dedicated
        // thread, exactly as local_pty splits its pty master and writer.
        let write_stream = stream
            .try_clone()
            .expect("a freshly connected UnixStream always clones");

        self.relay.lock().unwrap().open(
            &session_id,
            Frame::Hello {
                pid: pid.unwrap_or(0),
                cols: size.0,
                rows: size.1,
            },
        );
        let relay = self.relay.clone();
        let relay_session = session_id.clone();
        self.panes.lock().unwrap().insert(
            id,
            Pane {
                session_id,
                stream: Arc::new(Mutex::new(write_stream)),
                size: Mutex::new(size),
                pid,
                credit: credit.clone(),
                viewer: self.viewer,
            },
        );

        let tx = self.tx.clone();
        let panes = self.panes.clone();
        std::thread::spawn(move || {
            run_reader(
                id,
                stream,
                reader,
                tx,
                panes,
                credit,
                Tap {
                    relay: &relay,
                    session: &relay_session,
                },
            );
            // The reader is gone (exit, detach or eviction): viewers of this
            // session end with it. A later adopt registers a new reader and
            // opens the relay again.
            relay.lock().unwrap().close(&relay_session);
        });

        id
    }

    /// A handle to the live pane pids that outlives any borrow of the
    /// backend, for the same remote-session poller `local_pty::pids_source`
    /// serves. Unlike that backend, every daemon pane has a real pid: iftd
    /// always learns its child's before it says `Hello`.
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

    /// The directory this backend's sockets live under, so `Panes::live_sessions`
    /// can ask `DaemonBackend::live_sessions` about the right one without
    /// this module and `backend/mod.rs` each hard-coding `paths::sessions_dir()`.
    pub fn sessions_dir(&self) -> &Path {
        &self.sessions_dir
    }

    /// The session id a pane was spawned or adopted with, for the save file.
    pub fn session_id(&self, pane: PaneId) -> Option<String> {
        self.panes
            .lock()
            .unwrap()
            .get(&pane)
            .map(|p| p.session_id.clone())
    }

    /// The size the host's shell has, for a viewer's pane only: from the
    /// host's hello and every later `Resize` it sent (#423).
    pub fn viewer_pane_size(&self, pane: PaneId) -> Option<(u16, u16)> {
        if !self.viewer {
            return None;
        }
        self.panes
            .lock()
            .unwrap()
            .get(&pane)
            .map(|p| *p.size.lock().unwrap())
    }

    /// The viewers' relay, shared with the join server (#423).
    pub fn relay(&self) -> Arc<Mutex<Relay>> {
        self.relay.clone()
    }

    /// Writes keys a viewer sent to the session they were typed for, from any
    /// thread. The same path as the host's own keys: one connection to iftd,
    /// whole frames serialised by the pane's stream lock, so a host key and a
    /// viewer key never interleave inside a frame. A session this backend does
    /// not hold is ignored. Not through the app's poll, which would add up to
    /// 16 ms to every viewer keystroke.
    pub fn session_writer(&self) -> super::join::WriteSession {
        let panes = self.panes.clone();
        Arc::new(move |session_id: &str, bytes: &[u8]| {
            let stream = panes
                .lock()
                .unwrap()
                .values()
                .find(|p| p.session_id == session_id)
                .map(|p| p.stream.clone());
            if let Some(stream) = stream {
                let mut s = stream.lock().unwrap();
                let _ = s.write_all(&Frame::Data(bytes.to_vec()).encode());
            }
        })
    }

    /// The repaint jog for a viewer that just attached (#423): the same two
    /// resizes `nudge_resize` sends after the host's own replay, so a program
    /// that draws with relative cursor moves (Claude Code's input box)
    /// redraws itself over the ring the viewer replayed. The viewer gets the
    /// redraw as live output, which queues behind its replay.
    pub fn session_nudger(&self) -> Arc<dyn Fn(&str) + Send + Sync> {
        let panes = self.panes.clone();
        Arc::new(move |session_id: &str| {
            let id = panes
                .lock()
                .unwrap()
                .iter()
                .find(|(_, p)| p.session_id == session_id)
                .map(|(id, _)| *id);
            if let Some(id) = id {
                nudge_resize(&panes, id);
            }
        })
    }

    /// `terminal.redraw` (#447): the repaint jog on this pane, the same two
    /// resizes an attach sends. A viewer's pane has no say over the host's
    /// size (the host ignores its `Resize` frames), so it lets go of its
    /// connection instead and says so: the caller joins the session again,
    /// which replays the ring and makes the host jog the program
    /// (`session_nudger`). Does nothing for a pane that is gone.
    pub fn redraw_now(&self, pane: PaneId) -> Redraw {
        let viewer = self.panes.lock().unwrap().get(&pane).map(|p| p.viewer);
        if viewer == Some(true) {
            self.detach_now(pane);
            return Redraw::Rejoin;
        }
        nudge_resize(&self.panes, pane);
        Redraw::Asked
    }

    pub fn write_now(&self, pane: PaneId, bytes: &[u8]) {
        let found = self
            .panes
            .lock()
            .unwrap()
            .get(&pane)
            .map(|p| p.stream.clone());
        if let Some(stream) = found {
            send(&stream, &Frame::Data(bytes.to_vec()));
        }
    }

    pub fn resize_now(&self, pane: PaneId, cols: u16, rows: u16) {
        let stream = {
            let panes = self.panes.lock().unwrap();
            let Some(p) = panes.get(&pane) else {
                return;
            };
            *p.size.lock().unwrap() = (cols, rows);
            self.relay.lock().unwrap().resize(&p.session_id, cols, rows);
            p.stream.clone()
        };
        send(&stream, &Frame::Resize { cols, rows });
    }

    pub fn kill_now(&self, pane: PaneId) {
        let found = {
            let panes = self.panes.lock().unwrap();
            panes
                .get(&pane)
                .map(|p| (p.stream.clone(), p.credit.clone(), p.viewer))
        };
        let Some((stream, credit, viewer)) = found else {
            return;
        };
        if viewer {
            // A card closed in a viewer IS closed on the host: the canvas the
            // viewer sends up no longer has it, and the host closes it by its
            // own path (`Model::apply_remote_layout`, parking and all). This
            // pane only lets go of its connection, as `detach_now` does.
            let _ = stream.lock().unwrap().shutdown(Shutdown::Both);
            credit.close();
            return;
        }
        send(&stream, &Frame::Kill);
        // Frees a reader parked on credit, same reason and same call as
        // local_pty::kill_now: the pane is removed by the reader thread once
        // Exited (or EOF) actually arrives, not here, so PaneEvent::Exited
        // still fires exactly once.
        credit.close();
    }

    /// Lets go of one pane WITHOUT ending its session: the card closed but
    /// may come back (`Model::parked`), and `adopt` takes it back. The
    /// reader sees EOF and reports `Detached`, which no card owns any more.
    pub fn detach_now(&self, pane: PaneId) {
        let found = {
            let panes = self.panes.lock().unwrap();
            panes
                .get(&pane)
                .map(|p| (p.stream.clone(), p.credit.clone()))
        };
        let Some((stream, credit)) = found else {
            return;
        };
        let _ = stream.lock().unwrap().shutdown(Shutdown::Both);
        credit.close();
    }

    pub fn ack_now(&self, pane: PaneId, bytes: usize) {
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

    /// Kills every pane. Used the same way `local_pty::kill_all` is: the
    /// consumer that owned these cards is gone.
    pub fn kill_all(&self) {
        let ids: Vec<PaneId> = self.panes.lock().unwrap().keys().copied().collect();
        for id in ids {
            self.kill_now(id);
        }
    }

    /// Sessions this app made that no card claims: what a crash left
    /// running. Same sweep as tmux's `kill_orphans`, on a directory only
    /// this backend writes to.
    pub fn kill_orphans(&self, claimed: &[String]) {
        // A viewer owns no session: the ones in its data dir, if any, are not
        // litter of its own, and the host's are the host's to sweep.
        if self.viewer {
            return;
        }
        for id in Self::live_sessions(&self.sessions_dir) {
            if claimed.iter().any(|c| c == &id) {
                continue;
            }
            let socket = self.sessions_dir.join(format!("{id}.sock"));
            std::thread::spawn(move || send_kill(&socket));
        }
        // A dead session's ring that no saved card will ask for (the card
        // was closed while the app was down) is a leftover, not scrollback.
        if let Ok(entries) = std::fs::read_dir(&self.sessions_dir) {
            for path in entries.flatten().map(|e| e.path()) {
                if path.extension().is_none_or(|e| e != "ring") {
                    continue;
                }
                let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if !claimed.iter().any(|c| c == id) {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }

    /// The `attached` flag in a session's meta file, which `iftd` rewrites
    /// on every attach and detach. A meta from before the flag reads as
    /// not attached, which errs toward taking the session back.
    pub fn session_attached(&self, session_id: &str) -> Option<bool> {
        let path = self.sessions_dir.join(format!("{session_id}.meta"));
        let text = std::fs::read_to_string(path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        Some(v.get("attached").and_then(|a| a.as_bool()).unwrap_or(false))
    }

    /// The scrollback a dead session's daemon last wrote to disk
    /// (`<id>.ring`, see `iftd`'s SNAPSHOT_INTERVAL), taken: the file is
    /// removed, so a card replays it once and a later launch does not
    /// stack a second copy under the new shell. With it, when the file was
    /// last written, local time, which is within two seconds of the last
    /// output the card saw. `None` for a live session (its daemon still
    /// has the ring and `adopt` gets it over the socket) and for a session
    /// that left nothing.
    pub fn take_ring(&self, session_id: &str) -> Option<(Vec<u8>, String)> {
        let path = self.sessions_dir.join(format!("{session_id}.ring"));
        let bytes = std::fs::read(&path).ok()?;
        let when = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .map(|t| {
                chrono::DateTime::<chrono::Local>::from(t)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        let _ = std::fs::remove_file(&path);
        (!bytes.is_empty()).then_some((bytes, when))
    }

    /// Drops every socket this backend holds and returns; it sends nothing
    /// to any `iftd` and kills nothing. This is `Panes::leave` under the
    /// daemon backend: the shells are meant to keep running.
    ///
    /// Only shutting down the write half we hold is not enough by itself —
    /// a `try_clone`'d `UnixStream` is a second file descriptor onto the
    /// SAME kernel socket, so `shutdown` on one half is visible to the
    /// other, which is what actually wakes the reader thread's blocked
    /// `read()` with EOF (mirrors the accept loop's own eviction in
    /// `infiniterm-session/src/main.rs`).
    pub fn detach(&self) {
        for p in self.panes.lock().unwrap().values() {
            let _ = p.stream.lock().unwrap().shutdown(Shutdown::Both);
        }
    }

    /// Which sessions under `dir` are still live, sweeping the ones that
    /// are not. A `.sock` file with nothing listening is a daemon that died
    /// without unlinking it (a crash, `kill -9`); removed on the spot so it
    /// does not haunt this or a later listing.
    pub fn live_sessions(dir: &Path) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut ids = Vec::new();
        for path in entries.flatten().map(|e| e.path()) {
            if path.extension().is_none_or(|e| e != "sock") {
                continue;
            }
            if UnixStream::connect(&path).is_ok() {
                if let Some(id) = path.file_stem().and_then(|s| s.to_str()) {
                    ids.push(id.to_string());
                }
            } else {
                // Nothing is listening, so the daemon died without cleaning
                // up. Take the meta with the socket: `ift sessions` reads
                // the metas, and a meta left behind advertises a session
                // that cannot be attached to.
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::remove_file(path.with_extension("meta"));
            }
        }
        ids
    }
}

/// Reads frames until `Hello` arrives or `HELLO_DEADLINE` passes, returning
/// the pid, size and the `FrameReader` so far — which may already hold
/// `Replay`/`ReplayEnd` bytes read in the same syscall as `Hello`. Losing
/// those would be exactly the kind of desync `FrameReader::feed` exists to
/// prevent, so the reader is threaded through rather than discarded here.
fn read_hello(
    stream: &mut UnixStream,
    wait: Duration,
) -> anyhow::Result<(u32, u16, u16, FrameReader)> {
    stream.set_read_timeout(Some(HELLO_POLL))?;
    let mut reader = FrameReader::default();
    let mut buf = [0u8; 8192];
    let deadline = Instant::now() + wait;
    loop {
        match reader.next() {
            Ok(Some(Frame::Hello { pid, cols, rows })) => {
                // Back to blocking: the reader thread this hands off to
                // wants a real block, not a poll loop of its own.
                stream.set_read_timeout(None)?;
                return Ok((pid, cols, rows, reader));
            }
            // iftd's accept loop always writes Hello first; anything else
            // here would mean a protocol we do not speak.
            Ok(Some(other)) => anyhow::bail!("expected Hello first, got {other:?}"),
            Ok(None) => {}
            Err(e) => anyhow::bail!("iftd's greeting: {e}"),
        }
        if Instant::now() > deadline {
            anyhow::bail!("no Hello from iftd within {wait:?}");
        }
        match stream.read(&mut buf) {
            Ok(0) => anyhow::bail!("iftd closed the connection before Hello"),
            Ok(n) => reader.feed(&buf[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
}

/// Where a pane's session output is also sent: the viewers' relay and the
/// session id it files the bytes under (#423). One argument for `run_reader`
/// instead of two.
struct Tap<'a> {
    relay: &'a Arc<Mutex<Relay>>,
    session: &'a str,
}

/// One per pane, for its whole life. Turns frames arriving on the socket
/// into tagged `PaneEvent`s on the one shared channel, gated by `credit`
/// exactly as `local_pty`'s reader gates its pty reads — see this module's
/// header for why the two `Credit`s are separate types over one shared
/// constant.
fn run_reader(
    id: PaneId,
    mut stream: UnixStream,
    mut reader: FrameReader,
    tx: Sender<(PaneId, PaneEvent)>,
    panes: Arc<Mutex<HashMap<PaneId, Pane>>>,
    credit: Arc<Credit>,
    tap: Tap<'_>,
) {
    let mut buf = vec![0u8; 8192];
    let mut exited = false;
    'outer: loop {
        loop {
            match reader.next() {
                Ok(Some(Frame::Data(bytes))) => {
                    credit.sent(bytes.len());
                    tap.relay.lock().unwrap().publish(tap.session, &bytes);
                    if tx.send((id, PaneEvent::Output(bytes))).is_err() {
                        break 'outer;
                    }
                }
                Ok(Some(Frame::Replay(bytes))) => {
                    credit.sent(bytes.len());
                    tap.relay.lock().unwrap().seed(tap.session, &bytes);
                    if tx.send((id, PaneEvent::Replay(bytes))).is_err() {
                        break 'outer;
                    }
                }
                Ok(Some(Frame::ReplayEnd)) => nudge_resize(&panes, id),
                Ok(Some(Frame::Exited(code))) => {
                    tap.relay.lock().unwrap().exited(tap.session, code);
                    let _ = tx.send((id, PaneEvent::Exited { code }));
                    exited = true;
                    break 'outer;
                }
                // The host's size changed (#423): only a viewer's pane is
                // told, and only a viewer's pane keeps it.
                Ok(Some(Frame::Resize { cols, rows })) => {
                    if let Some(p) = panes.lock().unwrap().get(&id) {
                        if p.viewer {
                            *p.size.lock().unwrap() = (cols, rows);
                        }
                    }
                }
                // Hello only ever arrives once, consumed by read_hello
                // before this thread starts; Kill only ever travels
                // app-to-daemon. Neither is ours to act on, and dropping
                // the connection over it would be the wrong failure mode
                // for a frame that is merely misdirected.
                Ok(Some(Frame::Hello { .. } | Frame::Kill)) => {}
                Ok(None) => break,
                // The stream is not ours, or is no longer in step with us:
                // terminal, per ProtoError's own doc comment.
                Err(_) => break 'outer,
            }
        }
        if !credit.wait_for_room() {
            break;
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => reader.feed(&buf[..n]),
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    if let Some(p) = panes.lock().unwrap().remove(&id) {
        // Frees a reader parked on credit elsewhere (there is only one
        // reader per pane here, but closing is idempotent and cheap, and
        // matches local_pty's exit watcher doing the same for symmetry).
        p.credit.close();
    }
    // EOF without Exited: the daemon evicted us for another client. Said,
    // or the card sits frozen with nobody knowing why (it did, once).
    if !exited {
        let _ = tx.send((id, PaneEvent::Detached));
    }
}

/// How long `send_kill` waits for a daemon to greet and then to go.
const KILL_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// Ends the session behind `socket` from a connection of its own. iftd
/// greets every client with `Hello` and the whole ring BEFORE it reads a
/// frame, and drops a client whose greeting fails to send; a Kill written
/// on connect and the socket closed straight after was lost that way
/// whenever the ring outgrew the socket buffer (found 2026-09-29 by the
/// parked-session test). So: read the greeting, send Kill, and hold the
/// connection until the daemon closes it. Blocking; callers run it on a
/// thread of its own.
fn send_kill(socket: &Path) {
    let Ok(mut s) = UnixStream::connect(socket) else {
        return;
    };
    if read_hello(&mut s, HELLO_DEADLINE).is_err() {
        return;
    }
    // After the greeting: `read_hello` sets its own short poll.
    let _ = s.set_read_timeout(Some(KILL_WAIT));
    let _ = s.write_all(&Frame::Kill.encode());
    let mut sink = [0u8; 8192];
    while matches!(s.read(&mut sink), Ok(n) if n > 0) {}
}

/// After a replay ends, jog the pane's width down one column and back so a
/// `SIGWINCH` reaches the child.
///
/// The ring's replay is raw bytes cut at a line boundary (`Ring::push`), not
/// a snapshot of terminal MODE: a colour left set before the cut is undone
/// by the `\x1b[0m` `Ring::replay` prefixes, but a program that entered the
/// alternate screen before the cut has no re-entry sequence anywhere in a
/// truncated replay. The resize makes a full-screen program (Ink, vim,
/// htop — anything that repaints on `SIGWINCH`) redraw its own idea of the
/// screen over whatever the replay left, which self-corrects within one
/// frame; a shell just redraws its prompt. This is the safety net the
/// design's risk 1 depends on — see the spec's "The ring" section.
fn nudge_resize(panes: &Arc<Mutex<HashMap<PaneId, Pane>>>, id: PaneId) {
    let found = {
        let panes = panes.lock().unwrap();
        panes
            .get(&id)
            .map(|p| (p.stream.clone(), *p.size.lock().unwrap()))
    };
    let Some((stream, (cols, rows))) = found else {
        return;
    };
    // A pane is never actually 0 wide (Hello always carries iftd's real
    // initial size), but a bump that cannot go below 1 avoids ever asking
    // for a 0-column pty regardless.
    let bumped = if cols > 1 { cols - 1 } else { cols + 1 };
    let mut s = stream.lock().unwrap();
    let _ = s.write_all(&Frame::Resize { cols: bumped, rows }.encode());
    let _ = s.write_all(&Frame::Resize { cols, rows }.encode());
}

/// Refuses a socket path before it is ever bound, rather than letting a
/// too-long one reach `iftd` and fail there with an opaque exit code. See
/// `SUN_PATH_MAX`'s own doc comment for why this exists at all.
pub fn check_socket_path(socket: &Path) -> anyhow::Result<()> {
    // +1: sockaddr_un.sun_path always carries a terminating NUL, which is
    // part of the 104-byte budget, not extra room beyond it.
    let len = socket.as_os_str().as_bytes().len() + 1;
    if len > SUN_PATH_MAX {
        anyhow::bail!(
            "{} is {len} bytes, over the {SUN_PATH_MAX}-byte unix socket path limit \
             (sockaddr_un.sun_path on macOS, NUL included); shorten INFINITERM_DATA_DIR \
             or your home directory path",
            socket.display()
        );
    }
    Ok(())
}

/// Whether `iftd` can be found at all, for `Panes::start`'s fallback: a
/// daemon backend with no sidecar to run must not mean no terminal, exactly
/// as a missing `tmux` binary does not.
pub fn available() -> bool {
    find_iftd().is_ok()
}

/// Names a socket file under `sessions_dir` and nothing else: a slash or a
/// `..` in it would name a file outside that directory. Also short, which
/// is not cosmetic: the id becomes `sessions_dir.join(id + ".sock")`, and
/// `sockaddr_un.sun_path` is capped at 104 bytes on macOS, NUL included.
/// `~/Library/Application Support/dev.ekinertac.infiniterm/sessions/` alone
/// is already ~80 of that budget, so a hyphenated 36-character UUID would
/// overflow it (`infiniterm-session/tests/roundtrip.rs`'s own `TempDir` hit
/// this exact limit and says so in its header). 16 lowercase hex characters
/// — 64 bits off the front of the same CSPRNG `uuid` already pulls in — are
/// collision-free at the scale of "cards open at once" and fit with room
/// to spare.
fn new_session_id() -> String {
    uuid::Uuid::new_v4().as_bytes()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `iftd` beside this executable first — `tools/bundle.sh` puts it there,
/// next to `ift` and the hook, as a sidecar — then on `PATH`, for `cargo
/// run`/`cargo test` and any dev workflow that never bundled. A missing
/// `iftd` is an error a card can show, never a panic.
fn find_iftd() -> anyhow::Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("iftd");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("iftd");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    anyhow::bail!("iftd not found beside the app or on PATH; cards cannot use the daemon backend")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_id_is_a_filename_and_nothing_else() {
        // It names a socket in a directory we own; a slash or a dot-dot in
        // it would name a file outside that directory.
        let id = new_session_id();
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        assert_ne!(new_session_id(), id, "two cards never collide");
    }

    // A user whose home directory is a few characters longer than
    // `/Users/ekinertac` would otherwise be unable to open any card at
    // all, with nothing but a confusing bind failure from iftd to show for
    // it: this refuses before iftd is even run, with a message naming the
    // path and the limit.
    #[test]
    fn a_socket_path_over_the_sun_path_limit_is_refused_before_it_is_attempted() {
        // Deliberately absurd, so the assertion holds regardless of this
        // machine's actual $TMPDIR length.
        let dir = std::path::PathBuf::from("/tmp").join("x".repeat(120));
        let (backend, _rx) = DaemonBackend::new(dir.clone(), 4);

        let err = backend
            .spawn_now(Path::new("/tmp"), None, vec![])
            .expect_err("a path this long cannot be a unix socket");
        let msg = err.to_string();
        assert!(msg.contains("104"), "{msg}");
        assert!(msg.contains(dir.to_str().unwrap()), "{msg}");
        assert!(
            !dir.exists(),
            "refused before the sessions dir is even created, let alone iftd run"
        );
    }

    // The ring outlives the daemon so a reboot leaves the scrollback; it is
    // taken once, and one no card claims is swept with the orphans.
    #[test]
    fn a_dead_sessions_ring_is_taken_once_and_unclaimed_ones_are_swept() {
        let dir = std::env::temp_dir().join(format!("dmn-r-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (backend, _rx) = DaemonBackend::new(dir.clone(), 4);
        std::fs::write(dir.join("aaaa.ring"), b"old output\r\n").unwrap();
        std::fs::write(dir.join("bbbb.ring"), b"nobody's").unwrap();
        std::fs::write(dir.join("cccc.ring"), b"").unwrap();

        backend.kill_orphans(&["aaaa".to_string()]);
        assert!(
            dir.join("aaaa.ring").exists(),
            "claimed, kept for the replay"
        );
        assert!(!dir.join("bbbb.ring").exists(), "unclaimed, swept");

        let (bytes, when) = backend.take_ring("aaaa").unwrap();
        assert_eq!(bytes, b"old output\r\n");
        assert_eq!(when.len(), "2026-09-18 20:41".len(), "{when}");
        assert_eq!(backend.take_ring("aaaa"), None, "taken means gone");
        assert!(!dir.join("aaaa.ring").exists());
        std::fs::write(dir.join("cccc.ring"), b"").unwrap();
        assert_eq!(
            backend.take_ring("cccc"),
            None,
            "an empty ring is nothing to show"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_sockets_are_not_live_sessions() {
        // Short, for the same reason `new_session_id`'s own doc comment
        // gives: this becomes part of a unix socket path.
        let dir = std::env::temp_dir().join(format!("dmn-t-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("dead.sock"), b"").unwrap();

        // The meta beside it, which is what `ift sessions` reads.
        std::fs::write(dir.join("dead.meta"), b"{}").unwrap();

        assert!(DaemonBackend::live_sessions(&dir).is_empty());
        assert!(!dir.join("dead.sock").exists(), "and it is swept");
        assert!(
            !dir.join("dead.meta").exists(),
            "the meta goes with the socket, or ift sessions keeps offering \
             an attach to a daemon that is gone"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The viewer backend (#423): a window with full control of the host, where
/// the control is carried out BY the host (a close, a new card, a move go up as
/// the canvas and the host applies them), so this backend never ends, starts or
/// sweeps a session itself. Each test has a control on an ordinary backend, so
/// a test that passes cannot be one that never reached the code.
#[cfg(test)]
mod viewer_tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::time::Duration;

    fn frames_until_quiet(mut far: UnixStream, wait: Duration) -> Vec<Frame> {
        // A backend that has already let go of its end (a redraw does) leaves
        // a socket macOS refuses the option on (EINVAL, "peer gone"); the
        // read below then ends at once with what was sent, which is the
        // answer wanted, so the error is no failure. Seen in a busy full run.
        let _ = far.set_read_timeout(Some(wait));
        let mut reader = FrameReader::default();
        let mut frames = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match far.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => reader.feed(&buf[..n]),
            }
            while let Ok(Some(f)) = reader.next() {
                frames.push(f);
            }
        }
        frames
    }

    fn pane_with_peer(backend: &DaemonBackend) -> (PaneId, UnixStream) {
        let (near, far) = UnixStream::pair().unwrap();
        let id = backend.register("s".into(), near, FrameReader::default(), None, (80, 24));
        (id, far)
    }

    fn temp_sessions(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vw-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A stand-in `ssh` that ignores its arguments and runs `body`.
    fn stand_in(name: &str, body: &str) -> super::super::remote::RemoteHost {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_sessions(name).join("ssh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut host = super::super::remote::RemoteHost::new("h");
        host.ssh = path.display().to_string();
        host
    }

    fn reconnect_result(
        backend: &DaemonBackend,
    ) -> (String, Result<PaneId, super::super::remote::JoinFailure>) {
        for _ in 0..200 {
            let mut done = backend.take_reconnected();
            if let Some(one) = done.pop() {
                return one;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the reconnect never ended");
    }

    #[test]
    fn a_reconnect_to_a_host_that_refuses_the_connection_can_be_tried_again() {
        use super::super::remote::JoinFailure;
        let host = stand_in(
            "refuse",
            "echo 'ssh: connect to host h port 22: Connection refused' >&2; exit 255",
        );
        let (backend, _rx) = DaemonBackend::new_viewer(host, 1);
        backend.begin_reconnect("s1");
        let (id, result) = reconnect_result(&backend);
        assert_eq!(id, "s1");
        assert!(matches!(result, Err(JoinFailure::Retry(_))), "{result:?}");
    }

    #[test]
    fn a_reconnect_to_a_session_the_host_no_longer_has_is_final() {
        use super::super::remote::JoinFailure;
        let host = stand_in("gone", "echo 'ift-join 2 error no session s1 on this host'");
        let (backend, _rx) = DaemonBackend::new_viewer(host, 1);
        backend.begin_reconnect("s1");
        let (_, result) = reconnect_result(&backend);
        match result {
            Err(JoinFailure::Fatal(why)) => assert!(why.contains("no session s1"), "{why}"),
            other => panic!("expected a final failure, got {other:?}"),
        }
    }

    #[test]
    fn a_reconnect_that_works_gives_a_pane_and_its_loss_is_told_as_detached() {
        let dir = temp_sessions("drop");
        let hello = Frame::Hello {
            pid: 7,
            cols: 100,
            rows: 30,
        }
        .encode();
        std::fs::write(dir.join("hello.bin"), hello).unwrap();
        // The host answers, greets, and the connection drops: a killed ssh.
        let host = stand_in(
            "drop-ssh",
            &format!(
                "printf 'ift-join 2 ok\\n'; cat '{}'; sleep 0.3",
                dir.join("hello.bin").display()
            ),
        );
        let (backend, rx) = DaemonBackend::new_viewer(host, 1);
        backend.begin_reconnect("s1");
        let (_, result) = reconnect_result(&backend);
        let pane = result.expect("joined");
        assert_eq!(backend.viewer_pane_size(pane), Some((100, 30)));
        // The end of the connection reaches the card as `Detached`, which is
        // what makes it say so and join again.
        let event = rx.recv_timeout(Duration::from_secs(5)).expect("an event");
        assert_eq!(event.0, pane);
        assert!(matches!(event.1, PaneEvent::Detached), "got another event");
    }

    #[test]
    fn only_a_viewer_reconnects() {
        let (backend, _rx) = DaemonBackend::new(temp_sessions("not-a-viewer"), 1);
        backend.begin_reconnect("s1");
        std::thread::sleep(Duration::from_millis(200));
        assert!(backend.take_reconnected().is_empty());
    }

    #[test]
    fn a_viewer_pane_types_and_resizes_like_any_pane() {
        let (backend, _rx) =
            DaemonBackend::new_viewer(super::super::remote::RemoteHost::new("h"), 1);
        let (id, far) = pane_with_peer(&backend);
        backend.write_now(id, b"k");
        backend.resize_now(id, 100, 30);
        backend.detach_now(id);
        let frames = frames_until_quiet(far, Duration::from_secs(2));
        assert_eq!(
            frames,
            vec![
                Frame::Data(b"k".to_vec()),
                Frame::Resize {
                    cols: 100,
                    rows: 30
                }
            ]
        );
    }

    #[test]
    fn closing_a_viewers_pane_disconnects_it_and_the_host_ends_the_card() {
        let (backend, _rx) =
            DaemonBackend::new_viewer(super::super::remote::RemoteHost::new("h"), 1);
        let (id, far) = pane_with_peer(&backend);
        backend.kill_now(id);
        // The far end sees the connection end and no Kill: ending the shell is
        // the host's, done by its own close when the canvas goes up.
        assert!(frames_until_quiet(far, Duration::from_secs(2)).is_empty());
    }

    #[test]
    fn the_same_close_on_an_ordinary_backend_does_send_a_kill() {
        let (backend, _rx) = DaemonBackend::new(temp_sessions("control-pane"), 1);
        let (id, far) = pane_with_peer(&backend);
        backend.write_now(id, b"k");
        backend.resize_now(id, 100, 30);
        backend.kill_now(id);
        let frames = frames_until_quiet(far, Duration::from_millis(400));
        assert_eq!(
            frames,
            vec![
                Frame::Data(b"k".to_vec()),
                Frame::Resize {
                    cols: 100,
                    rows: 30
                },
                Frame::Kill
            ]
        );
    }

    // `terminal.redraw` (#447): on a pane of our own it is the attach's
    // jog, one column narrower and back, in that order.
    #[test]
    fn a_redraw_on_an_ordinary_pane_sends_the_two_resizes() {
        let (backend, _rx) = DaemonBackend::new(temp_sessions("redraw-own"), 1);
        let (id, far) = pane_with_peer(&backend);
        assert_eq!(backend.redraw_now(id), Redraw::Asked);
        backend.detach_now(id);
        assert_eq!(
            frames_until_quiet(far, Duration::from_secs(2)),
            vec![
                Frame::Resize { cols: 79, rows: 24 },
                Frame::Resize { cols: 80, rows: 24 },
            ]
        );
    }

    // The host ignores a viewer's `Resize`, so a viewer's redraw sends none:
    // it lets go of the connection and asks the caller to join again, which
    // replays the ring and makes the host jog the program.
    #[test]
    fn a_redraw_in_a_viewer_lets_go_and_asks_to_join_again() {
        let (backend, _rx) =
            DaemonBackend::new_viewer(super::super::remote::RemoteHost::new("h"), 1);
        let (id, far) = pane_with_peer(&backend);
        assert_eq!(backend.redraw_now(id), Redraw::Rejoin);
        assert!(
            frames_until_quiet(far, Duration::from_secs(2)).is_empty(),
            "no resize goes to the host, and no Kill"
        );
    }

    #[test]
    fn a_redraw_of_a_pane_that_is_gone_does_nothing() {
        let (backend, _rx) = DaemonBackend::new(temp_sessions("redraw-gone"), 1);
        assert_eq!(backend.redraw_now(999), Redraw::Asked);
    }

    #[test]
    fn a_viewer_starts_no_shell_here_the_host_starts_it() {
        let (backend, _rx) =
            DaemonBackend::new_viewer(super::super::remote::RemoteHost::new("h"), 1);
        let err = backend
            .spawn_now(Path::new("/"), None, vec![])
            .unwrap_err()
            .to_string();
        assert!(err.contains("started by the host"), "{err}");
    }

    fn orphan_sweep_reaches(backend: &DaemonBackend, dir: &Path) -> bool {
        let listener = UnixListener::bind(dir.join("zz.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        backend.kill_orphans(&[]);
        for _ in 0..20 {
            if listener.accept().is_ok() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    #[test]
    fn a_viewer_owns_no_session_so_it_sweeps_none_at_launch() {
        let dir = temp_sessions("viewer-sweep");
        let (mut backend, _rx) = DaemonBackend::new(dir.clone(), 1);
        backend.viewer = true;
        assert!(!orphan_sweep_reaches(&backend, &dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_same_sweep_on_an_ordinary_backend_does_reach_a_session() {
        let dir = temp_sessions("control-sweep");
        let (backend, _rx) = DaemonBackend::new(dir.clone(), 1);
        assert!(orphan_sweep_reaches(&backend, &dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
