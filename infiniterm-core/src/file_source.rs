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

pub trait FileSource: Send + Sync {
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
        let mut file = std::fs::File::open(&full).map_err(io)?;
        let meta = file.metadata().map_err(io)?;
        if meta.is_dir() {
            return Err(FileError::new(
                Code::IsADirectory,
                format!("{path}: is a directory"),
            ));
        }
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
