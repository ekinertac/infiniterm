//! Opening what a terminal line points at, and saying which candidates
//! exist. From the Tauri app's links.rs; `links.rs` here finds the
//! candidates in a line, this side answers for the filesystem.
//!
//! Existence is decided BEFORE anything is underlined: a token that matches
//! the shape of a path but is a version number or a package name must not
//! look clickable. Paths are resolved against the card's directory (from the
//! process table) and `~` against the home directory, the two ways a shell
//! writes a path that is not absolute.
//!
//! Opening goes through the system's `open` (macOS); the per-OS seam is
//! `open_with_system`, the one function Linux and Windows replace.
use crate::paths::home_dir;
use std::path::{Path, PathBuf};

fn resolve(cwd: &str, path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    if path == "~" {
        return home_dir();
    }
    Path::new(cwd).join(path)
}

/// Which of `paths` exist, relative to `cwd`. One call per line of output
/// with links in it, so it answers for a batch.
pub fn paths_exist(cwd: &str, paths: &[&str]) -> Vec<bool> {
    paths.iter().map(|p| resolve(cwd, p).exists()).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    Dir,
    File,
}

/// What each path is: a directory, a file, or nothing. The caller decides
/// from this whether a click means a terminal card or an editor card.
pub fn path_kinds(cwd: &str, paths: &[&str]) -> Vec<Option<PathKind>> {
    paths
        .iter()
        .map(|p| {
            let full = resolve(cwd, p);
            if full.is_dir() {
                Some(PathKind::Dir)
            } else if full.is_file() {
                Some(PathKind::File)
            } else {
                None
            }
        })
        .collect()
}

/// Hands a path or URL to the system's default handler.
pub fn open_with_system(target: &Path) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn open_path(cwd: &str, path: &str) -> Result<(), String> {
    let full = resolve(cwd, path);
    if !full.exists() {
        return Err(format!("{} does not exist", full.display()));
    }
    open_with_system(&full)
}

pub fn open_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("not a web URL".into());
    }
    open_with_system(Path::new(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_to_cwd_and_tilde_to_home() {
        assert_eq!(resolve("/tmp", "a/b"), PathBuf::from("/tmp/a/b"));
        assert_eq!(resolve("/tmp", "/etc/hosts"), PathBuf::from("/etc/hosts"));
        let home = home_dir();
        assert_eq!(resolve("/tmp", "~/x"), home.join("x"));
        assert_eq!(resolve("/tmp", "~"), home);
    }

    #[test]
    fn existence_is_answered_per_path() {
        assert_eq!(
            paths_exist("/", &["etc", "no-such-thing-xyz"]),
            [true, false]
        );
    }

    #[test]
    fn kinds_tell_a_directory_from_a_file() {
        assert_eq!(
            path_kinds("/", &["etc", "etc/hosts", "nope-xyz"]),
            [Some(PathKind::Dir), Some(PathKind::File), None]
        );
    }

    #[test]
    fn refuses_to_open_what_is_not_there() {
        assert!(open_path("/", "no-such-thing-xyz").is_err());
        assert!(open_url("ftp://x").is_err());
    }
}
