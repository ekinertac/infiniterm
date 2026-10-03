//! The remote backend end to end (#118): `DaemonBackend::spawn_remote` and
//! `adopt_remote` over a stand-in `ssh` that runs the real `ift proxy` and
//! `iftd` on this machine, with the "server" data dir apart from the client's.
//!
//! The stand-in takes the same arguments ssh would and runs the last one, the
//! remote command, in a shell, as the server's login shell would. So this
//! covers the quoting, the relay, a real daemon and the reattach with replay;
//! it does not cover ssh itself or a network.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use infiniterm_core::backend::daemon::DaemonBackend;
use infiniterm_core::backend::remote::RemoteHost;
use infiniterm_core::backend::{PaneEvent, PaneId};

const DEADLINE: Duration = Duration::from_secs(15);

fn profile_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().parent().unwrap().to_path_buf()
}

/// A scratch dir that kills any `iftd` under it on drop (see tests/proxy.rs).
struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!("ift-rm-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let needle = format!("--socket {}", self.0.display());
        if let Ok(out) = std::process::Command::new("ps")
            .args(["-axww", "-o", "pid=,command="])
            .output()
        {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if line.contains(&needle) {
                    if let Some(pid) = line.split_whitespace().next() {
                        let _ = std::process::Command::new("kill")
                            .args(["-9", pid])
                            .status();
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An `ssh` that runs its last argument locally against the "server" data dir.
fn stand_in_ssh(dir: &Path, server_data: &Path) -> PathBuf {
    let script = dir.join("ssh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nfor last; do :; done\nINFINITERM_DATA_DIR='{}' PATH='{}':\"$PATH\" exec sh -c \"$last\"\n",
            server_data.display(),
            profile_dir().display()
        ),
    )
    .unwrap();
    std::process::Command::new("chmod")
        .args(["+x"])
        .arg(&script)
        .status()
        .unwrap();
    script
}

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
