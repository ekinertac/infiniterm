//! `ift update-key` and `ift update-sign` as a process (#470): what `dist.ps1`
//! runs. The unit tests in `update_sign_cmd.rs` cover the logic; this holds
//! the command line, the exit codes and the stdout a script depends on.

use std::process::Command;

const MANIFEST: &str = r#"{"version":"0.7.0","build":1100,"tag":"v0.7.0-1100","url":"https://github.com/ekinertac/infiniterm/releases/download/v0.7.0-1100/x.zip","sha256":"00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff"}"#;

fn ift(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ift"))
        .args(args)
        .env_remove("INFINITERM_UPDATE_KEY")
        .output()
        .unwrap()
}

#[test]
fn a_key_is_made_the_manifest_signed_and_the_signature_verifies_with_the_printed_key() {
    let dir = std::env::temp_dir().join(format!("ift-update-sign-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let key = dir.join("update.key");
    let manifest = dir.join("latest-windows.json");
    std::fs::write(&manifest, MANIFEST).unwrap();

    // stdout is the public key and nothing else, so a script can capture it.
    let made = ift(&["update-key", "new", key.to_str().unwrap()]);
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    let public = String::from_utf8(made.stdout).unwrap().trim().to_string();
    assert_eq!(public.len(), 64, "{public}");
    assert!(public.chars().all(|c| c.is_ascii_hexdigit()));

    // A second `new` on the same file fails and keeps the key.
    let again = ift(&["update-key", "new", key.to_str().unwrap()]);
    assert_eq!(again.status.code(), Some(1));
    let printed = ift(&["update-key", "public", key.to_str().unwrap()]);
    assert_eq!(String::from_utf8(printed.stdout).unwrap().trim(), public);

    let signed = ift(&["update-sign", manifest.to_str().unwrap(), "--key", key.to_str().unwrap()]);
    assert!(signed.status.success(), "{}", String::from_utf8_lossy(&signed.stderr));
    let text = std::fs::read_to_string(&manifest).unwrap();
    let parsed = infiniterm_core::update::parse_manifest(&text).unwrap();
    assert!(infiniterm_core::update::check_manifest_signature(&public, &parsed).is_ok());

    // The key can come from the environment instead of the command line.
    let by_env = Command::new(env!("CARGO_BIN_EXE_ift"))
        .args(["update-sign", manifest.to_str().unwrap()])
        .env("INFINITERM_UPDATE_KEY", &key)
        .output()
        .unwrap();
    assert!(by_env.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_use_is_exit_2_and_a_failure_is_exit_1() {
    assert_eq!(ift(&["update-key"]).status.code(), Some(2));
    assert_eq!(ift(&["update-key", "nonsense", "x"]).status.code(), Some(2));
    assert_eq!(ift(&["update-sign"]).status.code(), Some(2));
    assert_eq!(ift(&["update-sign", "m.json"]).status.code(), Some(2), "no key given");
    let out = ift(&["update-sign", "/definitely/not/here.json", "--key", "/not/a/key"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read"));
}
