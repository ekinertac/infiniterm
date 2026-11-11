//! `ift phone add` on a Mac with Tailscale SSH: the phone cannot use port 22
//! there (Tailscale SSH owns it on the tailnet address and ignores
//! authorized_keys), so `phone add` forwards a tailnet port to the Mac's own
//! sshd with `tailscale serve`, and `phone remove` of the last phone closes it.
//!
//! A fake `tailscale` on PATH answers the questions and logs what it is told
//! to do, so no real tailnet is touched. The scratch HOME and config folder
//! keep the user's authorized_keys out of it. Needs the Mac's real sshd on
//! 127.0.0.1:22 and its host key, as `phone add` does; without them the test
//! says so and passes (a runner with no Remote Login cannot pair a phone).
//!
//! Unix only: `ift phone` answers "not available on Windows" there, and the
//! fake `tailscale` is a shell script with a mode bit.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ift-phone-ts-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn sshd_here() -> bool {
    std::net::TcpStream::connect_timeout(
        &"127.0.0.1:22".parse().unwrap(),
        std::time::Duration::from_secs(2),
    )
    .is_ok()
        && Path::new("/etc/ssh/ssh_host_ed25519_key.pub").exists()
}

/// A `tailscale` that prints `prefs` for `debug prefs` and `status` for
/// `serve status --json`, accepts `serve ...`, and appends its arguments to
/// `log`.
fn fake_tailscale(dir: &Path, prefs: &str, status: &str, log: &Path) {
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$*\" in\n  version) echo 1.0.0 ;;\n  \"debug prefs\") echo '{}' ;;\n  \"serve status --json\") echo '{}' ;;\n  *) exit 0 ;;\nesac\n",
        log.display(),
        prefs,
        status
    );
    let path = dir.join("tailscale");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

struct Env {
    home: PathBuf,
    bin: PathBuf,
    log: PathBuf,
}

fn env(tag: &str, prefs: &str, status: &str) -> Env {
    let root = scratch(tag);
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = root.join("tailscale.log");
    fake_tailscale(&bin, prefs, status, &log);
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    Env { home, bin, log }
}

fn ift(e: &Env, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ift"))
        .args(args)
        .env("HOME", &e.home)
        .env("INFINITERM_CONFIG_DIR", e.home.join("config"))
        // The fake first; the system tools `phone add` runs (ssh-keygen,
        // hostname, scutil) after it.
        .env("PATH", format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", e.bin.display()))
        .output()
        .unwrap()
}

fn said(e: &Env) -> String {
    std::fs::read_to_string(&e.log).unwrap_or_default()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn with_tailscale_ssh_on_the_phone_gets_a_forwarded_port_and_the_last_remove_closes_it() {
    if !sshd_here() {
        eprintln!("no sshd on 127.0.0.1:22 here: skipped");
        return;
    }
    let e = env("on", r#"{"RunSSH": true}"#, r#"{"TCP": {"443": {"HTTPS": true}}}"#);
    let add = ift(&e, &["phone", "add", "iphone"]);
    let out = text(&add);
    assert!(add.status.success(), "{out}");
    assert!(
        said(&e).contains("serve --bg --tcp=2222 tcp://localhost:22"),
        "{}",
        said(&e)
    );
    assert!(out.contains("port 2222"), "{out}");
    assert!(out.contains("password login"), "the risk is said: {out}");
    let marker = e.home.join("config").join("phone-forward");
    assert_eq!(std::fs::read_to_string(&marker).unwrap().trim(), "2222");

    // A second phone reuses the forward: the fake still reports no serve
    // config, so only the marker proves nothing new was asked for first.
    let remove = ift(&e, &["phone", "remove", "iphone"]);
    let out = text(&remove);
    assert!(remove.status.success(), "{out}");
    assert!(said(&e).contains("serve --tcp=2222 off"), "{}", said(&e));
    assert!(!marker.exists(), "the marker went with the forward");
    assert!(out.contains("closed the tailnet port 2222"), "{out}");
}

#[test]
fn removing_one_of_two_phones_leaves_the_forward() {
    if !sshd_here() {
        return;
    }
    let e = env("two", r#"{"RunSSH": true}"#, "{}");
    assert!(ift(&e, &["phone", "add", "one"]).status.success());
    assert!(ift(&e, &["phone", "add", "two"]).status.success());
    assert!(ift(&e, &["phone", "remove", "one"]).status.success());
    assert!(!said(&e).contains("off"), "one phone is left: {}", said(&e));
    assert!(ift(&e, &["phone", "remove", "two"]).status.success());
    assert!(said(&e).contains("serve --tcp=2222 off"));
}

#[test]
fn without_tailscale_ssh_no_forward_is_made() {
    if !sshd_here() {
        return;
    }
    let e = env("off", r#"{"RunSSH": false}"#, "{}");
    let add = ift(&e, &["phone", "add", "iphone"]);
    let out = text(&add);
    assert!(add.status.success(), "{out}");
    assert!(!said(&e).contains("serve --bg"), "{}", said(&e));
    assert!(!out.contains("Tailscale SSH"), "{out}");
    assert!(!e.home.join("config").join("phone-forward").exists());
}

#[test]
fn a_port_given_by_hand_skips_tailscale_altogether() {
    if !sshd_here() {
        return;
    }
    let e = env("hand", r#"{"RunSSH": true}"#, "{}");
    let add = ift(&e, &["phone", "add", "iphone", "--port", "2022"]);
    assert!(add.status.success(), "{}", text(&add));
    assert!(!said(&e).contains("serve"), "{}", said(&e));
}

#[test]
fn a_refused_forward_stops_before_any_key_is_written() {
    if !sshd_here() {
        return;
    }
    let e = env("refused", r#"{"RunSSH": true}"#, "{}");
    // The fake refuses `serve --bg`: the operator is not set, say.
    let path = e.bin.join("tailscale");
    let script = std::fs::read_to_string(&path).unwrap().replace(
        "  *) exit 0 ;;",
        "  serve*) echo 'access denied' >&2; exit 1 ;;\n  *) exit 0 ;;",
    );
    std::fs::write(&path, script).unwrap();
    let add = ift(&e, &["phone", "add", "iphone"]);
    let out = text(&add);
    assert!(!add.status.success(), "{out}");
    assert!(out.contains("tailscale set --operator"), "{out}");
    let keys = e.home.join(".ssh").join("authorized_keys");
    let written = std::fs::read_to_string(&keys).unwrap_or_default();
    assert!(!written.contains("infiniterm-phone:"), "no key was written: {written}");
}
