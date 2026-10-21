//! `ift serve` (#438): a canvas host with no window. It holds this Mac's saved
//! canvas, serves `j.sock` to viewers (`ift join`) and the hook/`ift` socket,
//! and keeps every card's shell running, with the app closed.
//!
//! The work is `infiniterm_core::headless::Host`; this file is the process
//! around it: the pid file, the signals, `--detach` and `--stop`.
//!
//!   ift serve            run in the foreground until SIGTERM or Ctrl+C
//!   ift serve --detach   start it in the background and return once it
//!                        answers on j.sock (what `ift join` does for a host
//!                        with no app, a later step)
//!   ift serve --stop     ask the running one to flush the canvas and exit
//!
//! A stop flushes the canvas file and leaves every session running: the next
//! host (this one again, or the app, which takes over from a running serve at
//! its launch) adopts them. Refuses to start when the app (or another serve)
//! already holds this data dir's socket.
//!
//! Called by main.rs. Related: infiniterm-core/src/headless.rs (the host and
//! the pid-file checks), join.rs (`ift join`, the viewer).

use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use infiniterm_core::headless::{live_serve, serve_pid_file, Host, HostIo};
use infiniterm_core::paths;

/// How long `--detach` waits for the host to answer, and `--stop` for it to go.
const WAIT: Duration = Duration::from_secs(15);

const USAGE: &str = "usage: ift serve [--detach | --stop]";

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

pub fn run(args: &[String]) -> ExitCode {
    match args {
        [] => foreground(),
        [a] if a == "--detach" => detach(),
        [a] if a == "--stop" => stop(),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Anything that already holds this data dir's socket: the app or a serve.
fn socket_in_use() -> bool {
    UnixStream::connect(paths::socket_path()).is_ok()
}

fn foreground() -> ExitCode {
    if let Some(pid) = live_serve(&serve_pid_file()) {
        eprintln!("ift: already serving (pid {pid})");
        return ExitCode::from(1);
    }
    if socket_in_use() {
        eprintln!("ift: infiniterm is already running here; there is nothing to serve");
        return ExitCode::from(1);
    }
    let mut host = match Host::start(HostIo::real()) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("ift: {e}");
            return ExitCode::from(1);
        }
    };
    if let Err(e) = std::fs::write(serve_pid_file(), std::process::id().to_string()) {
        eprintln!("ift: could not write {}: {e}", serve_pid_file().display());
        host.stop();
        return ExitCode::from(1);
    }
    // SAFETY: the handler only stores into an atomic, which is async-signal-safe.
    unsafe {
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGTERM, handler);
        libc::signal(libc::SIGINT, handler);
    }
    eprintln!(
        "[infiniterm] serving this canvas to viewers (pid {}); `ift serve --stop` ends it",
        std::process::id()
    );
    while !STOP.load(Ordering::SeqCst) {
        host.tick(now_ms());
        std::thread::sleep(Duration::from_millis(16));
    }
    host.stop();
    let _ = std::fs::remove_file(serve_pid_file());
    eprintln!("[infiniterm] stopped serving; the cards keep running");
    ExitCode::SUCCESS
}

/// Starts a serve of this same binary in a session of its own, its output in
/// `serve.log` beside the canvas file, and waits until it answers.
fn detach() -> ExitCode {
    if live_serve(&serve_pid_file()).is_some() && UnixStream::connect(paths::join_socket()).is_ok()
    {
        println!("already serving");
        return ExitCode::SUCCESS;
    }
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("ift: cannot find myself");
        return ExitCode::from(1);
    };
    let log = paths::socket_path().with_file_name("serve.log");
    let Ok(out) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
    else {
        eprintln!("ift: cannot write {}", log.display());
        return ExitCode::from(1);
    };
    let Ok(err) = out.try_clone() else {
        return ExitCode::from(1);
    };
    let mut child = Command::new(exe);
    child.arg("serve").stdin(Stdio::null()).stdout(out).stderr(err);
    // A session of its own: closing the ssh or terminal that started it must
    // not end it.
    // SAFETY: setsid is async-signal-safe and touches no memory of ours.
    unsafe {
        child.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = match child.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ift: could not start serve: {e}");
            return ExitCode::from(1);
        }
    };
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if UnixStream::connect(paths::join_socket()).is_ok() {
            println!("serving (pid {})", child.id());
            return ExitCode::SUCCESS;
        }
        if let Ok(Some(status)) = child.try_wait() {
            eprintln!("ift: serve ended at once ({status}); see {}", log.display());
            return ExitCode::from(1);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("ift: serve did not answer in time; see {}", log.display());
    ExitCode::from(1)
}

fn stop() -> ExitCode {
    let Some(pid) = live_serve(&serve_pid_file()) else {
        eprintln!("ift: no `ift serve` is running here");
        return ExitCode::from(1);
    };
    // SAFETY: SIGTERM to a pid just checked to be our own `ift serve`.
    unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if live_serve(&serve_pid_file()).is_none() {
            println!("stopped");
            return ExitCode::SUCCESS;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("ift: serve (pid {pid}) did not stop in time");
    ExitCode::from(1)
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.)
        .unwrap_or(0.)
}
