//! The remote backend end to end (#118): `DaemonBackend::spawn_remote` and
//! `adopt_remote` over a stand-in `ssh` that runs the real `ift proxy` and
//! `iftd` on this machine, with the "server" data dir apart from the client's.
//!
//! The stand-in takes the same arguments ssh would and runs the last one, the
//! remote command, in a shell, as the server's login shell would. So this
//! covers the quoting, the relay, a real daemon and the reattach with replay;
//! it does not cover ssh itself or a network.

use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use infiniterm_core::backend::daemon::DaemonBackend;
use infiniterm_core::backend::remote::RemoteHost;
use infiniterm_core::backend::{PaneEvent, PaneId};

mod common;
use common::{profile_dir, stand_in_ssh, Scratch};

const DEADLINE: Duration = Duration::from_secs(15);

fn host(ssh: &Path) -> RemoteHost {
    let mut h = RemoteHost::new("test@server");
    h.ssh = ssh.to_string_lossy().into();
    h.ift = env!("CARGO_BIN_EXE_ift").into();
    h
}

/// Collects Output and Replay text from `pane` until `needle` appears.
fn wait_for(rx: &Receiver<(PaneId, PaneEvent)>, pane: PaneId, needle: &str) -> String {
    let start = Instant::now();
    let mut seen = String::new();
    while start.elapsed() < DEADLINE {
        if let Ok((id, ev)) = rx.recv_timeout(Duration::from_millis(200)) {
            if id != pane {
                continue;
            }
            match ev {
                PaneEvent::Output(b) | PaneEvent::Replay(b) => {
                    seen.push_str(&String::from_utf8_lossy(&b));
                    if seen.contains(needle) {
                        return seen;
                    }
                }
                PaneEvent::Exited { .. } => break,
                _ => {}
            }
        }
    }
    panic!("never saw {needle:?}; saw {seen:?}");
}

#[test]
fn a_remote_shell_is_spawned_detached_and_taken_back_with_its_history() {
    assert!(profile_dir().join("iftd").is_file(), "cargo build -p infiniterm-session first");
    let server = Scratch::new("srv");
    let client = Scratch::new("cli");
    let ssh = stand_in_ssh(&client.0, &server.0);
    let h = host(&ssh);
    let (backend, rx) = DaemonBackend::new(client.0.join("s"), 4);

    // A directory with a space and a quote must arrive as text.
    let cwd = server.0.join("my 'dir'");
    std::fs::create_dir_all(&cwd).unwrap();
    let pane = backend
        .spawn_remote(
            &h,
            cwd.to_str().unwrap(),
            Some("pwd; printf READY; exec sleep 60"),
            vec![("INFINITERM_T".into(), "a b".into())],
        )
        .unwrap();
    let out = wait_for(&rx, pane, "READY");
    assert!(out.contains("my 'dir'"), "the shell started in the odd directory: {out:?}");
    let id = backend.session_id(pane).unwrap();

    // A closed connection is a detach: the shell keeps running, and a new
    // connection by id gets what it missed.
    backend.detach_now(pane);
    let again = backend.adopt_remote(&h, &id).expect("the session is still there");
    assert_ne!(again, pane);
    wait_for(&rx, again, "READY");

    // Gone is gone: a session that does not exist is None, not a hang.
    backend.kill_now(again);
    let start = Instant::now();
    let mut gone = None;
    while start.elapsed() < DEADLINE {
        gone = backend.adopt_remote(&h, &id);
        if gone.is_none() {
            break;
        }
        backend.kill_now(gone.unwrap());
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(gone.is_none(), "the killed session is gone");
}

#[test]
fn ssh_failing_says_why_and_does_not_hang() {
    let scratch = Scratch::new("bad");
    let script = scratch.0.join("ssh");
    std::fs::write(
        &script,
        "#!/bin/sh\necho 'Permission denied (publickey).' >&2\nexit 255\n",
    )
    .unwrap();
    std::process::Command::new("chmod").arg("+x").arg(&script).status().unwrap();
    let h = host(&script);
    let (backend, _rx) = DaemonBackend::new(scratch.0.join("s"), 4);
    let start = Instant::now();
    let err = backend
        .spawn_remote(&h, "/tmp", None, vec![])
        .expect_err("a refused login is an error");
    assert!(start.elapsed() < Duration::from_secs(10));
    let msg = err.to_string();
    assert!(msg.contains("test@server") && msg.contains("Permission denied"), "{msg}");
}

// #118: a remote instance's backend is told the host once; then the plain
// spawn, adopt and session list all go over ssh.
#[test]
fn a_backend_made_for_a_host_spawns_lists_and_adopts_over_ssh() {
    assert!(profile_dir().join("iftd").is_file(), "cargo build -p infiniterm-session first");
    let server = Scratch::new("srv2");
    let client = Scratch::new("cli2");
    let ssh = stand_in_ssh(&client.0, &server.0);
    let (backend, rx) = DaemonBackend::new_remote(host(&ssh), 4);

    assert!(backend.live_sessions_now().is_empty(), "the server has no sessions yet");
    let pane = backend
        .spawn_now(
            std::path::Path::new("/tmp"),
            Some("printf READY2; exec sleep 60"),
            vec![],
        )
        .unwrap();
    wait_for(&rx, pane, "READY2");
    let id = backend.session_id(pane).unwrap();
    assert_eq!(backend.live_sessions_now(), vec![id.clone()], "the server lists it");

    backend.detach_now(pane);
    let again = backend.adopt(&id).expect("adopted over ssh");
    wait_for(&rx, again, "READY2");
    backend.kill_now(again);
}
