//! `ift install-claude-hooks` against a settings.json that is a symlink into a
//! dotfiles repo (#219): the file the link points at gets the hooks and the
//! link stays a link. Before, the install renamed a temp file over the link
//! and left the repo's copy untouched.

use std::os::unix::fs::symlink;
use std::process::Command;

#[test]
fn installing_hooks_writes_through_a_symlinked_settings_file() {
    let root = std::env::temp_dir().join(format!("ift-symlink-{}", std::process::id()));
    let (home, dots) = (root.join("home"), root.join("dotfiles"));
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::create_dir_all(&dots).unwrap();
    let real = dots.join("claude-settings.json");
    std::fs::write(&real, "{\"model\": \"opus\"}\n").unwrap();
    let link = home.join(".claude").join("settings.json");
    symlink(&real, &link).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ift"))
        .arg("install-claude-hooks")
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
    let text = std::fs::read_to_string(&real).unwrap();
    assert!(text.contains("\"model\""), "what was there is kept: {text}");
    assert!(text.contains("hooks"), "the hooks went into the repo's file: {text}");
    std::fs::remove_dir_all(&root).ok();
}
