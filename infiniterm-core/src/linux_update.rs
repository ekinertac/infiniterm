//! The Linux self-update for an install.sh install (#840): stage a newer
//! release beside the running one, then move `current` to it at the restart.
//!
//! install.sh lays an install out as `~/.local/share/infiniterm/<version>/`
//! (the flat tarball: `infiniterm`, `ift`, `iftd`, `infiniterm-hook`,
//! `VERSION`, ...) with `current` a link to the folder in use, and `~/.local/bin`
//! and the desktop entry naming paths through `current`. This does what a
//! second run of install.sh does, in two halves so the phone can update a
//! Linux host the way it updates a Mac (`ift update status` says `ready`,
//! `run app.update.install` restarts into it):
//! - `stage`: download the tarball, check its sha256 (as install.sh does)
//!   and the manifest's signature once a key is set, unpack it into
//!   `<root>/<version>/`. `current` is not touched, so a failed download, a
//!   bad hash or a broken tarball leaves the running install exactly as it
//!   was, and only `<root>/.update` (scratch, removed again) is written
//!   before the checks pass.
//! - `repoint_current` and `prune`, at the restart: `current` moves in one
//!   rename (a new link beside it, renamed over it), then only the new and
//!   the previous version folders are kept, as install.sh keeps two. The
//!   running app keeps its old binary mapped until it exits, and the `iftd`
//!   daemons it started keep running from their deleted files; the new app
//!   adopts them under the protocol rule (CLAUDE.md), as on the Mac.
//!
//! A Homebrew install never comes here: brew owns its files, so it keeps the
//! notice and `brew upgrade` (`update::linux_update_hint`).
//!
//! Called by `infiniterm-ui/src/updater.rs` (the polling thread stages) and
//! `runtime.rs` (the install at a restart). The download is a closure so the
//! tests stage from a local file; unpacking uses `tar`, which every Linux and
//! macOS has, so the tests run on the Mac too. Related: `update.rs`
//! (`parse_linux_manifest`, `check_linux_signature`), `site/public/install.sh`.

use crate::update::LinuxRelease;
use std::path::{Path, PathBuf};

/// The scratch folder a stage downloads and unpacks in. Hidden, so neither
/// `prune` nor install.sh's `ls` takes it for a version.
const SCRATCH: &str = ".update";

/// The link to the folder in use, and the new link made beside it first.
const CURRENT: &str = "current";
// Only `repoint_current` makes the new link, and that is unix-only (a symlink);
// without the cfg a Windows build warned "never used".
#[cfg(unix)]
const CURRENT_NEXT: &str = ".current.next";

/// The folder the tarball unpacks to.
const TARBALL_DIR: &str = "infiniterm";

/// A file's SHA-256, lowercase hex.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let digest = ring::digest::digest(&ring::digest::SHA256, &bytes);
    Ok(digest.as_ref().iter().map(|b| format!("{b:02x}")).collect())
}

/// A version install.sh would accept as a folder name: letters, digits and
/// `.+-`, never empty, so it can neither leave `root` nor hide.
fn valid_version(v: &str) -> bool {
    !v.is_empty()
        && !v.starts_with('.')
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".+-".contains(c))
}

