#![cfg(unix)] // join is unix-only; see docs/windows-handoff.md
//! `ift join --check` against a stand-in ssh (#423): what the launcher decides
//! before it opens a viewer window. The stand-in runs the remote command here,
//! as the host's login shell would, against a "host" data directory.
//!
//! Opening the window itself (`open -n`) is not tested here; the app in viewer
//! mode was run on screen against a host instance (see the PR).

mod common;
use common::{profile_dir, stand_in_ssh, Scratch};

use infiniterm_core::backend::join::{JoinHub, WriteSession};
use infiniterm_core::backend::relay::Relay;
use std::process::Command;
use std::sync::{Arc, Mutex};

fn join(ssh: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ift"))
        .arg("join")
        .args(args)
        .env("INFINITERM_JOIN_SSH", ssh)
        .output()
        .unwrap()
}

#[test]
fn a_host_with_a_running_app_passes_the_join_check() {
    let server = Scratch::new("j-srv");
    let work = Scratch::new("j-cli");
    let write: WriteSession = Arc::new(|_, _| {});
    let hub = JoinHub::new(Arc::new(Mutex::new(Relay::new())), write);
    hub.serve(&server.0.join("j.sock")).unwrap();
    let ssh = stand_in_ssh(&work.0, &server.0);
    let out = join(&ssh, &["test@server", "--check", "--ift", env!("CARGO_BIN_EXE_ift")]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("can be joined"));
}

/// Stops an `ift serve` a test started on a host's data dir, even when the
/// test fails first.
struct ServeGuard(std::path::PathBuf);
impl Drop for ServeGuard {
    fn drop(&mut self) {
        let _ = Command::new(env!("CARGO_BIN_EXE_ift"))
            .args(["serve", "--stop"])
            .env("INFINITERM_DATA_DIR", &self.0)
            .output();
    }
}

#[test]
fn a_host_with_no_app_running_is_ready_through_ift_serve_and_check_starts_nothing() {
    let server = Scratch::new("j-srv2");
    let work = Scratch::new("j-cli2");
    let ssh = stand_in_ssh(&work.0, &server.0);
    let _guard = ServeGuard(server.0.clone());
    let out = join(&ssh, &["test@server", "--check", "--ift", env!("CARGO_BIN_EXE_ift")]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("ift join would start `ift serve`"), "{said}");
    assert!(
        !server.0.join("j.sock").exists() && !server.0.join("serve.pid").exists(),
        "--check starts nothing"
    );
}

#[test]
fn ift_join_starts_a_canvas_host_when_no_app_answers_and_then_joins_it() {
    let server = Scratch::new("j-srv5");
    let work = Scratch::new("j-cli5");
    let ssh = stand_in_ssh(&work.0, &server.0);
    let _guard = ServeGuard(server.0.clone());
    let out = Command::new(env!("CARGO_BIN_EXE_ift"))
        .args(["join", "test@server", "--ift", env!("CARGO_BIN_EXE_ift")])
        .env("INFINITERM_JOIN_SSH", &ssh)
        .env("INFINITERM_JOIN_NO_WINDOW", "1")
        .env("INFINITERM_DATA_DIR", work.0.join("viewer"))
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let (so, se) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(out.status.success(), "{so}{se}");
    assert!(se.contains("starting a canvas host there"), "{se}");
    assert!(so.contains("ready (window not opened)"), "{so}");
    assert!(server.0.join("serve.pid").is_file(), "a host is serving there");
    // Joining again finds it running and starts nothing more.
    let again = Command::new(env!("CARGO_BIN_EXE_ift"))
        .args(["join", "test@server", "--check", "--ift", env!("CARGO_BIN_EXE_ift")])
        .env("INFINITERM_JOIN_SSH", &ssh)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&again.stdout).contains("infiniterm is running there"));
}

#[test]
fn a_host_whose_ift_has_no_serve_says_how_to_fix_it() {
    let server = Scratch::new("j-srv6");
    let work = Scratch::new("j-cli6");
    // Everything works except `serve`, which an older ift answers as it
    // answers any word it does not know: not at all.
    let ssh = work.0.join("ssh");
    std::fs::write(
        &ssh,
        format!(
            "#!/bin/sh\nfor last; do :; done\ncase \"$last\" in\n*\"serve --detach\"*) echo 'ift: infiniterm is not running' >&2; exit 1;;\nesac\nINFINITERM_DATA_DIR='{}' PATH='{}':\"$PATH\" exec sh -c \"$last\"\n",
            server.0.display(),
            profile_dir().display()
        ),
    )
    .unwrap();
    Command::new("chmod").arg("+x").arg(&ssh).status().unwrap();
    let out = join(&ssh, &["test@server", "--ift", env!("CARGO_BIN_EXE_ift")]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("could not start a canvas host") && err.contains("update it there"), "{err}");
}

#[test]
fn a_host_whose_ift_cannot_join_is_told_to_update_it() {
    let server = Scratch::new("j-srv3");
    let work = Scratch::new("j-cli3");
    // Everything works as on a real host except `proxy join`, which an older
    // ift answers with a shell error where the handshake answer should be.
    let ssh = work.0.join("ssh");
    std::fs::write(
        &ssh,
        format!(
            "#!/bin/sh\nfor last; do :; done\ncase \"$last\" in\n*\"proxy join\"*) echo 'ift: unknown command proxy join'; exit 2;;\nesac\nINFINITERM_DATA_DIR='{}' PATH='{}':\"$PATH\" exec sh -c \"$last\"\n",
            server.0.display(),
            profile_dir().display()
        ),
    )
    .unwrap();
    Command::new("chmod").arg("+x").arg(&ssh).status().unwrap();
    let out = join(&ssh, &["test@server", "--check", "--ift", env!("CARGO_BIN_EXE_ift")]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("cannot be joined") && err.contains("update infiniterm there"), "{err}");
    assert!(out.stdout.is_empty());
}

/// `ift join --check` against an ssh that fails with `said` on stderr and
/// exits 255, as OpenSSH does when it cannot connect or log in.
fn check_against_failing_ssh(name: &str, said: &str) -> std::process::Output {
    let work = Scratch::new(name);
    let ssh = work.0.join("ssh");
    std::fs::write(&ssh, format!("#!/bin/sh\necho '{said}' >&2\nexit 255\n")).unwrap();
    Command::new("chmod").arg("+x").arg(&ssh).status().unwrap();
    join(&ssh, &["test@server", "--check", "--ift", env!("CARGO_BIN_EXE_ift")])
}

#[test]
fn an_unreachable_host_is_said_with_ssh_words_and_exit_one() {
    let out = check_against_failing_ssh(
        "j-down",
        "ssh: connect to host server port 22: Operation timed out",
    );
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("server") && err.contains("timed out"), "{err}");
    assert!(out.stdout.is_empty());
}

#[test]
fn a_refused_login_is_said_once_and_exit_one() {
    let out = check_against_failing_ssh(
        "j-login",
        "test@server: Permission denied (publickey).",
    );
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Permission denied"), "{err}");
    assert!(out.stdout.is_empty());
}

#[test]
fn join_installs_nothing_and_says_where_that_lives() {
    let out = Command::new(env!("CARGO_BIN_EXE_ift"))
        .args(["join", "test@server", "--install"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("ift connect test@server --install"), "{err}");
}
