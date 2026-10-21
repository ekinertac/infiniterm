//! Shared by the cli's integration tests that talk to a "server" through a
//! stand-in `ssh` (#118): the profile directory the binaries are built into, a
//! scratch dir that kills any `iftd` under it, and the stand-in itself.

// Each test file uses some of these and not others.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub fn profile_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().parent().unwrap().to_path_buf()
}

/// A scratch dir that kills any `iftd` under it on drop (see tests/proxy.rs).
pub struct Scratch(pub PathBuf);
impl Scratch {
    pub fn new(tag: &str) -> Scratch {
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
pub fn stand_in_ssh(dir: &Path, server_data: &Path) -> PathBuf {
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

