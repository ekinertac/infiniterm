//! `ift connect --install` against a stand-in ssh (#159): the package reaches
//! the host's `tar` through ssh, lands where a non-root user's files go, and the
//! check then passes. The stand-in gives the host its own HOME and a PATH with
//! no `ift` or `iftd` on it, so before the install the host cannot serve.
//!
//! The release download is exercised with a `file://` folder through
//! `INFINITERM_RELEASE_BASE`, so the checksum path runs without a network. The
//! real GitHub address is first used by the release after the Linux workflow.

mod common;
use common::{profile_dir, Scratch};

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A host with an empty HOME and nothing of ours on its PATH.
fn bare_host(work: &Path) -> (PathBuf, PathBuf) {
    let home = work.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let ssh = work.join("ssh");
    std::fs::write(
        &ssh,
        format!(
            "#!/bin/sh\nfor last; do :; done\nHOME='{}' PATH=/usr/bin:/bin exec sh -c \"$last\"\n",
            home.display()
        ),
    )
    .unwrap();
    Command::new("chmod").arg("+x").arg(&ssh).status().unwrap();
    (ssh, home)
}

/// A package of the binaries this test run built, as the workflow makes it.
fn package(work: &Path, name: &str) -> PathBuf {
    let tar = work.join(name);
    let ok = Command::new("tar")
        .arg("czf")
        .arg(&tar)
        .arg("-C")
        .arg(profile_dir())
        .args(["ift", "iftd", "infiniterm-hook"])
        .status()
        .unwrap()
        .success();
    assert!(ok, "build the binaries first: cargo build -p infiniterm-cli -p infiniterm-session -p infiniterm-hook");
    tar
}

fn connect(ssh: &Path, args: &[&str], base: Option<&Path>) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ift"));
    c.arg("connect").args(args).env("INFINITERM_REMOTE_SSH", ssh);
    if let Some(b) = base {
        c.env("INFINITERM_RELEASE_BASE", format!("file://{}", b.display()));
    }
    c.output().unwrap()
}

fn installed(home: &Path) -> bool {
    ["ift", "iftd", "infiniterm-hook"]
        .iter()
        .all(|f| home.join(".local/bin").join(f).is_file())
}

#[test]
fn a_package_from_a_file_is_installed_and_the_host_then_passes_the_check() {
    let work = Scratch::new("i-file");
    let (ssh, home) = bare_host(&work.0);
    let pkg = package(&work.0, "pkg.tar.gz");
    let out = connect(
        &ssh,
        &["test@server", "--install", "--from", pkg.to_str().unwrap(), "--platform", "linux-x86_64", "--check"],
        None,
    );
    let (so, se) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{so}{se}");
    assert!(so.contains("installed") && so.contains("ready"), "{so}");
    assert!(installed(&home), "the files are where a non-root user's go");
    assert!(!se.contains("this app is"), "same version, no warning: {se}");
}

#[test]
fn a_host_without_ift_is_told_how_to_fix_it() {
    let work = Scratch::new("i-hint");
    let (ssh, home) = bare_host(&work.0);
    let out = connect(&ssh, &["test@server", "--check"], None);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("not installed") && err.contains("--install"), "{err}");
    assert!(!home.join(".local").exists(), "nothing was installed unasked");
}

#[test]
fn the_release_package_is_downloaded_checked_and_installed() {
    let work = Scratch::new("i-rel");
    let (ssh, home) = bare_host(&work.0);
    let base = work.0.join("release");
    std::fs::create_dir_all(&base).unwrap();
    let name = format!("infiniterm-server-v{VERSION}-linux-x86_64.tar.gz");
    let pkg = package(&base, &name);
    let sum = Command::new("shasum").args(["-a", "256"]).arg(&pkg).output().unwrap();
    let hex = String::from_utf8_lossy(&sum.stdout).split_whitespace().next().unwrap().to_string();
    std::fs::write(base.join(format!("{name}.sha256")), format!("{hex}  {name}\n")).unwrap();
    let out = connect(&ssh, &["test@server", "--install", "--platform", "linux-x86_64", "--check"], Some(&base));
    let (so, se) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{so}{se}");
    assert!(installed(&home));
}

#[test]
fn a_package_that_does_not_match_its_checksum_installs_nothing() {
    let work = Scratch::new("i-bad");
    let (ssh, home) = bare_host(&work.0);
    let base = work.0.join("release");
    std::fs::create_dir_all(&base).unwrap();
    let name = format!("infiniterm-server-v{VERSION}-linux-x86_64.tar.gz");
    package(&base, &name);
    std::fs::write(base.join(format!("{name}.sha256")), format!("{}  {name}\n", "0".repeat(64))).unwrap();
    let out = connect(&ssh, &["test@server", "--install", "--platform", "linux-x86_64"], Some(&base));
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("does not match its checksum") && err.contains("nothing was installed"), "{err}");
    assert!(!home.join(".local").exists(), "nothing was sent");
}

#[test]
fn a_missing_release_says_where_to_look_instead() {
    let work = Scratch::new("i-none");
    let (ssh, home) = bare_host(&work.0);
    let empty = work.0.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let out = connect(&ssh, &["test@server", "--install", "--platform", "linux-x86_64"], Some(&empty));
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("could not download") && err.contains("--from"), "{err}");
    assert!(!home.join(".local").exists());
}

#[test]
fn bad_use_of_the_install_flags_is_refused_early() {
    let work = Scratch::new("i-use");
    let (ssh, _home) = bare_host(&work.0);
    let out = connect(&ssh, &["test@server", "--from", "x.tar.gz"], None);
    assert_eq!(out.status.code(), Some(2), "--from without --install");
    let out = connect(&ssh, &["test@server", "--install", "--platform", "linux-mips"], None);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a platform"));
}
