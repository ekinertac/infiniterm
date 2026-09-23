//! `iftd`: one card's shell, outliving the app.
//!
//! Invoked as `iftd --socket <path> --cwd <dir> [--cmd <s>] [--buffer <MiB>]
//! [--env K=V]...`. It binds the unix socket, forks itself into a daemon,
//! and the process the app launched exits 0 the instant that socket is
//! connectable — there is no readiness race for the app to poll for.
//!
//! It does NO VT parsing, which is the entire reason it exists rather than
//! tmux: bytes come off the pty, go into a fixed-size ring and out to
//! whichever client is attached, unchanged. `session_protocol.rs` is the
//! wire and the ring; `backend/daemon.rs` (Task 3, not yet built) is the
//! app's end of this socket; `local_pty.rs` is where the exact same child
//! is spawned when there is no daemon in the path, and this file is meant
//! to build its command line the same way.
//!
//! Related: docs/superpowers/specs/2026-09-17-session-daemon-design.md,
//! docs/windows-handoff.md for why there is no Windows version of this.
//!
//! Non-obvious constraints:
//! - The listener MUST be bound before either fork. The app's proof that
//!   the socket is connectable is this process exiting 0, and that proof
//!   is worthless if the bind happens after the exit.
//! - Once daemonised, nothing here may assume stderr reaches anyone: the
//!   process that launched us is already gone. Startup failures from this
//!   point on are reported through the `.meta` file instead, which `ift
//!   sessions` (Task 6) can read with no app running at all.
//! - There is no ack frame anywhere in this file, deliberately: the socket
//!   write blocking IS the backpressure. See session_protocol.rs's header.

use infiniterm_core::backend::local_pty::{terminal_identity, INHERITED_TERMINAL_VARS};
use infiniterm_core::backend::session_protocol::{Frame, FrameReader, Ring, MAX_PAYLOAD};
use infiniterm_core::shell_cmd::{default_shell, shell_args};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A pane starts at the same size `local_pty::spawn_now` uses, before the
/// first real size is known. The client resizes it the moment it attaches.
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;

/// The ring's default size when `--buffer` is not given. Matches the
/// `terminal.sessionBuffer` default (Task 4); this flag exists independent
/// of that config so `iftd` is a complete, testable program on its own.
const DEFAULT_BUFFER_MIB: usize = 4;

/// How often the ring is written to `<id>.ring` beside the socket, when
/// it changed. This is what survives a power cut: the daemon keeps the
/// shell across an app restart, but nothing keeps it across a reboot, and
/// the app replays this file into a card whose session is gone so 23 cards
/// do not all come back as a bare prompt. Two seconds bounds what a cut
/// costs; a busy card rewrites at most its ring every two seconds, an idle
/// one nothing. fsync'd, or the page cache keeps it from the disk.
const SNAPSHOT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// How long the child waiter thread will wait to tell an attached client
/// the pane exited before giving up and cleaning up anyway. The child is
/// already dead by the time this fires; a client too stalled to take a few
/// bytes within this window must not be able to wedge the daemon's exit.
const EXIT_NOTICE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

pub fn run() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match Options::parse(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("iftd: {e}");
            std::process::exit(2);
        }
    };

    // A socket left behind by a daemon that crashed without cleaning up is
    // stale, not busy: bind() would otherwise refuse a path that already
    // exists, mistaking "file present" for "someone is listening".
    if let Err(e) = clear_stale_socket(&opts.socket) {
        eprintln!("iftd: {e}");
        std::process::exit(3);
    }

    // Bind BEFORE any fork: see the header. This is what lets the app treat
    // its wait() on us as the readiness signal.
    let listener = match UnixListener::bind(&opts.socket) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("iftd: failed to bind {}: {e}", opts.socket.display());
            std::process::exit(4);
        }
    };

    daemonize();
    detach_stdio();

    // Past this line nobody is reading our stderr: the process the app
    // waited on has already exited 0. `run` reports its own failures by
    // writing `opts.meta_path()` instead.
    run(listener, opts);
}

/// Everything `iftd` was told to do, parsed from argv. A hand-rolled parser
/// rather than a dependency: the surface is five flags and this is the only
/// binary in the workspace that needs any of them (`infiniterm-cli`'s
/// argument parsing note explains the same call when `ift attach` grows
/// raw-mode flags in Task 6).
struct Options {
    socket: PathBuf,
    cwd: PathBuf,
    cmd: Option<String>,
    buffer_mib: usize,
    env: Vec<(String, String)>,
}

