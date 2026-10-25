//! The wire of the join file service (#560): frames, requests and replies.
//! Pure: no socket, no process, no gpui, so it compiles on every platform and
//! both ends (the viewer's `HostSource`, the host's `ift proxy files`) test
//! against the same code. The design is `docs/agent/join-file-service.md`.
//!
//! A frame is binary, because a file is bytes:
//!
//! ```text
//! len:u32 BE | kind:u8 | id:u32 BE | jlen:u32 BE | json (jlen bytes) | bytes
//! ```
//!
//! `len` counts everything after itself. One frame is at most `MAX_FRAME`; a
//! read is cut into `CHUNK`-sized `D` frames before its `R`. Replies to
//! different ids interleave; the viewer matches them by id.
//!
//! Called by `backend/file_client.rs` (the viewer's end), `file_server.rs`
//! (the host's end) and the cli's `proxy files`. Related: file_source.rs (the
//! trait the cards use and `LocalSource`, which the server answers from).

use serde_json::{json, Value};

/// The protocol's major version, said in the handshake line.
pub const MAJOR: u32 = 1;

/// The largest frame either end accepts: a corrupt length must not allocate
/// the machine.
pub const MAX_FRAME: usize = 8 * 1024 * 1024;

/// How much of a file one `D` frame carries: small enough that a big read
/// does not hold up the replies of other requests behind it.
pub const CHUNK: usize = 256 * 1024;

/// Bytes before the JSON: kind, id and the JSON's length.
const HEAD: usize = 1 + 4 + 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Viewer to host: a request.
    Request,
    /// The final reply to a request.
    Reply,
    /// A data chunk of a reply, before its `Reply`.
    Data,
    /// An error reply.
    Error,
    /// An event with no request (a watch fired); `id` is the watch's.
    Event,
}

impl Kind {
    fn byte(self) -> u8 {
        match self {
            Kind::Request => b'Q',
            Kind::Reply => b'R',
            Kind::Data => b'D',
            Kind::Error => b'E',
            Kind::Event => b'W',
        }
    }

    fn from_byte(b: u8) -> Option<Kind> {
        Some(match b {
            b'Q' => Kind::Request,
            b'R' => Kind::Reply,
            b'D' => Kind::Data,
            b'E' => Kind::Error,
            b'W' => Kind::Event,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub kind: Kind,
    pub id: u32,
    pub json: Value,
    pub bytes: Vec<u8>,
}

impl Frame {
    pub fn new(kind: Kind, id: u32, json: Value, bytes: Vec<u8>) -> Frame {
        Frame {
            kind,
            id,
            json,
            bytes,
        }
    }

    /// The frame as bytes. Panics only on a frame over `MAX_FRAME`, which no
    /// caller builds (reads are chunked).
    pub fn encode(&self) -> Vec<u8> {
        let json = serde_json::to_vec(&self.json).expect("a JSON value always serialises");
        let len = HEAD + json.len() + self.bytes.len();
        assert!(len <= MAX_FRAME, "a frame of {len} bytes is over the limit");
        let mut out = Vec::with_capacity(4 + len);
        out.extend_from_slice(&(len as u32).to_be_bytes());
        out.push(self.kind.byte());
        out.extend_from_slice(&self.id.to_be_bytes());
        out.extend_from_slice(&(json.len() as u32).to_be_bytes());
        out.extend_from_slice(&json);
        out.extend_from_slice(&self.bytes);
        out
    }
}

/// Why a stream is not ours any more: the connection should be dropped.
#[derive(Debug, PartialEq, Eq)]
pub enum ProtoError {
    TooLarge(usize),
    BadKind(u8),
    Truncated,
    BadJson,
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtoError::TooLarge(n) => write!(f, "a frame of {n} bytes is over the limit"),
            ProtoError::BadKind(b) => write!(f, "unknown frame kind {b}"),
            ProtoError::Truncated => write!(f, "a frame shorter than its own header"),
            ProtoError::BadJson => write!(f, "a frame whose JSON does not parse"),
        }
    }
}

/// Incremental frame reader: feed it what the socket gave, take whole frames.
/// A frame split across reads is rejoined, several in one read all come back.
#[derive(Debug, Default)]
pub struct FrameReader {
    buf: Vec<u8>,
}

impl FrameReader {
    pub fn feed(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Frame>, ProtoError> {
        if self.buf.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
        if len > MAX_FRAME {
            return Err(ProtoError::TooLarge(len));
        }
        if len < HEAD {
            return Err(ProtoError::Truncated);
        }
        if self.buf.len() < 4 + len {
            return Ok(None);
        }
        let body = &self.buf[4..4 + len];
        let kind = Kind::from_byte(body[0]).ok_or(ProtoError::BadKind(body[0]))?;
        let id = u32::from_be_bytes([body[1], body[2], body[3], body[4]]);
        let jlen = u32::from_be_bytes([body[5], body[6], body[7], body[8]]) as usize;
        if HEAD + jlen > len {
            return Err(ProtoError::Truncated);
        }
        let json =
            serde_json::from_slice(&body[HEAD..HEAD + jlen]).map_err(|_| ProtoError::BadJson)?;
        let bytes = body[HEAD + jlen..].to_vec();
        self.buf.drain(..4 + len);
        Ok(Some(Frame {
            kind,
            id,
            json,
            bytes,
        }))
    }
}

/// What kind of thing a path is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Other,
}

impl EntryKind {
    pub fn name(self) -> &'static str {
        match self {
            EntryKind::File => "file",
            EntryKind::Dir => "dir",
            EntryKind::Other => "other",
        }
    }

