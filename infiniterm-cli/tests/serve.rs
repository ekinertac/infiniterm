#![cfg(unix)] // ift serve is unix-only
//! `ift serve` as a process (#438): starts detached, answers viewers and `ift`,
//! refuses a second one, stops cleanly, and never signals a process because a
//! stale pid file names it.
//!
//! Every path is under a scratch data dir; the real one and the installed
//! app's sockets are never touched. `iftd` is found beside `ift` (the same
//! target dir), as `tests/proxy.rs` does.

mod common;
use common::{profile_dir, Scratch};

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Output, Stdio};

fn ift(data: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ift"))
        .args(args)
        .env("INFINITERM_DATA_DIR", data)
        .env("HOME", data)
        .env(
            "PATH",
            format!(
                "{}:{}",
                profile_dir().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn ift_serve_runs_detached_answers_refuses_a_second_and_stops_cleanly() {
    let work = Scratch::new("serve");
    let data = work.0.clone();

    // Detached: returns once the host answers on j.sock.
    let out = ift(&data, &["serve", "--detach"]);
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("serving"));
    let pid_file = data.join("serve.pid");
    assert!(pid_file.is_file(), "the pid file is written");

    // A viewer's handshake is answered by the host with no app.
    let mut canvas = UnixStream::connect(data.join("j.sock")).unwrap();
    writeln!(canvas, "ift-join 2 canvas").unwrap();
    let mut answer = String::new();
    BufReader::new(canvas.try_clone().unwrap())
        .read_line(&mut answer)
        .unwrap();
    assert_eq!(answer.trim(), "ift-join 2 ok");

    // `ift` itself is answered by the host (the hook and verb socket).
    let ls = ift(&data, &["ls"]);
    assert!(ls.status.success(), "{}", String::from_utf8_lossy(&ls.stderr));

    // A second one, and a plain `ift serve`, are refused with a sentence.
    let again = ift(&data, &["serve"]);
    assert_eq!(again.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&again.stderr).contains("already"));

    // Stopped: the canvas file is saved, the files are gone, nothing answers.
    let stop = ift(&data, &["serve", "--stop"]);
    assert!(stop.status.success(), "{}", String::from_utf8_lossy(&stop.stderr));
    assert!(!pid_file.exists());
    assert!(UnixStream::connect(data.join("j.sock")).is_err());
    assert!(data.join("workspace.json").is_file(), "the canvas was saved");
}

#[test]
fn a_stale_pid_file_does_not_make_stop_signal_an_unrelated_process() {
    let work = Scratch::new("servestale");
    let data = work.0.clone();
    let mut bystander = Command::new("sleep").arg("30").spawn().unwrap();
    std::fs::write(data.join("serve.pid"), bystander.id().to_string()).unwrap();
    let out = ift(&data, &["serve", "--stop"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no `ift serve`"));
    assert!(
        bystander.try_wait().unwrap().is_none(),
        "the unrelated process is still running"
    );
    let _ = bystander.kill();
    let _ = bystander.wait();
}