impl Options {
    fn parse(args: &[String]) -> Result<Options, String> {
        let mut socket = None;
        let mut cwd = None;
        let mut cmd = None;
        let mut buffer_mib = DEFAULT_BUFFER_MIB;
        let mut env = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let flag = args[i].as_str();
            let mut take = |name: &str| -> Result<String, String> {
                i += 1;
                args.get(i)
                    .cloned()
                    .ok_or_else(|| format!("{name} needs a value"))
            };
            match flag {
                "--socket" => socket = Some(PathBuf::from(take("--socket")?)),
                "--cwd" => cwd = Some(PathBuf::from(take("--cwd")?)),
                "--cmd" => cmd = Some(take("--cmd")?),
                "--buffer" => {
                    let v = take("--buffer")?;
                    buffer_mib = v
                        .parse()
                        .map_err(|_| format!("--buffer wants a number of MiB, got {v:?}"))?;
                }
                "--env" => {
                    let v = take("--env")?;
                    let (k, val) = v
                        .split_once('=')
                        .ok_or_else(|| format!("--env wants K=V, got {v:?}"))?;
                    env.push((k.to_string(), val.to_string()));
                }
                other => return Err(format!("unknown flag {other:?}")),
            }
            i += 1;
        }

        Ok(Options {
            socket: socket.ok_or("--socket is required")?,
            cwd: cwd.ok_or("--cwd is required")?,
            cmd,
            buffer_mib,
            env,
        })
    }

    /// The meta file sits beside the socket with the same stem: `ift
    /// sessions` (Task 6) pairs them up by replacing this same extension.
    fn meta_path(&self) -> PathBuf {
        self.socket.with_extension("meta")
    }
}

/// If `socket` exists and nothing answers a connect, it is a leftover from a
/// daemon that died without unlinking it (a crash, `kill -9`); remove it so
/// `bind` sees a free path. If something DOES answer, a daemon is already
/// serving this card and we must not steal its socket out from under it.
fn clear_stale_socket(socket: &Path) -> Result<(), String> {
    if !socket.exists() {
        return Ok(());
    }
    if UnixStream::connect(socket).is_ok() {
        return Err(format!("{} is already live", socket.display()));
    }
    std::fs::remove_file(socket).map_err(|e| {
        format!(
            "could not remove the stale socket {}: {e}",
            socket.display()
        )
    })
}

/// Two forks, in this order, for two different reasons:
/// 1. The first fork's PARENT exits 0 immediately. That is what frees the
///    app's `Command::status()` wait — the whole reason the bind above runs
///    first, so the exit is proof the socket already works.
/// 2. `setsid()` moves the child out of the app's session, so a closing
///    terminal window cannot HUP this process along with it.
/// 3. The second fork's PARENT (the new session leader) also exits. A
///    session leader can reacquire a controlling terminal by opening one;
///    forking again and exiting leaves a process that structurally cannot.
fn daemonize() {
    unsafe {
        match libc::fork() {
            -1 => {
                eprintln!("iftd: fork failed");
                std::process::exit(5);
            }
            0 => {}
            _ => std::process::exit(0),
        }
        if libc::setsid() == -1 {
            eprintln!("iftd: setsid failed");
        }
        match libc::fork() {
            -1 => std::process::exit(5),
            0 => {}
            _ => std::process::exit(0),
        }
    }
}

/// Redirects stdin/stdout/stderr to `/dev/null`, once daemonising is done
/// and nothing here still needs to report an error to whoever launched us.
///
/// This is not just cosmetic. Without it, a daemon that outlives its
/// launcher (the whole point) keeps its inherited fds open too — including,
/// on the app's side, whatever the app's own stdout/stderr were connected
/// to. Anything downstream reading that stream to EOF (a shell pipeline, a
/// test harness collecting a subprocess's output) then blocks forever,
/// because our still-running daemon is a second writer nobody told it
/// about. The listener fd is never touched here: it is a different fd
/// number, opened well before this call.
fn detach_stdio() {
    unsafe {
        let devnull = std::ffi::CString::new("/dev/null").expect("no interior NUL");
        let fd = libc::open(devnull.as_ptr(), libc::O_RDWR);
        if fd < 0 {
            return;
        }
        libc::dup2(fd, libc::STDIN_FILENO);
        libc::dup2(fd, libc::STDOUT_FILENO);
        libc::dup2(fd, libc::STDERR_FILENO);
        if fd > libc::STDERR_FILENO {
            libc::close(fd);
        }
    }
}

