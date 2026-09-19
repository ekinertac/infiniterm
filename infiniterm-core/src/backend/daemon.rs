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
use super::session_protocol::{Frame, FrameReader};
use super::{PaneEvent, PaneId};
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
        };
        (backend, rx)
    }

    pub fn spawn_now(
        &self,
        cwd: &Path,
        cmd: Option<&str>,
        env: Vec<(String, String)>,
    ) -> anyhow::Result<PaneId> {
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
        let (pid, cols, rows, reader) = read_hello(&mut stream)?;
        Ok(self.register(session_id, stream, reader, Some(pid), (cols, rows)))
    }

    /// Takes over a session an earlier launch left running. `None` when the
    /// socket refuses a connection (a daemon that already exited) or `Hello`
    /// never arrives; the caller spawns a fresh shell instead, same as a
    /// `local_pty` card that never had a session to come back to.
    pub fn adopt(&self, session_id: &str) -> Option<PaneId> {
        let socket = self.sessions_dir.join(format!("{session_id}.sock"));
        let mut stream = UnixStream::connect(&socket).ok()?;
        let (pid, cols, rows, reader) = read_hello(&mut stream).ok()?;
        Some(self.register(
            session_id.to_string(),
            stream,
            reader,
            Some(pid),
            (cols, rows),
        ))
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

        self.panes.lock().unwrap().insert(
            id,
            Pane {
                session_id,
                stream: Arc::new(Mutex::new(write_stream)),
                size: Mutex::new(size),
                pid,
                credit: credit.clone(),
            },
        );

        let tx = self.tx.clone();
        let panes = self.panes.clone();
        std::thread::spawn(move || run_reader(id, stream, reader, tx, panes, credit));

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

    pub fn write_now(&self, pane: PaneId, bytes: &[u8]) {
        let stream = self
            .panes
            .lock()
            .unwrap()
            .get(&pane)
            .map(|p| p.stream.clone());
        if let Some(stream) = stream {
            let mut s = stream.lock().unwrap();
            let _ = s.write_all(&Frame::Data(bytes.to_vec()).encode());
        }
    }

    pub fn resize_now(&self, pane: PaneId, cols: u16, rows: u16) {
        let stream = {
            let panes = self.panes.lock().unwrap();
            let Some(p) = panes.get(&pane) else {
                return;
            };
            *p.size.lock().unwrap() = (cols, rows);
            p.stream.clone()
        };
        let mut s = stream.lock().unwrap();
        let _ = s.write_all(&Frame::Resize { cols, rows }.encode());
    }

    pub fn kill_now(&self, pane: PaneId) {
        let found = {
            let panes = self.panes.lock().unwrap();
            panes
                .get(&pane)
                .map(|p| (p.stream.clone(), p.credit.clone()))
        };
        let Some((stream, credit)) = found else {
            return;
        };
        {
            let mut s = stream.lock().unwrap();
            let _ = s.write_all(&Frame::Kill.encode());
        }
        // Frees a reader parked on credit, same reason and same call as
        // local_pty::kill_now: the pane is removed by the reader thread once
        // Exited (or EOF) actually arrives, not here, so PaneEvent::Exited
        // still fires exactly once.
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
        for id in Self::live_sessions(&self.sessions_dir) {
            if claimed.iter().any(|c| c == &id) {
                continue;
            }
            let socket = self.sessions_dir.join(format!("{id}.sock"));
            if let Ok(mut s) = UnixStream::connect(&socket) {
                let _ = s.write_all(&Frame::Kill.encode());
            }
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
fn read_hello(stream: &mut UnixStream) -> anyhow::Result<(u32, u16, u16, FrameReader)> {
    stream.set_read_timeout(Some(HELLO_POLL))?;
    let mut reader = FrameReader::default();
    let mut buf = [0u8; 8192];
    let deadline = Instant::now() + HELLO_DEADLINE;
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
            anyhow::bail!("no Hello from iftd within {HELLO_DEADLINE:?}");
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
) {
    let mut buf = vec![0u8; 8192];
    let mut exited = false;
    'outer: loop {
        loop {
            match reader.next() {
                Ok(Some(Frame::Data(bytes))) => {
                    credit.sent(bytes.len());
                    if tx.send((id, PaneEvent::Output(bytes))).is_err() {
                        break 'outer;
                    }
                }
                Ok(Some(Frame::Replay(bytes))) => {
                    credit.sent(bytes.len());
                    if tx.send((id, PaneEvent::Replay(bytes))).is_err() {
                        break 'outer;
                    }
                }
                Ok(Some(Frame::ReplayEnd)) => nudge_resize(&panes, id),
                Ok(Some(Frame::Exited(code))) => {
                    let _ = tx.send((id, PaneEvent::Exited { code }));
                    exited = true;
                    break 'outer;
                }
                // Hello only ever arrives once, consumed by read_hello
                // before this thread starts; Resize and Kill only ever
                // travel app-to-daemon. Neither is ours to act on, and
                // dropping the connection over it would be the wrong
                // failure mode for a frame that is merely misdirected.
                Ok(Some(Frame::Hello { .. } | Frame::Resize { .. } | Frame::Kill)) => {}
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
fn check_socket_path(socket: &Path) -> anyhow::Result<()> {
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