    pub fn parse(s: &str) -> EntryKind {
        match s {
            "file" => EntryKind::File,
            "dir" => EntryKind::Dir,
            _ => EntryKind::Other,
        }
    }
}

/// The error codes of an `E` reply (design doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    NotFound,
    Denied,
    NotADirectory,
    IsADirectory,
    TooLarge,
    ChangedOnHost,
    Io,
    Unsupported,
    /// Not on the wire: the connection ended with the request in flight, or
    /// could not be made. The viewer's own code.
    Lost,
}

impl Code {
    pub fn name(self) -> &'static str {
        match self {
            Code::NotFound => "not_found",
            Code::Denied => "denied",
            Code::NotADirectory => "not_a_directory",
            Code::IsADirectory => "is_a_directory",
            Code::TooLarge => "too_large",
            Code::ChangedOnHost => "changed_on_host",
            Code::Io => "io",
            Code::Unsupported => "unsupported",
            Code::Lost => "lost",
        }
    }

    pub fn parse(s: &str) -> Code {
        match s {
            "not_found" => Code::NotFound,
            "denied" => Code::Denied,
            "not_a_directory" => Code::NotADirectory,
            "is_a_directory" => Code::IsADirectory,
            "too_large" => Code::TooLarge,
            "changed_on_host" => Code::ChangedOnHost,
            "unsupported" => Code::Unsupported,
            "lost" => Code::Lost,
            _ => Code::Io,
        }
    }
}

/// A failed request: what went wrong and a sentence for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileError {
    pub code: Code,
    pub message: String,
    /// With `ChangedOnHost`: the file's modification time on the host now, so
    /// a viewer that means to overwrite it can say "that one" next time.
    pub mtime_ms: Option<u64>,
}

impl FileError {
    pub fn new(code: Code, message: impl Into<String>) -> FileError {
        FileError {
            code,
            message: message.into(),
            mtime_ms: None,
        }
    }

    pub fn with_mtime(mut self, mtime_ms: Option<u64>) -> FileError {
        self.mtime_ms = mtime_ms;
        self
    }

    /// An OS error, with the code its kind maps to.
    pub fn from_io(path: &str, e: &std::io::Error) -> FileError {
        use std::io::ErrorKind::*;
        let code = match e.kind() {
            NotFound => Code::NotFound,
            PermissionDenied => Code::Denied,
            _ => match e.raw_os_error() {
                Some(20) => Code::NotADirectory,
                Some(21) => Code::IsADirectory,
                _ => Code::Io,
            },
        };
        FileError::new(code, format!("{path}: {e}"))
    }