/// Writes `{"error": "..."}` to the meta file and exits 1. The only way a
/// startup failure past `daemonize()` can be seen at all: stderr no longer
/// reaches anything, but the meta file's directory is the same one `ift
/// sessions` already scans.
fn fail_startup(meta_path: &Path, socket_path: &Path, err: impl std::fmt::Display) -> ! {
    let body = serde_json::json!({ "error": err.to_string() }).to_string();
    let _ = std::fs::write(meta_path, body);
    let _ = std::fs::remove_file(socket_path);
    std::process::exit(1);
}

/// Opens the pty, spawns the child exactly as `local_pty::spawn_now` would,
/// and runs the four threads described in the plan (pty reader, accept,
/// child waiter — the fourth, the client reader, is spawned per attach by
/// the accept loop below). Never returns: the process ends either via
/// `fail_startup` or when the child waiter calls `std::process::exit`.
fn run(listener: UnixListener, opts: Options) -> ! {
    let meta_path = opts.meta_path();

    let pair = match native_pty_system().openpty(PtySize {
        rows: INITIAL_ROWS,
        cols: INITIAL_COLS,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(p) => p,
        Err(e) => fail_startup(&meta_path, &opts.socket, format!("openpty: {e}")),
    };

    // Same construction as local_pty::spawn_now, through the same function:
    // the shell's own run flags for a one-shot command so the user's profile
    // loads, the parent terminal's identity scrubbed before this card's own
    // is applied, then the daemon's own --env pairs.
    let shell = default_shell();
    let mut builder = CommandBuilder::new(&shell);
    builder.args(shell_args(&shell, opts.cmd.as_deref()));
    builder.cwd(&opts.cwd);
    for k in INHERITED_TERMINAL_VARS {
        builder.env_remove(k);
    }
    for (k, v) in terminal_identity() {
        builder.env(k, v);
    }
    for (k, v) in &opts.env {
        builder.env(k, v);
    }

    let mut child = match pair.slave.spawn_command(builder) {
        Ok(c) => c,
        Err(e) => fail_startup(&meta_path, &opts.socket, format!("spawn: {e}")),
    };
    // Cloned before `child` moves into the waiter thread below, same reason
    // as local_pty.rs: kill() must be able to reach the child from another
    // thread once `child.wait()` owns `child` itself.
    let killer: Arc<Mutex<Box<dyn ChildKiller + Send + Sync>>> =
        Arc::new(Mutex::new(child.clone_killer()));
    let pid = child.process_id().unwrap_or(0);

    let mut pty_reader = match pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => fail_startup(&meta_path, &opts.socket, format!("try_clone_reader: {e}")),
    };
    let pty_writer: Arc<Mutex<Box<dyn Write + Send>>> = match pair.master.take_writer() {
        Ok(w) => Arc::new(Mutex::new(w)),
        Err(e) => fail_startup(&meta_path, &opts.socket, format!("take_writer: {e}")),
    };
    let master: Arc<Mutex<Box<dyn MasterPty + Send>>> = Arc::new(Mutex::new(pair.master));

    // Rewritten with `attached` flipped on every attach and detach: the app
    // reads it to know when an `ift attach` that displaced it has let go,
    // so it can take the session back (see `DaemonBackend::session_attached`).
    let meta = Arc::new(Meta {
        path: meta_path.clone(),
        pid,
        cwd: opts.cwd.display().to_string(),
        cmd: opts.cmd.clone().unwrap_or_else(default_shell),
        started: rfc3339_now(),
    });
    if let Err(e) = meta.write(false) {
        fail_startup(&meta_path, &opts.socket, format!("writing meta: {e}"));
    }

    let ring = Arc::new(Mutex::new(Ring::new(opts.buffer_mib * 1024 * 1024)));
    let client: Arc<Mutex<Option<ClientSlot>>> = Arc::new(Mutex::new(None));
    let size = Arc::new(Mutex::new((INITIAL_COLS, INITIAL_ROWS)));
    let next_epoch = Arc::new(AtomicU64::new(1));

    // pty reader: the only thread that ever reads the child's output. Its
    // read-then-maybe-blocking-write shape IS the backpressure described in
    // session_protocol.rs's header — nothing else needs to implement it.
    {
        let ring = ring.clone();
        let client = client.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 8192];
            loop {
                match pty_reader.read(&mut buf) {
                    Ok(0) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                    Ok(n) => {
                        ring.lock().unwrap().push(&buf[..n]);
                        forward_to_client(&client, &Frame::Data(buf[..n].to_vec()));
                    }
                }
            }
        });
    }

    // snapshot: the ring to disk, tmp then rename so a reader never sees
    // half a file. See SNAPSHOT_INTERVAL. The normal exit below removes
    // the file: a shell that ended has nothing to salvage.
    let ring_path = opts.socket.with_extension("ring");
    {
        let ring = ring.clone();
        let ring_path = ring_path.clone();
        std::thread::spawn(move || {
            let mut written = 0u64;
            loop {
                std::thread::sleep(SNAPSHOT_INTERVAL);
                let (generation, bytes) = {
                    let r = ring.lock().unwrap();
                    if r.generation() == written {
                        continue;
                    }
                    (r.generation(), r.replay())
                };
                if write_snapshot(&ring_path, &bytes).is_ok() {
                    written = generation;
                }
            }
        });
    }

    // child waiter: the one thread that owns `child`, so a long wait never
    // blocks the reader or the accept loop. It is the sole place the
    // process actually ends, so a socket left bound with no cleanup is only
    // possible if the daemon is killed with a signal it cannot catch.
    {
        let client = client.clone();
        let socket_path = opts.socket.clone();
        let meta_path = meta_path.clone();
        let ring_path = ring_path.clone();
        std::thread::spawn(move || {
            let code = child.wait().map(|s| s.exit_code() as i32).unwrap_or(-1);
            if let Some(slot) = client.lock().unwrap().take() {
                let mut s = slot.stream;
                // Bounded: a client whose socket buffer is already full (it
                // stopped reading a while ago, same as `forward_to_client`
                // would see) must not hold up cleanup below. Unlike a live
                // pane's output, the child is already gone — nothing is
                // lost by giving up on this one notification, and the
                // client learns the pane is gone anyway when the socket
                // disappears out from under it.
                let _ = s.set_write_timeout(Some(EXIT_NOTICE_TIMEOUT));
                let _ = s.write_all(&Frame::Exited(code).encode());
            }
            let _ = std::fs::remove_file(&socket_path);
            let _ = std::fs::remove_file(&meta_path);
            let _ = std::fs::remove_file(&ring_path);
            std::process::exit(0);
        });
    }

    // accept: one client at a time. A new connection displaces whatever was
    // there — fan-out would mean per-client backpressure, which is the hole
    // the one-daemon-per-card design exists to avoid (see the spec).
    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(s) => s,
            Err(_) => continue,
        };
        let epoch = next_epoch.fetch_add(1, Ordering::SeqCst);

        // Evict whoever was attached. shutdown(Both) — not drop — because a
        // dup'd fd (the old client-reader thread's) is what actually needs
        // to wake from its blocked read; closing just this handle would
        // leave that other descriptor, and the thread reading through it,
        // untouched.
        if let Some(old) = client.lock().unwrap().take() {
            let _ = old.stream.shutdown(Shutdown::Both);
        }

        let (cols, rows) = *size.lock().unwrap();
        let mut greet = match stream.try_clone() {
            Ok(s) => s,
            Err(_) => continue,
        };
        if greet
            .write_all(&Frame::Hello { pid, cols, rows }.encode())
            .is_err()
        {
            continue;
        }
        let replay = ring.lock().unwrap().replay();
        let mut ok = true;
        for chunk in replay.chunks(MAX_PAYLOAD) {
            if greet
                .write_all(&Frame::Replay(chunk.to_vec()).encode())
                .is_err()
            {
                ok = false;
                break;
            }
        }
        if ok {
            let _ = greet.write_all(&Frame::ReplayEnd.encode());
        }

        let read_handle = match stream.try_clone() {
            Ok(s) => s,
            Err(_) => continue,
        };
        *client.lock().unwrap() = Some(ClientSlot { epoch, stream });
        let _ = meta.write(true);

        let client = client.clone();
        let pty_writer = pty_writer.clone();
        let master = master.clone();
        let size = size.clone();
        let killer = killer.clone();
        let meta = meta.clone();
        let epochs = next_epoch.clone();
        std::thread::spawn(move || {
            run_client_reader(read_handle, epoch, client, pty_writer, master, size, killer);
            // Only when no newer attach has begun: the slot is empty during
            // a newcomer's handshake too, so the slot cannot say, but the
            // epoch counter can. A newer attach writes `true` for itself.
            if epochs.load(Ordering::SeqCst) == epoch + 1 {
                let _ = meta.write(false);
            }
        });
    }

    // listener.incoming() only ends if the listener itself errors out
    // (never happens in practice; nothing here closes it). Treated as fatal
    // rather than looping forever on a dead accept.
    std::process::exit(1);
}