/// Download `r` with `fetch(url, dest)`, check it, and unpack it into
/// `<root>/<version>/`. Returns that folder. On any failure nothing outside
/// `<root>/.update` was written, and that is removed again; `current` is
/// never touched here.
pub fn stage(
    r: &LinuxRelease,
    root: &Path,
    public_key_hex: &str,
    fetch: impl FnOnce(&str, &Path) -> Result<(), String>,
) -> Result<PathBuf, String> {
    // Before a byte is downloaded, as on Windows.
    crate::update::check_linux_signature(public_key_hex, r)?;
    if !valid_version(&r.version) {
        return Err(format!("not a version: {}", r.version));
    }
    let scratch = root.join(SCRATCH);
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let result = stage_in(r, root, &scratch, fetch);
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

fn stage_in(
    r: &LinuxRelease,
    root: &Path,
    scratch: &Path,
    fetch: impl FnOnce(&str, &Path) -> Result<(), String>,
) -> Result<PathBuf, String> {
    let tarball = scratch.join("infiniterm.tar.gz");
    fetch(&r.url, &tarball)?;
    if sha256_file(&tarball)? != r.sha256.to_ascii_lowercase() {
        return Err(format!("{} does not match its checksum", r.tag));
    }
    let unpacked = scratch.join("x");
    std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
    let out = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(&unpacked)
        .output()
        .map_err(|e| format!("tar: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "could not unpack {}: {}",
            r.tag,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let build = unpacked.join(TARBALL_DIR);
    let version = std::fs::read_to_string(build.join("VERSION")).unwrap_or_default();
    if !build.join("infiniterm").is_file() || version.trim().is_empty() {
        return Err(format!("{} holds no infiniterm build", r.tag));
    }
    if version.trim() != r.version {
        return Err(format!(
            "{} carries version {} where its manifest says {}",
            r.tag,
            version.trim(),
            r.version
        ));
    }
    // install.sh's own two steps: beside it as `.new`, then over the folder
    // of that name (a repair of the same version replaces it; a running copy
    // keeps the files it has open).
    let dest = root.join(&r.version);
    let next = root.join(format!("{}.new", r.version));
    let _ = std::fs::remove_dir_all(&next);
    std::fs::rename(&build, &next).map_err(|e| format!("{}: {e}", next.display()))?;
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&next, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    Ok(dest)
}

/// The version folder `current` names, if it is a link.
pub fn current_version(root: &Path) -> Option<String> {
    let target = std::fs::read_link(root.join(CURRENT)).ok()?;
    Some(target.file_name()?.to_str()?.to_string())
}

/// Point `<root>/current` at `<root>/<version>` in one step: a new link
/// beside it, renamed over it, so there is no moment without one. Relative,
/// as install.sh's `ln -sfn "$version"` makes it.
#[cfg(unix)]
pub fn repoint_current(root: &Path, version: &str) -> Result<(), String> {
    if !valid_version(version) || !root.join(version).is_dir() {
        return Err(format!("no staged version {version} in {}", root.display()));
    }
    let next = root.join(CURRENT_NEXT);
    let _ = std::fs::remove_file(&next);
    std::os::unix::fs::symlink(version, &next).map_err(|e| format!("{}: {e}", next.display()))?;
    std::fs::rename(&next, root.join(CURRENT)).map_err(|e| {
        let _ = std::fs::remove_file(&next);
        format!("could not move {}: {e}", root.join(CURRENT).display())
    })
}

/// Remove every version folder except those in `keep`: what install.sh does
/// after an install, keeping the new one and the one that was running. The
/// links, a `.new` folder and hidden entries are left alone.
pub fn prune(root: &Path, keep: &[&str]) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for e in entries.flatten() {
        let Some(name) = e.file_name().to_str().map(String::from) else {
            continue;
        };
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        if !is_dir
            || name.starts_with('.')
            || name.ends_with(".new")
            || keep.contains(&name.as_str())
        {
            continue;
        }
        let _ = std::fs::remove_dir_all(e.path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ift-lu-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A release tarball as `tools/linux-dist.sh` lays it out, with this
    /// VERSION, written to `<dir>/t.tar.gz`; returns its path. Only the unix
    /// tests stage from it, so a Windows test build leaves it out.
    #[cfg(unix)]
    fn tarball(dir: &Path, version: &str) -> PathBuf {
        let src = dir.join("src");
        let build = src.join(TARBALL_DIR);
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(build.join("infiniterm"), "#!/bin/sh\n").unwrap();
        std::fs::write(build.join("VERSION"), format!("{version}\n")).unwrap();
        let out = dir.join("t.tar.gz");
        let ok = std::process::Command::new("tar")
            .arg("-czf")
            .arg(&out)
            .arg("-C")
            .arg(&src)
            .arg(TARBALL_DIR)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        std::fs::remove_dir_all(&src).unwrap();
        out
    }

    fn release(version: &str, sha256: &str) -> LinuxRelease {
        LinuxRelease {
            version: version.into(),
            build: 1800,
            tag: format!("v{version}"),
            url: "https://github.com/ekinertac/infiniterm/releases/download/x.tar.gz".into(),
            sha256: sha256.into(),
            signature: None,
        }
    }

    /// An install.sh install of `version`: its folder and `current`.
    #[cfg(unix)]
    fn installed(root: &Path, version: &str) {
        std::fs::create_dir_all(root.join(version)).unwrap();
        std::fs::write(root.join(version).join("infiniterm"), "old").unwrap();
        std::os::unix::fs::symlink(version, root.join(CURRENT)).unwrap();
    }

    #[cfg(unix)]
    fn copy_from(src: PathBuf) -> impl FnOnce(&str, &Path) -> Result<(), String> {
        move |_url, dest| {
            std::fs::copy(&src, dest)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_good_tarball_is_staged_beside_the_running_version_and_current_stays() {
        let dir = scratch();
        let root = dir.join("root");
        installed(&root, "0.6.16");
        let t = tarball(&dir, "0.6.17");
        let sha = sha256_file(&t).unwrap();
        let staged = stage(
            &release("0.6.17", &sha.to_uppercase()),
            &root,
            "",
            copy_from(t),
        )
        .unwrap();
        assert_eq!(staged, root.join("0.6.17"));
        assert!(root.join("0.6.17/infiniterm").is_file());
        assert_eq!(current_version(&root).as_deref(), Some("0.6.16"));
        assert!(!root.join(SCRATCH).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // Ekin's test (#840): a failed download or a bad sha256 leaves `current`
    // and the folders as they were.
    #[cfg(unix)]
    #[test]
    fn a_failed_download_or_a_bad_checksum_leaves_current_untouched() {
        let dir = scratch();
        let root = dir.join("root");
        installed(&root, "0.6.16");
        let before = |root: &Path| {
            let mut names: Vec<String> = std::fs::read_dir(root)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        let names = before(&root);

        let failed = stage(&release("0.6.17", "00"), &root, "", |_, _| {
            Err("curl failed: 404".to_string())
        });
        assert_eq!(failed.unwrap_err(), "curl failed: 404");
        assert_eq!(before(&root), names);
        assert_eq!(current_version(&root).as_deref(), Some("0.6.16"));

        let t = tarball(&dir, "0.6.17");
        let bad = stage(
            &release("0.6.17", &"0".repeat(64)),
            &root,
            "",
            copy_from(t.clone()),
        );
        assert!(bad.unwrap_err().contains("does not match its checksum"));
        assert_eq!(before(&root), names);
        assert_eq!(current_version(&root).as_deref(), Some("0.6.16"));
        assert_eq!(
            std::fs::read_to_string(root.join("0.6.16/infiniterm")).unwrap(),
            "old"
        );

        // Not a tarball at all, with a matching hash: refused at the unpack.
        let junk = dir.join("junk");
        std::fs::write(&junk, "not a tarball").unwrap();
        let sha = sha256_file(&junk).unwrap();
        assert!(stage(&release("0.6.17", &sha), &root, "", copy_from(junk))
            .unwrap_err()
            .contains("could not unpack"));
        assert_eq!(before(&root), names);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_tarball_of_another_version_than_its_manifest_is_refused() {
        let dir = scratch();
        let root = dir.join("root");
        installed(&root, "0.6.16");
        let t = tarball(&dir, "0.6.15");
        let sha = sha256_file(&t).unwrap();
        let err = stage(&release("0.6.17", &sha), &root, "", copy_from(t)).unwrap_err();
        assert!(err.contains("carries version 0.6.15"), "{err}");
        assert!(!root.join("0.6.17").exists() && !root.join("0.6.15").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn with_a_key_set_an_unsigned_release_is_refused_before_any_download() {
        let root = scratch();
        let mut fetched = false;
        let err = stage(&release("0.6.17", "00"), &root, &"ab".repeat(32), |_, _| {
            fetched = true;
            Ok(())
        })
        .unwrap_err();
        assert!(err.contains("not signed"), "{err}");
        assert!(!fetched);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_version_that_could_leave_the_folder_is_refused() {
        for bad in ["", "..", "../x", "a/b", ".hidden", "1 2"] {
            assert!(!valid_version(bad), "{bad:?}");
        }
        assert!(valid_version("0.6.17"));
        assert!(valid_version("0.7.0-rc1+5"));
    }

    #[cfg(unix)]
    #[test]
    fn current_moves_in_one_step_and_two_versions_are_kept() {
        let dir = scratch();
        let root = dir.join("root");
        installed(&root, "0.6.16");
        for v in ["0.6.14", "0.6.15", "0.6.17", "0.6.18.new"] {
            std::fs::create_dir_all(root.join(v)).unwrap();
        }
        repoint_current(&root, "0.6.17").unwrap();
        assert_eq!(current_version(&root).as_deref(), Some("0.6.17"));
        // Relative, as install.sh writes it.
        assert_eq!(
            std::fs::read_link(root.join(CURRENT)).unwrap(),
            Path::new("0.6.17")
        );
        assert!(!root.join(CURRENT_NEXT).exists());
        prune(&root, &["0.6.17", "0.6.16"]);
        let mut left: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["0.6.16", "0.6.17", "0.6.18.new", "current"]);
        // A version that is not there is never pointed at.
        assert!(repoint_current(&root, "0.9.0").is_err());
        assert!(repoint_current(&root, "../x").is_err());
        assert_eq!(current_version(&root).as_deref(), Some("0.6.17"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
