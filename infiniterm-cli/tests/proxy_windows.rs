//! `ift proxy join` on Windows, the real binary against a pipe server in this
//! test: the host's join pipe over the child's stdin and stdout, as a Mac's
//! `ssh host ift proxy join ...` uses it. The server plays the host app
//! (`backend/join.rs`, not built on Windows yet): it reads the handshake line
//! and answers.
//!
//! What it pins down: the handshake line, every byte value both ways (binary
//! clean through the child's stdio), the child ending when the host closes,
//! and the one-line failures (no app running, bad use, a verb that needs
//! `iftd`). A scratch data dir moves the pipe's name, so nothing here can reach
//! a real app.
#![cfg(windows)]

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use infiniterm_core::paths::join_socket_for;
use infiniterm_core::pipe::PipeListener;

fn payload(len: usize) -> Vec<u8> {
    (0..=255u8).cycle().take(len).collect()
}

fn scratch(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("ift-proxywin-{tag}-{}", std::process::id()))
}

fn ift(dir: &std::path::Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ift"));
    c.args(args)
        .env("INFINITERM_DATA_DIR", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

#[test]
fn the_real_proxy_carries_a_session_both_ways_byte_for_byte_and_ends_with_the_host() {
    let dir = scratch("carry");
    let pipe = join_socket_for(Some(dir.as_os_str()));
    let mut listener = PipeListener::bind(&pipe).unwrap();
    let from_host = payload(250_000);
    let from_viewer = payload(90_000);
    let (seen_tx, seen_rx) = mpsc::channel();
    let (sent, expect_len) = (from_host.clone(), from_viewer.len());
    let server = std::thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let mut out = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        writeln!(out, "ift-join 2 ok").unwrap();
        out.write_all(&sent).unwrap();
        let mut got = vec![0u8; expect_len];
        reader.read_exact(&mut got).unwrap();
        seen_tx.send((line, got)).unwrap();
        // Returning drops the connection: the child must end by itself.
    });

    let mut child = ift(&dir, &["proxy", "join", "session", "abc-123"])
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    // stdout is drained on its own thread, as the viewer's ssh does.
    let reader = std::thread::spawn(move || {
        let mut all = Vec::new();
        stdout.read_to_end(&mut all).unwrap();
        all
    });
    stdin.write_all(&from_viewer).unwrap();
    stdin.flush().unwrap();
    let (line, got) = seen_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(line, "ift-join 2 session abc-123\n");
    assert!(got == from_viewer, "the viewer's bytes reached the host intact");
    server.join().unwrap();
    // stdin is still open (as ssh holds it); the child ends because the host did.
    let status = child.wait().unwrap();
    assert!(status.success(), "{status:?}");
    drop(stdin);
    let out = reader.join().unwrap();
    let mut want = b"ift-join 2 ok\n".to_vec();
    want.extend_from_slice(&from_host);
    assert!(out == want, "the answer line and the host's bytes, intact");
}

#[test]
fn with_no_app_running_it_says_so_on_stderr_and_stdout_stays_empty() {
    let dir = scratch("none");
    let out = ift(&dir, &["proxy", "join", "canvas"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "stdout is the protocol, nothing else");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no running infiniterm"), "{err}");
}

#[test]
fn bad_use_and_a_verb_that_needs_iftd_are_refused_with_a_reason() {
    let dir = scratch("bad");
    for (args, needle) in [
        (&["proxy", "join", "session", "../x"][..], "not a session id"),
        (&["proxy", "join"][..], "usage: ift proxy join"),
        (&["proxy", "session", "abc"][..], "not available on Windows"),
        (&["proxy", "files"][..], "not available on Windows"),
    ] {
        let out = ift(&dir, args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(needle), "{args:?}: {err}");
    }
}
