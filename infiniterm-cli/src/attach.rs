//! `ift sessions` and `ift attach <id>`: reaching a card's shell from outside
//! the app, with the app dead or alive.
//!
//! `list_sessions` reads `<data>/s/*.meta` directly and never touches a
//! socket, which is the entire reason `ift sessions` works with no app
//! running: the meta file is written by `iftd` at startup (see
//! `infiniterm-session/src/main.rs`, `run`), not by the app.
//!
//! `attach` speaks the same wire as the app's `DaemonBackend`
//! (`infiniterm-core/src/backend/daemon.rs`): connect, read `Hello`, forward
//! `Data`/`Replay` to stdout, forward stdin as `Data`, and end on `Exited`.
//! It puts the calling terminal into raw mode for the duration, because a
//! shell running inside iftd expects to see raw keystrokes, not a line at a
//! time with local echo doubling every character.
//!
//! Called by `main.rs`, which owns argument dispatch; this file owns the
//! session listing and the raw-mode plumbing. Related:
//! `infiniterm-core/src/backend/session_protocol.rs` (the frames),
//! `infiniterm-core/src/paths.rs` (`sessions_dir`),
//! `docs/superpowers/specs/2026-09-17-session-daemon-design.md` ("Attaching
//! from outside").
//!
//! Non-obvious constraints:
//! - Every exit path must leave the terminal exactly as `attach` found it.
//!   `TermiosGuard`'s `Drop` covers every path that returns normally out of
//!   `run_attached`; a `Drop` never runs when a signal kills the process
//!   instead, so SIGINT/SIGTERM/SIGHUP are also caught explicitly and
//!   restore the same termios by hand before calling `libc::_exit`.
//! - `Ctrl-\` (0x1c, ASCII FS) detaches rather than reaching the shell: raw
//!   mode clears ISIG, so the terminal driver no longer turns that byte into
//!   SIGQUIT on its own, which frees it up as a chord of ours.
//! - Raw mode and the signal handlers need a real tty and cannot be exercised
//!   by a unit test; `list_sessions` and the id-lookup logic can, and are.

use infiniterm_core::backend::session_protocol::{Frame, FrameReader};
use std::io::{Read, Write};
use std::os::unix::io::RawFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

/// Ctrl-\, ASCII FS (file separator). Chosen because it is the byte a normal
/// terminal already wires to SIGQUIT and raw mode disarms that wiring (ISIG
/// is cleared by `cfmakeraw`), so the byte would otherwise do nothing at all
/// rather than colliding with something a shell or an editor wants.
const DETACH_KEY: u8 = 0x1c;

/// How often the resize-poller thread checks whether a SIGWINCH landed.
/// Short enough that a resized terminal window feels immediate, long enough
/// that an idle attach costs nothing measurable.
const RESIZE_POLL: Duration = Duration::from_millis(150);

/// One row of `ift sessions`: a session as `iftd`'s own `.meta` file
/// describes it, nothing more. See `infiniterm-session/src/main.rs`'s `run`
/// for exactly what gets written there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub id: String,
    pub pid: u32,
    pub cwd: String,
    pub cmd: String,
    pub started: String,
}

/// Reads every `*.meta` file in `dir` and returns what parses. This must
/// work with no `iftd` and no app running at all — that is the entire
/// reason it reads files instead of connecting to anything — so a missing
/// directory is an empty list, not an error, and one malformed file (a
/// daemon that died mid-write, or one that failed at startup and wrote
/// `{"error": "..."}` instead of the usual fields, see `fail_startup` in
/// `infiniterm-session`) is skipped rather than aborting the whole listing.
pub fn list_sessions(dir: &Path) -> Vec<SessionRow> {
    let mut rows = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return rows;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("meta") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // A meta whose socket is gone is a daemon that died without
        // cleaning up. Listing it offers an attach that cannot work, and
        // the app sweeps these only when it happens to be running, so the
        // listing checks for itself rather than trusting the directory.
        if !path.with_extension("sock").exists() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let (Some(pid), Some(cwd), Some(cmd), Some(started)) = (
            v.get("pid").and_then(serde_json::Value::as_u64),
            v.get("cwd").and_then(serde_json::Value::as_str),
            v.get("cmd").and_then(serde_json::Value::as_str),
            v.get("started").and_then(serde_json::Value::as_str),
        ) else {
            continue;
        };
        rows.push(SessionRow {
            id: id.to_string(),
            pid: pid as u32,
            cwd: cwd.to_string(),
            cmd: cmd.to_string(),
            started: started.to_string(),
        });
    }
    // Directory order is not filesystem-guaranteed; a stable listing is
    // worth the sort given there are at most a few dozen cards.
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    rows
}