    pub fn to_json(&self) -> Value {
        json!({ "code": self.code.name(), "message": self.message, "mtime_ms": self.mtime_ms })
    }

    pub fn from_json(v: &Value) -> FileError {
        FileError {
            code: Code::parse(v.get("code").and_then(Value::as_str).unwrap_or("io")),
            message: v
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("the host answered with an error")
                .to_string(),
            mtime_ms: v.get("mtime_ms").and_then(Value::as_u64),
        }
    }
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// One folder entry, as `list` answers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    pub name: String,
    pub is_dir: bool,
}

/// What `stat` answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stat {
    pub kind: EntryKind,
    pub size: u64,
    pub mtime_ms: Option<u64>,
    pub symlink: bool,
}

/// A request, as it goes on the wire in a `Q` frame's JSON. Paths are the
/// host's and absolute (the helper expands `~`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Home,
    Stat {
        path: String,
    },
    Exists {
        paths: Vec<String>,
    },
    List {
        path: String,
    },
    Read {
        path: String,
        offset: u64,
        len: Option<u64>,
    },
    /// The whole file's new bytes, which ride the request as `D` frames of
    /// the same id (and the `Q` frame's own bytes). `expect_mtime_ms`: the
    /// write is refused with `changed_on_host` if the file's mtime is not
    /// this one (`None`: no check, a file being made).
    Write {
        path: String,
        expect_mtime_ms: Option<u64>,
    },
    /// Tell me when this file changes or this folder's entries do: `W`
    /// events carry THIS request's id, until `Unwatch` or the connection
    /// ends. The reply is sent once the watch is set, so nothing between the
    /// reply and the first event is missed.
    Watch {
        path: String,
    },
    /// Stops the watch that was requested with id `watch`.
    Unwatch {
        watch: u32,
    },
    /// The host's git (#560 step 4): the file's HEAD text. The reply carries
    /// `{ "repo", "rel" }` and the text as `D` chunks; `not_found` when the
    /// path is in no repository.
    GitHead {
        path: String,
    },
    /// A file sent from a viewer (#556), its bytes riding the request as `D`
    /// frames like a `write`'s: stored as `~/.infiniterm/drops/<id>/<name>`
    /// on the host after old drop folders are tidied. The reply is
    /// `{ "path" }`. Confined to the drops folder, and the only request that
    /// makes a folder.
    PutDrop {
        id: String,
        name: String,
    },
    /// What changed under `path` against HEAD (`git::git_changes`).
    GitChanges {
        path: String,
    },
    /// `rel` (relative to `repo`) at HEAD, as `D` chunks: the diff card's
    /// "before" side, where the file may be gone from the disk.
    GitShow {
        repo: String,
        path: String,
    },
    /// Who last touched each line of `path` in `repo` (`git::git_blame`).
    GitBlame {
        repo: String,
        path: String,
    },
}

impl Request {
    pub fn to_json(&self) -> Value {
        match self {
            Request::Home => json!({ "op": "home" }),
            Request::Stat { path } => json!({ "op": "stat", "path": path }),
            Request::Exists { paths } => json!({ "op": "exists", "paths": paths }),
            Request::List { path } => json!({ "op": "list", "path": path }),
            Request::Read { path, offset, len } => {
                json!({ "op": "read", "path": path, "offset": offset, "len": len })
            }
            Request::Write {
                path,
                expect_mtime_ms,
            } => json!({ "op": "write", "path": path, "expect_mtime_ms": expect_mtime_ms }),
            Request::Watch { path } => json!({ "op": "watch", "path": path }),
            Request::Unwatch { watch } => json!({ "op": "unwatch", "watch": watch }),
            Request::GitHead { path } => json!({ "op": "git_head", "path": path }),
            Request::PutDrop { id, name } => json!({ "op": "put_drop", "id": id, "name": name }),
            Request::GitChanges { path } => json!({ "op": "git_changes", "path": path }),
            Request::GitShow { repo, path } => {
                json!({ "op": "git_show", "repo": repo, "path": path })
            }
            Request::GitBlame { repo, path } => {
                json!({ "op": "git_blame", "repo": repo, "path": path })
            }
        }
    }

