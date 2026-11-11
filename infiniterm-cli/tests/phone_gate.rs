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
fn the_file_service_runs_for_a_phone_only_read_only() {
    // No stdin, so the service ends at once with a clean exit.
    let (code, _, err) = gate(Some(&format!("{} proxy files --read-only", paired())));
    assert_eq!(code, Some(0), "{err}");
}

#[test]
fn the_read_only_file_service_refuses_a_write_and_creates_no_file() {
    use infiniterm_core::file_proto::{Frame, FrameReader, Kind};
    use std::io::{Read, Write};
    use std::process::Stdio;

    let dir = std::env::temp_dir().join(format!("ift-gate-ro-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("scratch.txt");
    let _ = std::fs::remove_file(&target);

    let mut child = Command::new(env!("CARGO_BIN_EXE_ift"))
        .args(["proxy", "files", "--read-only"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = b"ift-files 1\n".to_vec();
    input.extend(Frame::new(Kind::Data, 7, serde_json::json!({}), b"bytes".to_vec()).encode());
    let request = serde_json::json!({ "op": "write", "path": target.to_string_lossy() });
    input.extend(Frame::new(Kind::Request, 7, request, vec![]).encode());
    // Closing stdin ends the service after it has answered.
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let mut out = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut out).unwrap();
    child.wait().unwrap();

    let hello = b"ift-files 1 ok\n";
    assert!(out.starts_with(hello), "{}", String::from_utf8_lossy(&out));
    let mut reader = FrameReader::default();
    reader.feed(&out[hello.len()..]);
    let frame = reader.next().unwrap().unwrap();
    assert_eq!((frame.kind, frame.id), (Kind::Error, 7));
    assert_eq!(frame.json["code"], "denied");
    assert!(!target.exists(), "a refused write leaves no file");
    let _ = std::fs::remove_dir_all(&dir);
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
        format!("{me} proxy files"),
        format!("{me} proxy files --read-only extra"),
        format!("{me}x --version"),
        "".to_string(),
    ] {
        let (code, out, err) = gate(Some(&bad));
        assert_eq!(code, Some(2), "{bad:?}: {out} {err}");
        assert!(out.is_empty(), "{bad:?} printed {out}");
    }
}
