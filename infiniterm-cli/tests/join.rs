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

#[test]
fn a_host_with_no_app_running_is_refused_once_with_the_sentence() {
    let server = Scratch::new("j-srv2");
    let work = Scratch::new("j-cli2");
    let ssh = stand_in_ssh(&work.0, &server.0);
    let out = join(&ssh, &["test@server", "--check", "--ift", env!("CARGO_BIN_EXE_ift")]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no running infiniterm to join on this host"), "{err}");
    assert!(out.stdout.is_empty());
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