    /// `Err` carries the op's name when this helper does not know it (an
    /// `unsupported` reply), or a sentence when the request is malformed.
    pub fn from_json(v: &Value) -> Result<Request, FileError> {
        let op = v.get("op").and_then(Value::as_str).unwrap_or("");
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let need = |k: &str| {
            text(k).ok_or_else(|| FileError::new(Code::Io, format!("{op}: needs \"{k}\"")))
        };
        Ok(match op {
            "home" => Request::Home,
            "stat" => Request::Stat {
                path: need("path")?,
            },
            "exists" => Request::Exists {
                paths: v
                    .get("paths")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|p| p.as_str().map(str::to_string))
                            .collect()
                    })
                    .ok_or_else(|| FileError::new(Code::Io, "exists: needs \"paths\""))?,
            },
            "list" => Request::List {
                path: need("path")?,
            },
            "read" => Request::Read {
                path: need("path")?,
                offset: v.get("offset").and_then(Value::as_u64).unwrap_or(0),
                len: v.get("len").and_then(Value::as_u64),
            },
            "write" => Request::Write {
                path: need("path")?,
                expect_mtime_ms: v.get("expect_mtime_ms").and_then(Value::as_u64),
            },
            "watch" => Request::Watch {
                path: need("path")?,
            },
            "unwatch" => Request::Unwatch {
                watch: v
                    .get("watch")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| FileError::new(Code::Io, "unwatch: needs \"watch\""))?
                    as u32,
            },
            "git_head" => Request::GitHead {
                path: need("path")?,
            },
            "put_drop" => Request::PutDrop {
                id: need("id")?,
                name: need("name")?,
            },
            "git_changes" => Request::GitChanges {
                path: need("path")?,
            },
            "git_show" => Request::GitShow {
                repo: need("repo")?,
                path: need("path")?,
            },
            "git_blame" => Request::GitBlame {
                repo: need("repo")?,
                path: need("path")?,
            },
            other => {
                return Err(FileError::new(
                    Code::Unsupported,
                    format!("this host's ift does not know the request \"{other}\""),
                ))
            }
        })
    }
}

/// The handshake lines. The viewer says `ift-files <major>`; the helper
/// answers `ift-files <major> ok` or `ift-files <major> error <sentence>`.
pub fn hello_line() -> String {
    format!("ift-files {MAJOR}")
}

/// The major version in a viewer's `ift-files <major>` line, or `None` when
/// the line is not one.
pub fn parse_hello(line: &str) -> Option<u32> {
    let mut words = line.trim().split(' ');
    match (words.next(), words.next(), words.next()) {
        (Some("ift-files"), Some(major), None) => major.parse().ok(),
        _ => None,
    }
}

/// What the helper's answer line means: `Ok` or the host's sentence.
pub fn check_answer(line: &str) -> Result<(), String> {
    let line = line.trim_end();
    let mut words = line.splitn(4, ' ');
    match (words.next(), words.next(), words.next(), words.next()) {
        (Some("ift-files"), Some(_), Some("ok"), None) => Ok(()),
        (Some("ift-files"), Some(_), Some("error"), Some(why)) => Err(why.to_string()),
        _ => Err(format!(
            "the host did not answer as ift-files (update infiniterm there): {line:?}"
        )),
    }
}

/// `git::Changes` as the reply of `git_changes`.
pub fn changes_to_json(c: &crate::git::Changes) -> Value {
    json!({
        "repo": c.repo,
        "files": c.files.iter().map(|f| json!({
            "path": f.path, "added": f.added, "removed": f.removed, "status": f.status,
        })).collect::<Vec<_>>(),
    })
}