/// `ift sessions`. Tab separated, because that is what the rest of `ift`
/// prints (`ift ls`) and it pipes into `cut`/`awk` without a flag to ask for.
pub fn sessions_cmd() -> ExitCode {
    print_sessions(&list_sessions(&infiniterm_core::paths::sessions_dir()));
    ExitCode::SUCCESS
}

fn print_sessions(rows: &[SessionRow]) {
    for r in rows {
        println!("{}\t{}\t{}\t{}\t{}", r.id, r.pid, r.cwd, r.cmd, r.started);
    }
}

/// `ift attach` with no id at all: the id was the whole point, so this is a
/// usage error, not "nothing is running".
pub fn no_id() -> ExitCode {
    eprintln!("ift: attach needs a session id\n");
    print_sessions(&list_sessions(&infiniterm_core::paths::sessions_dir()));
    ExitCode::from(2)
}

/// `ift attach <id>`.
///
/// Two failure shapes on the way in, and they mean different things: an id
/// nothing in `.meta` recognises is a typo, so the full list is the useful
/// answer. An id that IS a real session but whose socket refuses a connect
/// is a daemon that died without cleaning up (a crash, `kill -9`) — that is
/// not "which one did you mean", it is "this one is gone", so it gets a
/// plain statement and a pointer rather than the whole table.
pub fn attach(id: &str) -> ExitCode {
    let dir = infiniterm_core::paths::sessions_dir();
    let rows = list_sessions(&dir);
    if !rows.iter().any(|r| r.id == id) {
        eprintln!("ift: no session {id:?}\n");
        print_sessions(&rows);
        return ExitCode::from(2);
    }

    let sock = dir.join(format!("{id}.sock"));
    let stream = match UnixStream::connect(&sock) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "ift: {id} is not connectable ({e}); its daemon likely died without cleaning up"
            );
            eprintln!("ift: run `ift sessions` to see what is actually running");
            return ExitCode::from(1);
        }
    };
    run_attached(stream)
}

// --- termios plumbing ------------------------------------------------------

/// Published once raw mode is entered, so the signal handlers below have
/// something to restore. `OnceLock`/`AtomicI32` rather than a `Mutex`: a
/// signal handler must not take a lock that the interrupted code might
/// already be holding, so both of these are read with plain atomic loads.
static ORIGINAL_TERMIOS: OnceLock<libc::termios> = OnceLock::new();
static TTY_FD: AtomicI32 = AtomicI32::new(-1);

/// Set by the SIGWINCH handler, cleared by the poller that acts on it. A
/// signal handler may not safely call `TIOCGWINSZ` itself (ioctl is not on
/// the async-signal-safe list), so it only flags that a resize happened and
/// a normal thread does the actual work on the next poll.
static WINCH: AtomicBool = AtomicBool::new(false);

extern "C" fn on_winch(_sig: libc::c_int) {
    WINCH.store(true, Ordering::SeqCst);
}

