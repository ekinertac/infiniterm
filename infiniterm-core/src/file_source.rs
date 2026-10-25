//! Where a card's files come from (#560): the seam between a card body and a
//! disk. `LocalSource` is the machine's own disk, exactly what the bodies read
//! before; `backend/file_client.rs`'s `HostSource` is another machine's, over
//! the join file service. A body holds an `Arc<dyn FileSource>` and asks it, so
//! the same card shows a local file or a host's.
//!
//! Every call may block (a host's answer comes over ssh): a body that can face
//! a host source calls from a thread of its own and polls for the answer, and
//! `is_local` says whether it must (a local answer is microseconds, so a body
//! may keep asking inline).
//!
//! Called by the card bodies in the ui crate and by the host's helper
//! (`file_server.rs` answers from a `LocalSource`). Related: file_proto.rs
//! (the wire types reused here), files.rs (the local reads this wraps),
//! docs/agent/join-file-service.md (the design and what comes next).

use std::io::{Read, Seek, SeekFrom};

pub use crate::file_proto::{Code, EntryKind, FileError, ListEntry, Stat};

/// A whole file: its bytes and its modification time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileData {
    pub bytes: Vec<u8>,
    pub mtime_ms: Option<u64>,
}

impl FileData {
    /// The bytes as text, lossily: a card shows text and a stray byte must not
    /// refuse the whole file.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

/// The host's own idea of where it lives, for naming paths on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPaths {
    pub home: String,
    pub config_dir: String,
}

/// A file or folder being watched on a host. `poll` says whether it changed
/// since the last poll and whether the watch is over (the connection ended:
/// the owner asks again, with a `stat` or `list` of its own to catch what it
/// missed). Dropping it stops the watch on the host.
pub struct Watch {
    events: std::sync::mpsc::Receiver<()>,
    stop: Option<Box<dyn FnOnce() + Send>>,
}

/// What `Watch::poll` found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Watched {
    pub changed: bool,
    pub closed: bool,
}

impl Watch {
    pub fn new(events: std::sync::mpsc::Receiver<()>, stop: Box<dyn FnOnce() + Send>) -> Watch {
        Watch {
            events,
            stop: Some(stop),
        }
    }