pub fn changes_from_json(v: &Value) -> crate::git::Changes {
    // The statuses git.rs makes; any other word reads as "modified" rather
    // than leaking a string this build did not mean.
    fn status(s: &str) -> &'static str {
        match s {
            "added" => "added",
            "deleted" => "deleted",
            "untracked" => "untracked",
            _ => "modified",
        }
    }
    crate::git::Changes {
        repo: v.get("repo").and_then(Value::as_str).unwrap_or("").into(),
        files: v
            .get("files")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|f| crate::git::ChangedFile {
                        path: f.get("path").and_then(Value::as_str).unwrap_or("").into(),
                        added: f.get("added").and_then(Value::as_u64).unwrap_or(0) as u32,
                        removed: f.get("removed").and_then(Value::as_u64).unwrap_or(0) as u32,
                        status: status(f.get("status").and_then(Value::as_str).unwrap_or("")),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// `git::BlameLine`s as the reply of `git_blame`.
pub fn blame_to_json(lines: &[crate::git::BlameLine]) -> Value {
    json!({
        "lines": lines.iter().map(|l| json!({
            "line": l.line, "hash": l.hash, "author": l.author,
            "time": l.time, "summary": l.summary,
        })).collect::<Vec<_>>(),
    })
}

pub fn blame_from_json(v: &Value) -> Vec<crate::git::BlameLine> {
    v.get("lines")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|l| crate::git::BlameLine {
                    line: l.get("line").and_then(Value::as_u64).unwrap_or(0) as u32,
                    hash: l.get("hash").and_then(Value::as_str).unwrap_or("").into(),
                    author: l.get("author").and_then(Value::as_str).unwrap_or("").into(),
                    time: l.get("time").and_then(Value::as_u64).unwrap_or(0),
                    summary: l
                        .get("summary")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into(),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(kind: Kind, id: u32, bytes: &[u8]) -> Frame {
        Frame::new(kind, id, json!({ "k": "v" }), bytes.to_vec())
    }

    #[test]
    fn every_kind_round_trips_with_its_id_json_and_bytes() {
        for kind in [
            Kind::Request,
            Kind::Reply,
            Kind::Data,
            Kind::Error,
            Kind::Event,
        ] {
            let f = frame(kind, 7, b"\x00\xffbytes");
            let mut r = FrameReader::default();
            r.feed(&f.encode());
            assert_eq!(r.next().unwrap(), Some(f));
            assert_eq!(r.next().unwrap(), None);
        }
    }

    #[test]
    fn a_frame_split_across_reads_is_rejoined_and_two_in_one_read_both_come_back() {
        let (a, b) = (frame(Kind::Data, 1, b"aaaa"), frame(Kind::Reply, 1, b""));
        let mut bytes = a.encode();
        bytes.extend(b.encode());
        let mut r = FrameReader::default();
        // One byte at a time: nothing until the first frame is whole.
        let mut got = vec![];
        for byte in &bytes {
            r.feed(&[*byte]);
            while let Some(f) = r.next().unwrap() {
                got.push(f);
            }
        }
        assert_eq!(got, vec![a.clone(), b.clone()]);
        // Both at once.
        let mut r = FrameReader::default();
        r.feed(&bytes);
        assert_eq!(r.next().unwrap(), Some(a));
        assert_eq!(r.next().unwrap(), Some(b));
    }

    #[test]
    fn a_frame_over_the_limit_or_with_an_unknown_kind_or_bad_json_is_refused() {
        let mut r = FrameReader::default();
        r.feed(&((MAX_FRAME as u32 + 1).to_be_bytes()));
        assert_eq!(r.next(), Err(ProtoError::TooLarge(MAX_FRAME + 1)));

        let mut bad = frame(Kind::Reply, 1, b"").encode();
        bad[4] = b'?';
        let mut r = FrameReader::default();
        r.feed(&bad);
        assert_eq!(r.next(), Err(ProtoError::BadKind(b'?')));

        // A length shorter than the header, and JSON that is not JSON.
        let mut r = FrameReader::default();
        r.feed(&[0, 0, 0, 3, b'R', 0, 0]);
        assert_eq!(r.next(), Err(ProtoError::Truncated));
        let mut body = vec![b'R', 0, 0, 0, 1, 0, 0, 0, 3];
        body.extend(b"{{{");
        let mut r = FrameReader::default();
        r.feed(&(body.len() as u32).to_be_bytes());
        r.feed(&body);
        assert_eq!(r.next(), Err(ProtoError::BadJson));
    }

    #[test]
    fn requests_round_trip_and_an_unknown_one_is_unsupported() {
        for req in [
            Request::Home,
            Request::Stat {
                path: "/a b".into(),
            },
            Request::Exists {
                paths: vec!["/a".into(), "~/b".into()],
            },
            Request::List { path: "/".into() },
            Request::Read {
                path: "/f".into(),
                offset: 10,
                len: Some(5),
            },
            Request::Read {
                path: "/f".into(),
                offset: 0,
                len: None,
            },
            Request::Write {
                path: "/f".into(),
                expect_mtime_ms: Some(5),
            },
            Request::Write {
                path: "/new".into(),
                expect_mtime_ms: None,
            },
            Request::Watch { path: "/d".into() },
            Request::Unwatch { watch: 9 },
            Request::GitHead {
                path: "/r/a.rs".into(),
            },
            Request::GitChanges { path: "/r".into() },
            Request::PutDrop {
                id: "1-0".into(),
                name: "shot.png".into(),
            },
            Request::GitShow {
                repo: "/r".into(),
                path: "a.rs".into(),
            },
            Request::GitBlame {
                repo: "/r".into(),
                path: "a.rs".into(),
            },
        ] {
            assert_eq!(Request::from_json(&req.to_json()).unwrap(), req);
        }
        let err = Request::from_json(&json!({ "op": "teleport" })).unwrap_err();
        assert_eq!(err.code, Code::Unsupported);
        let err = Request::from_json(&json!({ "op": "stat" })).unwrap_err();
        assert!(err.message.contains("path"));
    }

    #[test]
    fn git_answers_round_trip_through_json() {
        let changes = crate::git::Changes {
            repo: "/r".into(),
            files: vec![
                crate::git::ChangedFile {
                    path: "a.rs".into(),
                    added: 3,
                    removed: 1,
                    status: "modified",
                },
                crate::git::ChangedFile {
                    path: "n.rs".into(),
                    added: 9,
                    removed: 0,
                    status: "untracked",
                },
            ],
        };
        assert_eq!(changes_from_json(&changes_to_json(&changes)), changes);
        // A status word this build does not know reads as modified.
        let odd = json!({ "repo": "/r", "files": [{ "path": "x", "status": "teleported" }] });
        assert_eq!(changes_from_json(&odd).files[0].status, "modified");

        let blame = vec![crate::git::BlameLine {
            line: 2,
            hash: "abc1234".into(),
            author: "Ekin".into(),
            time: 1_700_000_000,
            summary: "why".into(),
        }];
        assert_eq!(blame_from_json(&blame_to_json(&blame)), blame);
    }

    #[test]
    fn errors_keep_their_code_through_json_and_the_os_maps_to_codes() {
        let e = FileError::new(Code::ChangedOnHost, "changed");
        assert_eq!(FileError::from_json(&e.to_json()), e);
        let with = FileError::new(Code::ChangedOnHost, "changed").with_mtime(Some(77));
        assert_eq!(FileError::from_json(&with.to_json()).mtime_ms, Some(77));
        let gone = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(FileError::from_io("/x", &gone).code, Code::NotFound);
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(FileError::from_io("/x", &denied).code, Code::Denied);
        assert_eq!(Code::parse("something new"), Code::Io);
    }

    #[test]
    fn the_handshake_answer_is_ok_the_hosts_sentence_or_not_an_answer() {
        assert_eq!(check_answer("ift-files 1 ok"), Ok(()));
        assert_eq!(
            check_answer("ift-files 1 error no such thing"),
            Err("no such thing".into())
        );
        assert!(check_answer("ift: unknown command proxy")
            .unwrap_err()
            .contains("update infiniterm there"));
        assert_eq!(hello_line(), "ift-files 1");
        assert_eq!(parse_hello("ift-files 1"), Some(1));
        assert_eq!(parse_hello("ift-files 7\r"), Some(7));
        assert_eq!(parse_hello("ift-join 2 canvas"), None);
        assert_eq!(parse_hello("ift-files one"), None);
    }
}
