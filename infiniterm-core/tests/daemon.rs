//! Integration test for `backend::daemon::DaemonBackend` against a real,
//! built `iftd` (infiniterm-session, Task 2) — the one thing the in-file
//! unit tests in `daemon.rs` cannot cover, because they run with no daemon
//! at all.
//!
//! Never touches the real data dir or `/tmp/infiniterm.sock`: every socket
//! here lives under a throwaway temp directory this file creates and
//! removes itself, because a real, installed instance of the app may be
//! running on this machine at the same time.
//!
//! Related: `infiniterm_core::backend::daemon` (what is under test),
//! `infiniterm-session/tests/roundtrip.rs` (the same daemon, tested one
//! frame at a time instead of through this client), the design spec's
//! "Testing" section for why this one exists at all.

use infiniterm_core::backend::daemon::DaemonBackend;
use infiniterm_core::backend::PaneEvent;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// Budget for any single "wait for an event" loop below. Generous because
/// this spawns a real shell under a real pty, not a mock.
const DEADLINE: Duration = Duration::from_secs(5);

/// `DaemonBackend::find_iftd` looks beside its OWN `current_exe()` (where
/// `tools/bundle.sh` puts the sidecar) and then on `PATH`. Neither applies
/// to a `cargo test` binary: its `current_exe()` is this test binary under
/// `target/<profile>/deps/`, not the app, and cargo does not put
/// `target/<profile>/` on `PATH`. So this is resolved once, here, and
/// handed to `DaemonBackend` the one way its own search already supports:
/// by prepending iftd's directory to this process's `PATH`.
///
/// `env!("CARGO_BIN_EXE_iftd")` does not work for this: that macro only
/// resolves within the crate that owns the `[[bin]]` (infiniterm-session),
/// and this is `infiniterm-core`.
fn ensure_iftd_on_path() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    static FOUND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    ONCE.call_once(|| {
        if let Some(dir) = find_iftd_dir() {
            let existing = std::env::var_os("PATH").unwrap_or_default();
            let mut paths: Vec<PathBuf> = std::env::split_paths(&existing).collect();
            paths.insert(0, dir);
            if let Ok(joined) = std::env::join_paths(paths) {
                std::env::set_var("PATH", joined);
                FOUND.store(true, Ordering::SeqCst);
            }
        }
    });
    assert!(
        FOUND.load(Ordering::SeqCst),
        "iftd not found next to this test binary's target dir; \
         run `cargo build -p infiniterm-session` first"
    );
}

/// Test binaries live in `target/<profile>/deps/<name>-<hash>`; `iftd`
/// (built by infiniterm-session) lands one directory up, in
/// `target/<profile>/`, alongside `ift` and `infiniterm-hook`.
fn find_iftd_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let deps_dir = exe.parent()?;
    let profile_dir = deps_dir.parent()?;
    profile_dir
        .join("iftd")
        .is_file()
        .then(|| profile_dir.to_path_buf())
}

/// A directory under the OS temp dir that removes itself on drop, killing
/// any `iftd` it started first. Modelled on
/// `infiniterm-session/tests/roundtrip.rs`'s `TempDir`: a daemon outlives
/// its client by design, so a test that fails before its own cleanup would
/// otherwise leave a detached shell running forever on whatever machine ran
/// the suite.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        // Kept SHORT for the same reason infiniterm-session's own
        // roundtrip.rs TempDir is: a unix socket path is capped at
        // sizeof(sockaddr_un.sun_path), 104 bytes on macOS including the
        // NUL, and $TMPDIR is already ~50 of that on this OS.
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("ic-dmn-t-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a fresh temp dir");
        TempDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Ok(entries) = std::fs::read_dir(&self.0) {
            for socket in entries.flatten().map(|e| e.path()) {
                if socket.extension().is_some_and(|e| e == "sock") {
                    for pid in daemons_for(&socket) {
                        // SIGKILL: this runs on a path where a graceful
                        // Kill has already failed to happen (a panic
                        // unwinding past it), so nothing here waits for one
                        // more chance at grace.
                        let _ = std::process::Command::new("kill")
                            .args(["-9", &pid.to_string()])
                            .status();
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Every `iftd` process serving this socket path, by grepping the process
/// table for the `--socket <path>` it was launched with (it never re-execs
/// across its double fork, so the surviving grandchild keeps that argv).
fn daemons_for(socket: &Path) -> Vec<u32> {
    let needle = format!("--socket {}", socket.display());
    let Ok(out) = std::process::Command::new("ps")
        .args(["-axww", "-o", "pid=,command="])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.contains(&needle))
        .filter_map(|line| line.split_whitespace().next()?.parse().ok())
        .collect()
}

fn wait_until(mut pred: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if pred() {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Drains `rx` until an event matching `want` for `pane` arrives, or the
/// deadline passes.
fn wait_for_event(
    rx: &Receiver<(infiniterm_core::backend::PaneId, PaneEvent)>,
    pane: infiniterm_core::backend::PaneId,
    mut want: impl FnMut(&PaneEvent) -> bool,
) -> Option<PaneEvent> {
    let deadline = Instant::now() + DEADLINE;
    while Instant::now() < deadline {
        if let Ok((p, event)) = rx.recv_timeout(Duration::from_millis(200)) {
            if p == pane && want(&event) {
                return Some(event);
            }
        }
    }
    None
}

// The whole point: a card's shell outlives the DaemonBackend that spawned
// it, and a fresh backend on the same sessions directory can reattach and
// see what it missed. Everything else (write/resize/kill/ack) is exercised
// against a live iftd by infiniterm-session's own roundtrip tests; this is
// the one path that is specifically DaemonBackend's to get right, since it
// owns session ids, `Hello`-reading and the replay-triggered resize.
#[test]
fn a_session_survives_the_backend_and_replays_on_reattach() {
    ensure_iftd_on_path();
    let dir = TempDir::new();

    let (backend, rx) = DaemonBackend::new(dir.path().to_path_buf(), 4);
    let pane = backend
        .spawn_now(Path::new("/tmp"), None, vec![])
        .expect("iftd starts");
    backend.write_now(pane, b"echo marker-99\n");
    let saw = wait_for_event(
        &rx,
        pane,
        |e| matches!(e, PaneEvent::Output(b) if String::from_utf8_lossy(b).contains("marker-99")),
    );
    assert!(saw.is_some(), "the live shell echoed what it was told to");

    let session_id = backend
        .session_id(pane)
        .expect("a spawned pane has a session id");
    // Detach: our sockets close, iftd and the shell underneath keep running.
    backend.detach();

    // A second backend, as a relaunch of the app would build, on the same
    // sessions directory.
    let (backend2, rx2) = DaemonBackend::new(dir.path().to_path_buf(), 4);
    let pane2 = backend2
        .adopt(&session_id)
        .expect("the session iftd is still holding is still there to adopt");
    let saw_replay = wait_for_event(
        &rx2,
        pane2,
        |e| matches!(e, PaneEvent::Replay(b) if String::from_utf8_lossy(b).contains("marker-99")),
    );
    assert!(
        saw_replay.is_some(),
        "the ring came back with what the first backend missed seeing torn down"
    );

    backend2.kill_now(pane2);
    assert!(
        wait_until(|| !dir.path().join(format!("{session_id}.sock")).exists()),
        "iftd unlinks its socket once Kill reaches it"
    );
}