/// The current client, if any, tagged with an epoch so a thread that just
/// lost a race (its write failed, or it hit EOF) can tell whether it is
/// clearing the client IT owned or one that has already replaced it. Plain
/// fd comparison would have the same bug once a closed fd number gets
/// reused by the very next accept.
struct ClientSlot {
    epoch: u64,
    stream: UnixStream,
}

/// Sends `frame` to whoever is attached, if anyone. A write failure means
/// that client detached (or was disconnected) — completely normal, so the
/// client is cleared and reading continues; it is never a reason to stop
/// draining the pty, or a shell nothing is watching would fill the kernel's
/// pty buffer and stall.
fn forward_to_client(client: &Arc<Mutex<Option<ClientSlot>>>, frame: &Frame) {
    let handle = client
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| (c.epoch, c.stream.try_clone()));
    let Some((epoch, Ok(mut stream))) = handle else {
        return;
    };
    // Written outside the lock: this call is exactly where the design's
    // backpressure lives (a client that stops reading makes this block),
    // and holding the mutex across it would also freeze the accept loop's
    // ability to evict that same stalled client via shutdown().
    if stream.write_all(&frame.encode()).is_err() {
        let mut slot = client.lock().unwrap();
        if slot.as_ref().is_some_and(|c| c.epoch == epoch) {
            *slot = None;
        }
    }
}