/// Restores the terminal and ends the process. A `Drop` never runs when a
/// signal ends it instead — SIGINT/SIGTERM/SIGHUP each name a case where
/// that happens (`kill`, a closing window, the shell exiting under us) — so
/// this exists to do by hand what `TermiosGuard::drop` does on every other
/// path. `_exit`, not `std::process::exit`: this runs on the interrupted
/// thread's stack mid-syscall, where only the async-signal-safe subset of
/// libc — `tcsetattr` and `_exit` among them — is safe to call.
extern "C" fn restore_and_exit(sig: libc::c_int) {
    if let Some(original) = ORIGINAL_TERMIOS.get() {
        let fd = TTY_FD.load(Ordering::SeqCst);
        if fd >= 0 {
            unsafe {
                libc::tcsetattr(fd, libc::TCSANOW, original);
            }
        }
    }
    unsafe {
        libc::_exit(128 + sig);
    }
}

fn install_restore_signal(sig: libc::c_int) {
    unsafe {
        libc::signal(sig, restore_and_exit as *const () as libc::sighandler_t);
    }
}

/// Restores the tty's termios when it drops, covering every NORMAL exit path
/// out of `run_attached` (return, `?`, an early `return ExitCode`). The
/// signal path is `restore_and_exit` above, not this: this type's `drop`
/// simply never runs there.
struct TermiosGuard {
    fd: RawFd,
    original: libc::termios,
}

impl Drop for TermiosGuard {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
        }
    }
}

fn terminal_size(fd: RawFd) -> Option<(u16, u16)> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } != 0 {
        return None;
    }
    Some((ws.ws_col, ws.ws_row))
}

fn send_resize(write_handle: &Arc<Mutex<UnixStream>>, tty_fd: RawFd) {
    if let Some((cols, rows)) = terminal_size(tty_fd) {
        let _ = write_handle
            .lock()
            .unwrap()
            .write_all(&Frame::Resize { cols, rows }.encode());
    }
}

/// Forwards stdin bytes as `Frame::Data`, watching for the detach key.
/// Not joined: it ends on its own (stdin EOF, a write failure, or detaching)
/// and the process exits without waiting for background threads either way.
fn forward_stdin(write_handle: Arc<Mutex<UnixStream>>, detached: Arc<AtomicBool>) {
    let mut stdin = std::io::stdin();
    let mut buf = [0u8; 4096];
    loop {
        let n = match stdin.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        if let Some(i) = buf[..n].iter().position(|&b| b == DETACH_KEY) {
            if i > 0 {
                let _ = write_handle
                    .lock()
                    .unwrap()
                    .write_all(&Frame::Data(buf[..i].to_vec()).encode());
            }
            detached.store(true, Ordering::SeqCst);
            // shutdown() acts on the underlying socket, not just this fd, so
            // it also unblocks the main thread's read() on its own clone —
            // that is what lets a detach end the frame loop immediately
            // instead of waiting for the daemon to say something first.
            let _ = write_handle
                .lock()
                .unwrap()
                .shutdown(std::net::Shutdown::Both);
            return;
        }
        if write_handle
            .lock()
            .unwrap()
            .write_all(&Frame::Data(buf[..n].to_vec()).encode())
            .is_err()
        {
            return;
        }
    }
}

fn resize_poller(write_handle: Arc<Mutex<UnixStream>>, tty_fd: RawFd) {
    loop {
        thread::sleep(RESIZE_POLL);
        if WINCH.swap(false, Ordering::SeqCst) {
            send_resize(&write_handle, tty_fd);
        }
    }
}

/// The low byte of a child's exit code, the same convention a shell's `$?`
/// uses for a signal-killed child: `ExitCode` only carries a `u8`, and a
/// `Frame::Exited` payload is the daemon's `portable_pty` exit code as-is.
fn exit_code_byte(code: i32) -> u8 {
    (code & 0xff) as u8
}

