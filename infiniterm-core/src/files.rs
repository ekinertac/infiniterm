//! Reading and writing the file behind an editor card, the drafts that keep
//! an unsaved buffer across a quit, and one directory's entries for the
//! explorer. From the Tauri app's files.rs.
//!
//! Whole-file, as text: an editor card holds one file and saves it in one
//! write, which keeps "dirty" meaning something (the buffer differs from
//! the disk, or it does not). Writes go through a temp file and a rename so
//! a crash mid-save cannot leave a half-written file where a whole one was.
use crate::paths::drafts_dir;
use std::path::{Path, PathBuf};

pub fn file_read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

/// The file's modification time in ms since the epoch, or `None` if it is
/// gone. An editor card polls this to notice the file changing under it
/// (the app itself writes settings.json from the theme picker) and reloads
/// a clean buffer, or says so over a dirty one.
pub fn file_mtime(path: &str) -> Option<u64> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
}

pub fn file_write(path: &str, contents: &str) -> Result<(), String> {
    write_atomically(Path::new(path), contents).map_err(|e| format!("{path}: {e}"))
}

/// Where a write to `path` must land: the file a symlink points at, else
/// `path` as given. Every writer that renames a temp file into place calls
/// this first, or the rename replaces a link with a plain file (#219: a
/// `settings.json` kept in a dotfiles repo).
pub fn resolve_link(path: &Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Writes through a symlink: a rename onto the link would replace the link
/// itself with a regular file, so a `settings.json` symlinked from a
/// dotfiles repo stopped being a link at the first save (#219). The temp file
/// goes beside the TARGET, which is also the same filesystem, so the rename
/// stays atomic. A path that does not exist yet is made; a link to nothing is
/// replaced by a file, there being nothing to write through to.
pub fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let target = resolve_link(path);
    let path = target.as_path();
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.infiniterm-tmp"));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

/// Sublime's hot exit: one file per editor card holding its UNSAVED buffer,
/// written as you type and read back in place of the file when the card is
/// restored. A draft exists only while the buffer differs from the file:
/// saving deletes it, and so does closing the card (closing is the one act
/// that means discard; an unmount is a reload or a quit as often as a close
/// and must NOT delete it).
fn draft_path(card_id: &str) -> Result<PathBuf, String> {
    // Refuse anything that is not a plain name so an id cannot name a path
    // outside the directory.
    if card_id.is_empty()
        || card_id.contains('/')
        || card_id.contains('\\')
        || card_id.contains("..")
    {
        return Err("invalid card id".into());
    }
    let dir = drafts_dir();
    std::fs::create_dir_all(&dir).ok();
    Ok(dir.join(format!("{card_id}.txt")))
}

pub fn draft_write(card_id: &str, contents: &str) -> Result<(), String> {
    let path = draft_path(card_id)?;
    write_atomically(&path, contents).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn draft_read(card_id: &str) -> Result<Option<String>, String> {
    let path = draft_path(card_id)?;
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

pub fn draft_delete(card_id: &str) -> Result<(), String> {
    let path = draft_path(card_id)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Removes drafts for cards that no longer exist. Called after the layout
/// is loaded with every card id it contains; a draft whose card is gone is a
/// leak, not a backup.
pub fn draft_prune(keep: &[String]) {
    let Ok(entries) = std::fs::read_dir(drafts_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let id = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !keep.contains(&id) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// One directory's entries for the explorer: directories first, then
/// files, each group sorted case-insensitively. Dotfiles are listed (a
/// terminal user opens .zshrc more than most files) but `.git` is not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

pub fn dir_list(path: &str) -> Result<Vec<DirEntry>, String> {
    let entries = std::fs::read_dir(path).map_err(|e| format!("{path}: {e}"))?;
    let mut out: Vec<DirEntry> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                return None;
            }
            let file_type = e.file_type().ok()?;
            let is_dir = file_type.is_dir() || (file_type.is_symlink() && e.path().is_dir());
            Some(DirEntry {
                path: e.path(),
                name,
                is_dir,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_keeps_a_symlink_and_writes_the_file_it_points_at() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!("infiniterm-link-{}", std::process::id()));
        let (dots, config) = (root.join("dotfiles"), root.join("config"));
        std::fs::create_dir_all(&dots).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        let real = dots.join("settings.json");
        let link = config.join("settings.json");
        std::fs::write(&real, "old").unwrap();
        symlink(&real, &link).unwrap();
        write_atomically(&link, "new").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
        // a link that points at nothing has no target to write through, so
        // it is replaced by the file, as before
        let dangling = config.join("keys.json");
        symlink(dots.join("missing.json"), &dangling).unwrap();
        write_atomically(&dangling, "k").unwrap();
        assert!(!std::fs::symlink_metadata(&dangling).unwrap().is_symlink());
        assert_eq!(std::fs::read_to_string(&dangling).unwrap(), "k");
        // a new file is simply made
        let fresh = config.join("fresh.json");
        write_atomically(&fresh, "f").unwrap();
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "f");
        assert!(
            std::fs::read_dir(&config)
                .unwrap()
                .chain(std::fs::read_dir(&dots).unwrap())
                .all(|e| !e
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with("infiniterm-tmp")),
            "no temp file left behind"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolve_link_follows_a_link_and_leaves_other_paths_alone() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!("infiniterm-resolve-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let real = root.join("real.json");
        std::fs::write(&real, "x").unwrap();
        let link = root.join("link.json");
        symlink(&real, &link).unwrap();
        assert_eq!(resolve_link(&link), std::fs::canonicalize(&real).unwrap());
        let missing = root.join("nope.json");
        assert_eq!(resolve_link(&missing), missing);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn round_trips_and_replaces_whole() {
        let dir = std::env::temp_dir().join(format!("infiniterm-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        let p = file.to_string_lossy().into_owned();
        file_write(&p, "one\ntwo\n").unwrap();
        assert_eq!(file_read(&p).unwrap(), "one\ntwo\n");
        file_write(&p, "x").unwrap();
        assert_eq!(file_read(&p).unwrap(), "x");
        // No temp file left behind.
        assert!(!dir.join(".a.txt.infiniterm-tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_file_is_an_error_with_the_path_in_it() {
        let err = file_read("/no/such/file/anywhere").unwrap_err();
        assert!(err.contains("/no/such/file/anywhere"));
    }

    #[test]
    fn a_draft_round_trips_and_is_gone_after_delete() {
        let id = format!("test-{}", std::process::id());
        draft_write(&id, "half typed").unwrap();
        assert_eq!(draft_read(&id).unwrap().as_deref(), Some("half typed"));
        draft_delete(&id).unwrap();
        assert_eq!(draft_read(&id).unwrap(), None);
        draft_delete(&id).unwrap(); // deleting twice is fine
    }

    #[test]
    fn an_id_cannot_leave_the_directory() {
        assert!(draft_write("../x", "").is_err());
        assert!(draft_read("a/b").is_err());
    }

    #[test]
    fn lists_directories_first_and_skips_git() {
        let dir = std::env::temp_dir().join(format!("infiniterm-dir-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("b.txt"), "").unwrap();
        std::fs::write(dir.join("A.txt"), "").unwrap();
        let got = dir_list(&dir.to_string_lossy()).unwrap();
        let names: Vec<&str> = got.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["sub", "A.txt", "b.txt"]);
        assert!(got[0].is_dir && !got[1].is_dir);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