    pub fn poll(&self) -> Watched {
        use std::sync::mpsc::TryRecvError;
        let mut out = Watched::default();
        loop {
            match self.events.try_recv() {
                Ok(()) => out.changed = true,
                Err(TryRecvError::Empty) => return out,
                Err(TryRecvError::Disconnected) => {
                    out.closed = true;
                    return out;
                }
            }
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

/// A file's place in its git repository and its text at HEAD, for the
/// editor's gutter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHead {
    /// The repository root, absolute.
    pub repo: String,
    /// The file relative to it, with `/` whatever the host's separator.
    pub rel: String,
    /// The file at HEAD; empty for one HEAD did not have.
    pub text: String,
}

/// A git failure as a file error: "not a git repository" is the one answer a
/// card acts on (no gutter), anything else is the OS-level `io`.
fn git_error(path: &str, message: String) -> FileError {
    let code = if message.contains("not a git repository") {
        Code::NotFound
    } else {
        Code::Io
    };
    FileError::new(code, format!("{path}: {message}"))
}

pub trait FileSource: Send + Sync {
    /// The file's text at HEAD with its repository (`not_found` outside one).
    fn git_head(&self, _path: &str) -> Result<GitHead, FileError> {
        Err(FileError::new(Code::Unsupported, "git is not available"))
    }

    /// What changed under `path` against HEAD.
    fn git_changes(&self, _path: &str) -> Result<crate::git::Changes, FileError> {
        Err(FileError::new(Code::Unsupported, "git is not available"))
    }

    /// Who last touched each line of `path` (relative to `repo`).
    fn git_blame(&self, _repo: &str, _path: &str) -> Result<Vec<crate::git::BlameLine>, FileError> {
        Err(FileError::new(Code::Unsupported, "git is not available"))
    }

    /// Tells when `path` (a file, or a folder's entries) changes. A local
    /// source does not: its bodies keep their own cheap poll, so the default
    /// is `unsupported`, which is also what an older host's helper answers.
    fn watch(&self, _path: &str) -> Result<Watch, FileError> {
        Err(FileError::new(
            Code::Unsupported,
            "watching is not available",
        ))
    }

    /// The machine's own disk: answers at once, so a body may call inline.
    /// False for another machine's, where a call may take seconds.
    fn is_local(&self) -> bool;
    fn home(&self) -> Result<HostPaths, FileError>;
    fn stat(&self, path: &str) -> Result<Stat, FileError>;
    /// What each path is, or `None` where there is nothing: the batch a click
    /// on output asks (`links_fs::path_kinds` is today's local form).
    fn exists(&self, paths: &[String]) -> Result<Vec<Option<EntryKind>>, FileError>;
    fn list(&self, path: &str) -> Result<Vec<ListEntry>, FileError>;
    /// The whole file.
    fn read(&self, path: &str) -> Result<FileData, FileError>;
    /// Replaces the whole file, through a temp file and a rename. With
    /// `expect_mtime_ms` the write is refused (`ChangedOnHost`, carrying the
    /// file's mtime now) if the file is not the one that was read; a file
    /// that is gone is no conflict. Returns the new mtime.
    fn write(
        &self,
        path: &str,
        bytes: &[u8],
        expect_mtime_ms: Option<u64>,
    ) -> Result<Option<u64>, FileError>;
}

/// The most a read returns: a card holds the file in memory, and a stray
/// multi-gigabyte log must not take the machine with it.
pub const MAX_READ: u64 = 128 * 1024 * 1024;

/// This machine's disk.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalSource;

fn kind_of(meta: &std::fs::Metadata) -> EntryKind {
    if meta.is_dir() {
        EntryKind::Dir
    } else if meta.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

fn mtime_of(meta: &std::fs::Metadata) -> Option<u64> {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
}

/// `~` and `~/x` are this machine's home; an absolute path stays itself; any
/// other path is refused (the caller resolves against a card's directory
/// first).
pub fn expand(path: &str) -> Result<std::path::PathBuf, FileError> {
    if path == "~" {
        return Ok(crate::paths::home_dir());
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return Ok(crate::paths::home_dir().join(rest));
    }
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        Ok(p.to_path_buf())
    } else {
        Err(FileError::new(
            Code::Io,
            format!("{path}: not an absolute path"),
        ))
    }
}

impl LocalSource {
    /// `len` bytes from `offset` (to the end when `None`), and the file's
    /// size and mtime: what the host's helper sends in chunks.
    pub fn read_range(
        &self,
        path: &str,
        offset: u64,
        len: Option<u64>,
    ) -> Result<(Vec<u8>, u64, Option<u64>), FileError> {
        let full = expand(path)?;
        let io = |e: std::io::Error| FileError::from_io(path, &e);
        // Looked at before it is opened: on Windows opening a folder fails
        // with "access denied", which would hide what it is.
        if std::fs::metadata(&full).map_err(io)?.is_dir() {
            return Err(FileError::new(
                Code::IsADirectory,
                format!("{path}: is a directory"),
            ));
        }
        let mut file = std::fs::File::open(&full).map_err(io)?;
        let meta = file.metadata().map_err(io)?;
        let size = meta.len();
        let want = len.unwrap_or(size.saturating_sub(offset));
        if want > MAX_READ {
            return Err(FileError::new(
                Code::TooLarge,
                format!("{path}: {want} bytes is more than a card opens ({MAX_READ})"),
            ));
        }
        file.seek(SeekFrom::Start(offset)).map_err(io)?;
        let mut bytes = Vec::new();
        file.take(want).read_to_end(&mut bytes).map_err(io)?;
        Ok((bytes, size, mtime_of(&meta)))
    }
}

impl FileSource for LocalSource {
    fn is_local(&self) -> bool {
        true
    }

    fn home(&self) -> Result<HostPaths, FileError> {
        Ok(HostPaths {
            home: crate::paths::home_dir().to_string_lossy().into_owned(),
            config_dir: crate::paths::config_dir().to_string_lossy().into_owned(),
        })
    }

    fn stat(&self, path: &str) -> Result<Stat, FileError> {
        let full = expand(path)?;
        let meta = std::fs::metadata(&full).map_err(|e| FileError::from_io(path, &e))?;
        let symlink = std::fs::symlink_metadata(&full)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        Ok(Stat {
            kind: kind_of(&meta),
            size: meta.len(),
            mtime_ms: mtime_of(&meta),
            symlink,
        })
    }

    fn exists(&self, paths: &[String]) -> Result<Vec<Option<EntryKind>>, FileError> {
        Ok(paths
            .iter()
            .map(|p| {
                let full = expand(p).ok()?;
                let meta = std::fs::metadata(full).ok()?;
                Some(kind_of(&meta))
            })
            .collect())
    }

    fn list(&self, path: &str) -> Result<Vec<ListEntry>, FileError> {
        let full = expand(path)?;
        // `dir_list` is the one rule for a folder: sorted, `.git` left out,
        // a link to a folder is a folder.
        let entries = crate::files::dir_list(&full.to_string_lossy()).map_err(|e| {
            let code = if full.is_file() {
                Code::NotADirectory
            } else if !full.exists() {
                Code::NotFound
            } else {
                Code::Io
            };
            FileError::new(code, e)
        })?;
        Ok(entries
            .into_iter()
            .map(|e| ListEntry {
                name: e.name,
                is_dir: e.is_dir,
            })
            .collect())
    }

    fn read(&self, path: &str) -> Result<FileData, FileError> {
        let (bytes, _size, mtime_ms) = self.read_range(path, 0, None)?;
        Ok(FileData { bytes, mtime_ms })
    }

    fn git_head(&self, path: &str) -> Result<GitHead, FileError> {
        let full = expand(path)?;
        let repo = crate::git::repo_of(&full).map_err(|e| git_error(path, e))?;
        let repo = repo.to_string_lossy().into_owned();
        // As paths, not text: on Windows git's root has forward slashes and
        // the file's path has backslashes.
        // git names the root with symlinks resolved (/private/var for
        // /var on a Mac), the path asked about may not have them resolved.
        let under = |p: &std::path::Path| p.strip_prefix(&repo).map(|r| r.to_path_buf()).ok();
        let rel = under(&full)
            .or_else(|| std::fs::canonicalize(&full).ok().and_then(|c| under(&c)))
            .ok_or_else(|| FileError::new(Code::NotFound, format!("{path}: not inside {repo}")))?
            .to_string_lossy()
            .replace('\\', "/");
        let text = crate::git::git_show_head(&repo, &rel).map_err(|e| git_error(path, e))?;
        Ok(GitHead { repo, rel, text })
    }

    fn git_changes(&self, path: &str) -> Result<crate::git::Changes, FileError> {
        let full = expand(path)?;
        crate::git::git_changes(&full.to_string_lossy()).map_err(|e| git_error(path, e))
    }

    fn git_blame(&self, repo: &str, path: &str) -> Result<Vec<crate::git::BlameLine>, FileError> {
        crate::git::git_blame(repo, path).map_err(|e| git_error(path, e))
    }

    fn write(
        &self,
        path: &str,
        bytes: &[u8],
        expect_mtime_ms: Option<u64>,
    ) -> Result<Option<u64>, FileError> {
        let full = expand(path)?;
        if bytes.len() as u64 > MAX_READ {
            return Err(FileError::new(
                Code::TooLarge,
                format!("{path}: {} bytes is more than a card holds", bytes.len()),
            ));
        }
        // Checked where the write happens, just before it: a viewer's idea of
        // the file is the mtime it read, and the file may have moved since.
        if let (Some(expected), Ok(meta)) = (expect_mtime_ms, std::fs::metadata(&full)) {
            let now = mtime_of(&meta);
            if now != Some(expected) {
                return Err(FileError::new(
                    Code::ChangedOnHost,
                    format!("{path}: changed on the host since it was read"),
                )
                .with_mtime(now));
            }
        }
        crate::files::write_bytes_atomically(&full, bytes)
            .map_err(|e| FileError::from_io(path, &e))?;
        Ok(std::fs::metadata(&full).ok().and_then(|m| mtime_of(&m)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("fs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "hello\n").unwrap();
        std::fs::write(dir.join("sub/b.txt"), "b").unwrap();
        dir
    }

    fn p(dir: &std::path::Path, rest: &str) -> String {
        dir.join(rest).to_string_lossy().into_owned()
    }

    #[test]
    fn a_local_source_stats_lists_and_reads_like_the_disk_it_is() {
        let dir = scratch("local");
        let src = LocalSource;
        assert!(src.is_local());
        let s = src.stat(&p(&dir, "a.txt")).unwrap();
        assert_eq!((s.kind, s.size), (EntryKind::File, 6));
        assert!(s.mtime_ms.is_some() && !s.symlink);
        assert_eq!(src.stat(&p(&dir, "sub")).unwrap().kind, EntryKind::Dir);
        let names: Vec<_> = src
            .list(&dir.to_string_lossy())
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.is_dir))
            .collect();
        assert_eq!(
            names,
            vec![("sub".to_string(), true), ("a.txt".to_string(), false)],
            "folders first, as the tree sorts them"
        );
        let read = src.read(&p(&dir, "a.txt")).unwrap();
        assert_eq!(read.text(), "hello\n");
        assert_eq!(
            src.exists(&[p(&dir, "a.txt"), p(&dir, "sub"), p(&dir, "nope")])
                .unwrap(),
            vec![Some(EntryKind::File), Some(EntryKind::Dir), None]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failure_has_the_code_a_card_can_act_on() {
        let dir = scratch("codes");
        let src = LocalSource;
        assert_eq!(
            src.read(&p(&dir, "missing")).unwrap_err().code,
            Code::NotFound
        );
        assert_eq!(
            src.read(&p(&dir, "sub")).unwrap_err().code,
            Code::IsADirectory
        );
        assert_eq!(
            src.list(&p(&dir, "a.txt")).unwrap_err().code,
            Code::NotADirectory
        );
        assert_eq!(
            src.list(&p(&dir, "missing")).unwrap_err().code,
            Code::NotFound
        );
        let relative = src.read("a.txt").unwrap_err();
        assert!(relative.message.contains("not an absolute path"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_replaces_the_file_and_is_refused_when_it_changed_since_it_was_read() {
        let dir = scratch("write");
        let src = LocalSource;
        let path = p(&dir, "a.txt");
        let read = src.read(&path).unwrap();
        // The file as read: the write goes through and says the new mtime.
        let mtime = src.write(&path, b"new text\n", read.mtime_ms).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new text\n");
        assert_eq!(mtime, src.stat(&path).unwrap().mtime_ms);
        // Somebody else writes it (a later mtime): a write that expects the
        // old one is refused, nothing is written, and the error carries the
        // mtime to expect next time.
        let theirs = std::fs::File::options().write(true).open(&path).unwrap();
        theirs
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
            .unwrap();
        let err = src.write(&path, b"mine\n", mtime).unwrap_err();
        assert_eq!(err.code, Code::ChangedOnHost);
        assert_eq!(err.mtime_ms, src.stat(&path).unwrap().mtime_ms);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new text\n");
        // Overwriting on purpose: expect what the refusal said.
        src.write(&path, b"mine\n", err.mtime_ms).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine\n");
        // A file being made has nothing to expect, and a file that is gone
        // is no conflict.
        let fresh = p(&dir, "sub/new.txt");
        src.write(&fresh, b"made", None).unwrap();
        std::fs::remove_file(&fresh).unwrap();
        src.write(&fresh, b"again", Some(1)).unwrap();
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "again");
        // Not a path: refused, not written.
        assert_eq!(src.write("rel.txt", b"x", None).unwrap_err().code, Code::Io);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_range_is_read_from_its_offset_and_a_tilde_is_the_home() {
        let dir = scratch("range");
        let (bytes, size, _) = LocalSource
            .read_range(&p(&dir, "a.txt"), 1, Some(3))
            .unwrap();
        assert_eq!((bytes.as_slice(), size), (&b"ell"[..], 6));
        assert_eq!(expand("~").unwrap(), crate::paths::home_dir());
        assert_eq!(expand("~/x").unwrap(), crate::paths::home_dir().join("x"));
        let home = LocalSource.home().unwrap();
        assert_eq!(home.home, crate::paths::home_dir().to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
