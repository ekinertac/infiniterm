//! A host's pictures on this Mac (#560, step 5): an image in an editor card
//! and the canvas background list of a joined window name files on the
//! HOST's disk. gpui decodes a picture from a path on this machine's disk, so
//! a host picture is fetched through the file service into a cache folder
//! and the cached copy's path is what the painters get. Nothing else about
//! how a picture is drawn changes.
//!
//! The cache file is named from the host path, the file's mtime and size
//! (`<hash of path>-<mtime>-<size>.<ext>`), so a picture that changed on the
//! host is a new file, and gpui's per-path cache never shows the old one.
//! When one is made, the older copies of the same host path are removed.
//!
//! Called by `editor_body.rs` (an image file) and `background_show.rs` (the
//! `ui.backgroundImage` list in a viewer), on threads: both calls block on
//! the host. Related: file_source.rs (`FileSource`), background.rs (which
//! setting entries are bundled names).
//! shortcut: nothing clears the folder between runs except a copy being
//! replaced; at a few MB per picture that is fine for tens of pictures,
//! prune by age if a rotation of hundreds is ever pointed at a host.

use std::path::{Path, PathBuf};

use crate::file_source::{Code, FileError, FileSource};

/// Where host pictures are cached: under the instance's own data directory,
/// so a scratch instance leaves nothing in the real one.
pub fn cache_dir() -> PathBuf {
    crate::paths::app_support_dir().join("host-pictures")
}

/// FNV-1a: stable across runs and builds, unlike the std hasher, so a copy
/// made yesterday is found today.
fn hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

/// A host path in the cache folder `dir`: fetched when this version of the
/// file is not there yet. Returns the cached copy's path and the file's
/// mtime on the host (for the card's change poll).
pub fn fetch(
    source: &dyn FileSource,
    dir: &Path,
    host_path: &str,
) -> Result<(PathBuf, Option<u64>), FileError> {
    let stat = source.stat(host_path)?;
    let ext = Path::new(host_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("img");
    let prefix = format!("{:016x}-", hash(host_path));
    let name = format!("{prefix}{}-{}.{ext}", stat.mtime_ms.unwrap_or(0), stat.size);
    let target = dir.join(&name);
    if target.is_file() {
        return Ok((target, stat.mtime_ms));
    }
    let data = source.read(host_path)?;
    std::fs::create_dir_all(dir).map_err(|e| FileError::from_io(&dir.to_string_lossy(), &e))?;
    // Whole or not at all: a half-written copy under the final name would be
    // served as a broken picture until the file changed.
    let tmp = dir.join(format!("{name}.part"));
    std::fs::write(&tmp, &data.bytes)
        .map_err(|e| FileError::from_io(&tmp.to_string_lossy(), &e))?;
    std::fs::rename(&tmp, &target)
        .map_err(|e| FileError::from_io(&target.to_string_lossy(), &e))?;
    // The copies of earlier versions of the same file are no use now.
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let n = entry.file_name().to_string_lossy().into_owned();
            if n.starts_with(&prefix) && n != name {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok((target, stat.mtime_ms))
}

/// A `ui.backgroundImage` entry that names a file on the host: `~/x` (the
/// host's home) or an absolute path. A bundled name (no `/`, no extension)
/// is not: the app has those on every machine.
pub fn is_host_entry(setting: &str) -> bool {
    let s = setting.trim();
    !s.is_empty()
        && (s.starts_with("~/")
            || s.starts_with('/')
            || s.contains('/')
            || std::path::Path::new(s).extension().is_some())
}

/// The host's absolute path for an entry: `~/x` against the host's home.
pub fn host_path(source: &dyn FileSource, setting: &str) -> Result<String, FileError> {
    let s = setting.trim();
    match s.strip_prefix("~/") {
        Some(rest) => {
            let home = source.home()?.home;
            Ok(format!("{}/{rest}", home.trim_end_matches('/')))
        }
        None if s.starts_with('/') => Ok(s.to_string()),
        None => Err(FileError::new(
            Code::Io,
            format!("{s}: not a path on the host"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_source::LocalSource;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hp-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // The host's picture is copied once, found again without a second read,
    // and a changed picture is a new file with the old one removed.
    #[test]
    fn a_host_picture_is_cached_by_version_and_the_old_copy_goes() {
        let host = scratch("host");
        let cache = scratch("cache");
        let pic = host.join("sky.png").to_string_lossy().into_owned();
        std::fs::write(&pic, b"first bytes").unwrap();

        let (one, mtime) = fetch(&LocalSource, &cache, &pic).unwrap();
        assert_eq!(std::fs::read(&one).unwrap(), b"first bytes");
        assert!(mtime.is_some());
        assert_eq!(one.extension().unwrap(), "png");
        let (again, _) = fetch(&LocalSource, &cache, &pic).unwrap();
        assert_eq!(again, one, "the same version is not fetched twice");

        std::fs::write(&pic, b"second, longer bytes").unwrap();
        let (two, _) = fetch(&LocalSource, &cache, &pic).unwrap();
        assert_ne!(two, one);
        assert_eq!(std::fs::read(&two).unwrap(), b"second, longer bytes");
        assert!(!one.exists(), "the earlier copy was removed");
        let left: Vec<_> = std::fs::read_dir(&cache).unwrap().flatten().collect();
        assert_eq!(left.len(), 1, "no .part file is left");

        // Another file in the same folder is not touched by that clean-up.
        let other = host.join("sea.png").to_string_lossy().into_owned();
        std::fs::write(&other, b"sea").unwrap();
        let (sea, _) = fetch(&LocalSource, &cache, &other).unwrap();
        fetch(&LocalSource, &cache, &pic).unwrap();
        assert!(sea.exists());
        let _ = std::fs::remove_dir_all(&host);
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_picture_the_host_does_not_have_is_an_error_and_leaves_nothing() {
        let cache = scratch("none");
        let err = fetch(&LocalSource, &cache, "/no/such/pic.jpg").unwrap_err();
        assert_eq!(err.code, Code::NotFound);
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn setting_entries_are_bundled_names_or_the_hosts_files() {
        assert!(!is_host_entry("aurora"), "a bundled name");
        assert!(!is_host_entry("  "));
        assert!(is_host_entry("~/Pictures/a.jpg"));
        assert!(is_host_entry("/srv/a.jpg"));
        assert!(is_host_entry("a.jpg"));
        assert!(is_host_entry("pics/a"));
        assert_eq!(host_path(&LocalSource, "/srv/a.jpg").unwrap(), "/srv/a.jpg");
        let home = crate::paths::home_dir().to_string_lossy().into_owned();
        assert_eq!(
            host_path(&LocalSource, "~/x/a.jpg").unwrap(),
            format!("{home}/x/a.jpg")
        );
        assert_eq!(host_path(&LocalSource, "a.jpg").unwrap_err().code, Code::Io);
    }
}
