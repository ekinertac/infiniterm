//! `ift connect --check` against a stand-in ssh (#118): what the launcher
//! decides before it opens a window. The stand-in runs the remote command
//! here, as the server's login shell would.
//!
//! Opening the window itself (`open -n`) is not tested here; `tools/drive/
//! remote.sh` runs the app in remote mode.

mod common;
use common::{stand_in_ssh, Scratch};

use std::process::Command;

fn connect(ssh: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ift"))
        .arg("connect")
        .args(args)
        .env("INFINITERM_REMOTE_SSH", ssh)
        .output()
        .unwrap()
}

#[test]
fn a_ready_host_passes_the_check() {
    let server = Scratch::new("c-srv");
    let work = Scratch::new("c-cli");
    let ssh = stand_in_ssh(&work.0, &server.0);
    let out = connect(&ssh, &["test@server", "--check", "--ift", env!("CARGO_BIN_EXE_ift")]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("ready"));
}

#[test]
fn a_host_without_ift_says_so() {
    let server = Scratch::new("c-srv2");
    let work = Scratch::new("c-cli2");
    let ssh = stand_in_ssh(&work.0, &server.0);
    let out = connect(&ssh, &["test@server", "--check", "--ift", "/nonexistent/ift"]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("ift is not installed on the server"), "{err}");
    assert!(out.stdout.is_empty());
}

#[test]
fn a_host_with_ift_but_no_iftd_says_so() {
    let work = Scratch::new("c-cli3");
    // An `ift` alone in a folder, and a PATH that has no iftd on it.
    let bin = work.0.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ift"), bin.join("ift")).unwrap();
    let ssh = work.0.join("ssh");
    std::fs::write(
        &ssh,
        "#!/bin/sh\nfor last; do :; done\nPATH=/usr/bin:/bin exec sh -c \"$last\"\n",
    )
    .unwrap();
    Command::new("chmod").arg("+x").arg(&ssh).status().unwrap();
    let out = connect(
        &ssh,
        &["test@server", "--check", "--ift", bin.join("ift").to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("iftd is not installed on the server"), "{err}");
}

#[test]
fn a_login_that_fails_says_why() {
    let work = Scratch::new("c-cli4");
    let ssh = work.0.join("ssh");
    std::fs::write(&ssh, "#!/bin/sh\necho 'Permission denied (publickey).' >&2\nexit 255\n").unwrap();
    Command::new("chmod").arg("+x").arg(&ssh).status().unwrap();
    let out = connect(&ssh, &["test@server", "--check"]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("test@server") && err.contains("Permission denied"), "{err}");
}

#[test]
fn a_host_that_looks_like_an_ssh_option_never_reaches_ssh() {
    let work = Scratch::new("c-cli5");
    let ssh = work.0.join("ssh");
    // If this ran it would leave a file behind.
    let marker = work.0.join("ran");
    std::fs::write(&ssh, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    Command::new("chmod").arg("+x").arg(&ssh).status().unwrap();
    let out = connect(&ssh, &["-oProxyCommand=touch /tmp/ift-pwned"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!marker.exists(), "ssh must not run for a bad host");
}
