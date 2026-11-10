//! `ift phone-gate` as sshd runs it: the forced command of a paired phone's
//! key. `$SSH_ORIGINAL_COMMAND` is what the phone asked for; the gate runs
//! this same binary for the exact commands it knows and refuses the rest with
//! exit 2 and a reason on stderr. The word table is tested in
//! `infiniterm_core::phone`; this holds the real process to it, including the
//! check that the first word is the ift the key was paired with.
//!
//! Unix only: `ift phone-gate` answers "not available on Windows" there, and
//! sshd's forced command is not a Windows thing.
#![cfg(unix)]

use std::process::Command;

fn gate(original: Option<&str>) -> (Option<i32>, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ift"));
    cmd.arg("phone-gate").env_remove("SSH_ORIGINAL_COMMAND");
    if let Some(o) = original {
        cmd.env("SSH_ORIGINAL_COMMAND", o);
    }
    let out = cmd.output().unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The path the phone would have been paired with: this binary, resolved.
fn paired() -> String {
    std::fs::canonicalize(env!("CARGO_BIN_EXE_ift"))
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn the_paired_ift_may_run_an_allowed_verb() {
    let (code, out, _) = gate(Some(&format!("{} --version", paired())));
    assert_eq!(code, Some(0));
    assert!(out.starts_with("ift "), "{out}");
}

#[test]
fn a_quoted_path_is_the_same_path() {
    let (code, out, _) = gate(Some(&format!("'{}' --version", paired())));
    assert_eq!(code, Some(0), "{out}");
}

#[test]
fn a_key_with_no_command_opens_no_shell() {
    let (code, _, err) = gate(None);
    assert_eq!(code, Some(2));
    assert!(err.contains("opens no shell"), "{err}");
}

#[test]
fn every_other_command_is_refused_before_anything_runs() {
    let me = paired();
    for bad in [
        "/bin/sh -c id".to_string(),
        "ls".to_string(),
        format!("{me} close 7"),
        format!("{me} send 7 hello"),
        format!("{me} --version; id"),
        format!("{me} --version && id"),
        format!("{me} read 7$(id)"),
        format!("{me} run app.quit"),
        format!("{me} phone add x"),
        format!("{me}x --version"),
        "".to_string(),
    ] {
        let (code, out, err) = gate(Some(&bad));
        assert_eq!(code, Some(2), "{bad:?}: {out} {err}");
        assert!(out.is_empty(), "{bad:?} printed {out}");
    }
}