/// One per attach. Reads `Data` (to the pty), `Resize` (to the pty and the
/// shared size, for the next attach's `Hello`) and `Kill` (signals the
/// child; the waiter thread does the actual exit and cleanup once
/// `child.wait()` returns). Ends on EOF, a protocol error, or `Kill`.
#[allow(clippy::too_many_arguments)]
/// The ring to `path`, whole or not at all: written beside it, synced, then
/// renamed over it.
fn write_snapshot(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("ring.tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_data()?;
    std::fs::rename(&tmp, path)
}

fn run_client_reader(
    mut stream: UnixStream,
    epoch: u64,
    client: Arc<Mutex<Option<ClientSlot>>>,
    pty_writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    size: Arc<Mutex<(u16, u16)>>,
    killer: Arc<Mutex<Box<dyn ChildKiller + Send + Sync>>>,
) {
    let mut reader = FrameReader::default();
    let mut buf = vec![0u8; 8192];
    'outer: loop {
        loop {
            match reader.next() {
                Ok(Some(Frame::Data(bytes))) => {
                    let mut w = pty_writer.lock().unwrap();
                    let _ = w.write_all(&bytes);
                    let _ = w.flush();
                }
                Ok(Some(Frame::Resize { cols, rows })) => {
                    let _ = master.lock().unwrap().resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                    *size.lock().unwrap() = (cols, rows);
                }
                Ok(Some(Frame::Kill)) => {
                    let _ = killer.lock().unwrap().kill();
                    break 'outer;
                }
                // Hello / Replay / ReplayEnd / Exited only ever travel
                // daemon-to-app; a client sending one is not ours to act on
                // and not worth dropping the connection over either.
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => break 'outer,
            }
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => reader.feed(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }

    // Detach cleanup: only clear the slot if it is still ours. A newer
    // attach's accept() may already have replaced it (that replacement is
    // what caused our shutdown() and brought us here in the first place).
    let mut slot = client.lock().unwrap();
    if slot.as_ref().is_some_and(|c| c.epoch == epoch) {
        *slot = None;
    }
}

/// The `.meta` file beside the socket: what `ift sessions` lists with no
/// app running, plus `attached`, which the app polls after being displaced.
struct Meta {
    path: PathBuf,
    pid: u32,
    cwd: String,
    cmd: String,
    started: String,
}

impl Meta {
    fn write(&self, attached: bool) -> std::io::Result<()> {
        let text = serde_json::json!({
            "pid": self.pid,
            "cwd": self.cwd,
            "cmd": self.cmd,
            "started": self.started,
            "attached": attached,
        })
        .to_string();
        // Whole or not at all: `ift sessions` reads this at any moment.
        let tmp = self.path.with_extension("meta.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &self.path)
    }
}

/// UTC, RFC 3339, second precision (`2026-09-17T10:00:00Z`). Written once
/// per daemon start into a file `ift sessions` (Task 6) reads with no app
/// running, so it deliberately does not reach for a calendar crate just for
/// one timestamp in every card's daemon. The day-to-date math is Howard
/// Hinnant's `civil_from_days`, the standard epoch-days-to-Gregorian-date
/// algorithm.
fn rfc3339_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86_400) as i64;
    let time = secs % 86_400;
    let (h, m, s) = (time / 3600, (time % 3600) / 60, time % 60);

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };

    format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}