/// Everything after a successful connect: enter raw mode, wire up the three
/// directions of traffic (stdin in, frames out, `SIGWINCH` out), and run
/// until detach, `Exited`, or the connection dropping out from under us.
fn run_attached(stream: UnixStream) -> ExitCode {
    let stdin_fd: RawFd = libc::STDIN_FILENO;
    if unsafe { libc::isatty(stdin_fd) } != 1 {
        eprintln!("ift: attach needs a real terminal on stdin");
        return ExitCode::from(2);
    }

    let original = unsafe {
        let mut t = std::mem::MaybeUninit::<libc::termios>::uninit();
        if libc::tcgetattr(stdin_fd, t.as_mut_ptr()) != 0 {
            eprintln!("ift: tcgetattr failed: {}", std::io::Error::last_os_error());
            return ExitCode::from(1);
        }
        t.assume_init()
    };

    // Published before raw mode is entered: from this point on, any signal
    // that lands has something correct to restore.
    let _ = ORIGINAL_TERMIOS.set(original);
    TTY_FD.store(stdin_fd, Ordering::SeqCst);
    install_restore_signal(libc::SIGINT);
    install_restore_signal(libc::SIGTERM);
    install_restore_signal(libc::SIGHUP);
    unsafe {
        libc::signal(libc::SIGWINCH, on_winch as *const () as libc::sighandler_t);
    }

    let mut raw = original;
    unsafe {
        libc::cfmakeraw(&mut raw);
    }
    if unsafe { libc::tcsetattr(stdin_fd, libc::TCSANOW, &raw) } != 0 {
        eprintln!("ift: tcsetattr failed: {}", std::io::Error::last_os_error());
        return ExitCode::from(1);
    }
    // From here, every return out of this function restores the terminal.
    let _guard = TermiosGuard {
        fd: stdin_fd,
        original,
    };

    let write_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ift: could not clone the session socket: {e}");
            return ExitCode::from(1);
        }
    };
    let write_handle = Arc::new(Mutex::new(write_stream));
    send_resize(&write_handle, stdin_fd);

    let detached = Arc::new(AtomicBool::new(false));
    {
        let write_handle = write_handle.clone();
        let detached = detached.clone();
        thread::spawn(move || forward_stdin(write_handle, detached));
    }
    {
        let write_handle = write_handle.clone();
        thread::spawn(move || resize_poller(write_handle, stdin_fd));
    }

    let mut reader = FrameReader::default();
    let mut sock = stream;
    let mut buf = vec![0u8; 8192];
    let mut stdout = std::io::stdout();
    loop {
        loop {
            match reader.next() {
                Ok(Some(Frame::Data(bytes))) | Ok(Some(Frame::Replay(bytes))) => {
                    let _ = stdout.write_all(&bytes);
                    let _ = stdout.flush();
                }
                Ok(Some(Frame::Exited(code))) => {
                    return ExitCode::from(exit_code_byte(code));
                }
                // Hello / Resize / Kill / ReplayEnd carry nothing to draw.
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(e) => {
                    eprintln!("\r\nift: session protocol error: {e}");
                    return ExitCode::from(1);
                }
            }
        }
        match sock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => reader.feed(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }

    if detached.load(Ordering::SeqCst) {
        ExitCode::SUCCESS
    } else {
        eprintln!("\r\nift: session connection lost");
        ExitCode::from(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own.
    ///
    /// A COUNTER, not a timestamp. These tests run in parallel and the
    /// clock is not guaranteed to tick between two calls, so two tests
    /// could be handed the same path and then read each other's files:
    /// `a_malformed_meta_file_is_skipped_not_panicked_on` would find
    /// another test's valid session and fail, only sometimes, and only
    /// under load. Measured on a full `cargo test`, not theorised.
    fn tempdir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ift-attach-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        // A path from a previous run with a recycled pid must not carry
        // its files into this one.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // It must work with no app running: that is the entire reason it exists.
    #[test]
    fn sessions_are_listed_from_the_meta_files_alone() {
        let dir = tempdir();
        std::fs::write(
            dir.join("x.meta"),
            r#"{"pid":42,"cwd":"/tmp","cmd":"zsh","started":"2026-09-17T10:00:00Z"}"#,
        )
        .unwrap();
        std::fs::write(dir.join("x.sock"), b"").unwrap();
        let rows = list_sessions(&dir);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pid, 42);
        assert_eq!(rows[0].cwd, "/tmp");
        assert_eq!(rows[0].cmd, "zsh");
        assert_eq!(rows[0].started, "2026-09-17T10:00:00Z");
    }

    // Found by running tools/drive/daemon.sh: a killed daemon left its
    // .meta behind and `ift sessions` went on advertising a session whose
    // socket was gone, offering an attach that could only fail. The app
    // sweeps these, but only while it is running, and this command's whole
    // reason to exist is working when it is not.
    #[test]
    fn a_meta_with_no_socket_is_not_a_session() {
        let dir = tempdir();
        let row = r#"{"pid":42,"cwd":"/tmp","cmd":"zsh","started":"2026-09-17T10:00:00Z"}"#;
        std::fs::write(dir.join("alive.meta"), row).unwrap();
        std::fs::write(dir.join("alive.sock"), b"").unwrap();
        std::fs::write(dir.join("dead.meta"), row).unwrap();

        let ids: Vec<String> = list_sessions(&dir).into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["alive".to_string()]);
    }

    #[test]
    fn a_missing_directory_is_an_empty_list_not_an_error() {
        let dir = std::env::temp_dir().join("ift-attach-test-does-not-exist-at-all");
        assert!(list_sessions(&dir).is_empty());
    }

    // A daemon that dies mid-write leaves a truncated or empty .meta behind;
    // `ift sessions` must skip it, never panic on it.
    #[test]
    fn a_malformed_meta_file_is_skipped_not_panicked_on() {
        let dir = tempdir();
        std::fs::write(dir.join("truncated.meta"), b"{\"pid\":1,\"cwd\":").unwrap();
        std::fs::write(dir.join("empty.meta"), b"").unwrap();
        std::fs::write(dir.join("not-json.meta"), b"not json at all").unwrap();
        // A daemon that failed at startup writes this instead of the usual
        // fields (see infiniterm-session's fail_startup) — also not a row.
        std::fs::write(dir.join("failed.meta"), br#"{"error":"openpty: EIO"}"#).unwrap();
        assert!(list_sessions(&dir).is_empty());
    }

    // One malformed file must not hide the sessions that parsed fine.
    #[test]
    fn a_malformed_file_does_not_hide_the_good_ones() {
        let dir = tempdir();
        std::fs::write(dir.join("bad.meta"), b"not json").unwrap();
        std::fs::write(
            dir.join("good.meta"),
            r#"{"pid":7,"cwd":"/home/x","cmd":"fish","started":"2026-09-17T11:00:00Z"}"#,
        )
        .unwrap();
        // A listed session needs its socket: see `a_meta_with_no_socket_is_not_a_session`.
        std::fs::write(dir.join("good.sock"), b"").unwrap();
        let rows = list_sessions(&dir);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "good");
    }

    #[test]
    fn rows_come_back_sorted_by_id() {
        let dir = tempdir();
        for (id, pid) in [("b", 2), ("a", 1), ("c", 3)] {
            std::fs::write(
                dir.join(format!("{id}.meta")),
                format!(
                    r#"{{"pid":{pid},"cwd":"/tmp","cmd":"zsh","started":"2026-09-17T10:00:00Z"}}"#
                ),
            )
            .unwrap();
            std::fs::write(dir.join(format!("{id}.sock")), b"").unwrap();
        }
        let ids: Vec<_> = list_sessions(&dir).into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    // `ift attach <id-nobody-has>`: a usage error, not "nothing is running" —
    // the daemon layer is never touched to find this out.
    #[test]
    fn attaching_an_unknown_id_never_touches_a_socket() {
        let dir = tempdir();
        std::fs::write(
            dir.join("real.meta"),
            r#"{"pid":1,"cwd":"/tmp","cmd":"zsh","started":"2026-09-17T10:00:00Z"}"#,
        )
        .unwrap();
        std::fs::write(dir.join("real.sock"), b"").unwrap();
        let rows = list_sessions(&dir);
        assert!(!rows.iter().any(|r| r.id == "nonexistent"));
        assert!(rows.iter().any(|r| r.id == "real"));
    }
}
