//! The viewer's end of the join file service (#560): `HostSource`, a
//! `FileSource` whose files are another machine's. One persistent connection
//! to the host's `ift proxy files`, made at the first request (no extra ssh
//! login for a window that never opens a file) and made again after it ends.
//!
//! A request gets an id, goes out as one frame, and its reply (`D` chunks then
//! `R`, or an `E`) is matched back by id, so any number of requests are in
//! flight at once and their replies interleave. A request waits on its own
//! channel; a reader thread owns the socket's read side. When the connection
//! ends every request still waiting fails at once with `Code::Lost`, and the
//! next request tries a new connection, no sooner than a back-off after a
//! failure (the cards' own numbers: 2 s doubling to 30 s) so a host that is
//! down is not asked on every keystroke.
//!
//! Every method blocks until the host answers: call from a thread (see
//! `FileSource::is_local`). Called by the card bodies of a joined window.
//! Related: file_proto.rs (the frames), file_server.rs (the host's end),
//! remote.rs (`connect_files`, the ssh), docs/agent/join-file-service.md.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::remote::{connect_files, RemoteHost};
use crate::file_proto::{
    Code, EntryKind, FileError, Frame, FrameReader, Kind, ListEntry, Request, Stat,
};
use crate::file_source::{FileData, FileSource, HostPaths};

/// How long a request waits for its reply before it is given up on. Generous:
/// a big read over a slow link is not a failure.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// The wait before a new connection is tried after `failures` in a row, as the
/// cards' links wait (`terminal_body::rejoin_wait_ms`): 2 s, doubling, up to 30 s.
fn retry_wait(failures: u32) -> Duration {
    Duration::from_secs((2u64 << failures.min(4)).min(30))
}

/// Makes a connection: an ssh to the host in real use, a socket pair to a
/// server thread in a test.
type Connector = Box<dyn Fn() -> Result<UnixStream, String> + Send + Sync>;

struct Conn {
    writer: Mutex<UnixStream>,
    waiting: Mutex<HashMap<u32, Sender<Frame>>>,
    next_id: AtomicU32,
    alive: AtomicBool,
}

#[derive(Default)]
struct State {
    conn: Option<Arc<Conn>>,
    failures: u32,
    retry_at: Option<Instant>,
}

pub struct HostSource {
    connector: Connector,
    label: String,
    state: Mutex<State>,
}

impl HostSource {
    /// A source for `host`. Nothing is connected until the first request.
    pub fn new(host: RemoteHost) -> Arc<HostSource> {
        let label = host.target.clone();
        Arc::new(HostSource {
            connector: Box::new(move || connect_files(&host)),
            label,
            state: Mutex::new(State::default()),
        })
    }

    /// A source over any way of making a connection, for a test.
    pub fn with_connector(label: &str, connector: Connector) -> Arc<HostSource> {
        Arc::new(HostSource {
            connector,
            label: label.to_string(),
            state: Mutex::new(State::default()),
        })
    }

    fn lost(&self, why: impl std::fmt::Display) -> FileError {
        FileError::new(Code::Lost, format!("{}: {why}", self.label))
    }

    /// The live connection, made now when there is none and the back-off
    /// allows.
    fn conn(&self) -> Result<Arc<Conn>, FileError> {
        let mut st = self.state.lock().unwrap();
        if let Some(c) = &st.conn {
            if c.alive.load(Ordering::SeqCst) {
                return Ok(c.clone());
            }
            st.conn = None;
        }
        if let Some(at) = st.retry_at {
            let now = Instant::now();
            if now < at {
                return Err(self.lost(format!(
                    "the host's files are unreachable; trying again in {} s",
                    (at - now).as_secs() + 1
                )));
            }
        }
        match (self.connector)() {
            Ok(stream) => {
                st.failures = 0;
                st.retry_at = None;
                let reader = stream
                    .try_clone()
                    .map_err(|e| self.lost(format!("could not use the connection: {e}")))?;
                let conn = Arc::new(Conn {
                    writer: Mutex::new(stream),
                    waiting: Mutex::new(HashMap::new()),
                    next_id: AtomicU32::new(1),
                    alive: AtomicBool::new(true),
                });
                let for_reader = conn.clone();
                std::thread::spawn(move || read_replies(reader, for_reader));
                st.conn = Some(conn.clone());
                Ok(conn)
            }
            Err(why) => {
                st.failures += 1;
                st.retry_at = Some(Instant::now() + retry_wait(st.failures - 1));
                Err(self.lost(why))
            }
        }
    }

    /// One request: its reply JSON and the bytes of its `D` frames.
    fn call(&self, request: &Request) -> Result<(Value, Vec<u8>), FileError> {
        let conn = self.conn()?;
        let id = conn.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = channel();
        conn.waiting.lock().unwrap().insert(id, tx);
        let frame = Frame::new(Kind::Request, id, request.to_json(), vec![]);
        let sent = {
            let mut w = conn.writer.lock().unwrap();
            w.write_all(&frame.encode()).and_then(|_| w.flush())
        };
        if let Err(e) = sent {
            conn.waiting.lock().unwrap().remove(&id);
            conn.alive.store(false, Ordering::SeqCst);
            return Err(self.lost(format!("could not send the request: {e}")));
        }
        let mut bytes = Vec::new();
        loop {
            match rx.recv_timeout(REQUEST_TIMEOUT) {
                Ok(f) => match f.kind {
                    Kind::Data => bytes.extend_from_slice(&f.bytes),
                    Kind::Reply => return Ok((f.json, bytes)),
                    Kind::Error => return Err(FileError::from_json(&f.json)),
                    _ => {}
                },
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(self.lost("the connection to the host ended"))
                }
                Err(RecvTimeoutError::Timeout) => {
                    conn.waiting.lock().unwrap().remove(&id);
                    return Err(self.lost("the host did not answer in time"));
                }
            }
        }
    }
}

/// Reads the connection's frames and hands each to the request waiting for
/// its id. When the stream ends, marks the connection dead and drops every
/// waiting sender, which fails every request still in flight.
fn read_replies(mut stream: UnixStream, conn: Arc<Conn>) {
    let mut reader = FrameReader::default();
    let mut buf = vec![0u8; 64 * 1024];
    'outer: loop {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => reader.feed(&buf[..n]),
        }
        loop {
            match reader.next() {
                Ok(Some(frame)) => {
                    let done = matches!(frame.kind, Kind::Reply | Kind::Error);
                    let mut waiting = conn.waiting.lock().unwrap();
                    if let Some(tx) = waiting.get(&frame.id) {
                        let _ = tx.send(frame.clone());
                    }
                    if done {
                        waiting.remove(&frame.id);
                    }
                }
                Ok(None) => break,
                Err(_) => break 'outer,
            }
        }
    }
    conn.alive.store(false, Ordering::SeqCst);
    conn.waiting.lock().unwrap().clear();
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

impl FileSource for HostSource {
    fn is_local(&self) -> bool {
        false
    }

    fn home(&self) -> Result<HostPaths, FileError> {
        let (v, _) = self.call(&Request::Home)?;
        Ok(HostPaths {
            home: str_of(&v, "home"),
            config_dir: str_of(&v, "config_dir"),
        })
    }

    fn stat(&self, path: &str) -> Result<Stat, FileError> {
        let (v, _) = self.call(&Request::Stat { path: path.into() })?;
        Ok(Stat {
            kind: EntryKind::parse(v.get("kind").and_then(Value::as_str).unwrap_or("other")),
            size: v.get("size").and_then(Value::as_u64).unwrap_or(0),
            mtime_ms: v.get("mtime_ms").and_then(Value::as_u64),
            symlink: v.get("symlink").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    fn exists(&self, paths: &[String]) -> Result<Vec<Option<EntryKind>>, FileError> {
        let (v, _) = self.call(&Request::Exists {
            paths: paths.to_vec(),
        })?;
        Ok(v.get("kinds")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|k| k.as_str().map(EntryKind::parse)).collect())
            .unwrap_or_default())
    }

    fn list(&self, path: &str) -> Result<Vec<ListEntry>, FileError> {
        let (v, _) = self.call(&Request::List { path: path.into() })?;
        Ok(v.get("entries")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|e| ListEntry {
                        name: str_of(e, "name"),
                        is_dir: e.get("is_dir").and_then(Value::as_bool).unwrap_or(false),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn read(&self, path: &str) -> Result<FileData, FileError> {
        let (v, bytes) = self.call(&Request::Read {
            path: path.into(),
            offset: 0,
            len: None,
        })?;
        Ok(FileData {
            bytes,
            mtime_ms: v.get("mtime_ms").and_then(Value::as_u64),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_server::serve;
    use std::sync::atomic::AtomicUsize;

    /// The handshake, as `connect_files` does it: hello out, the answer line
    /// in a byte at a time.
    fn shake(mut near: UnixStream) -> Result<UnixStream, String> {
        writeln!(near, "{}", crate::file_proto::hello_line()).map_err(|e| e.to_string())?;
        let mut line = Vec::new();
        let mut b = [0u8; 1];
        while near.read(&mut b).map_err(|e| e.to_string())? == 1 && b[0] != b'\n' {
            line.push(b[0]);
        }
        crate::file_proto::check_answer(&String::from_utf8_lossy(&line))?;
        Ok(near)
    }

    /// A connector whose far end is the real server on a thread.
    fn served(label: &str) -> Arc<HostSource> {
        HostSource::with_connector(label, Box::new(|| shake(real_server()?)))
    }

    /// A socket pair with the real server on the far end.
    fn real_server() -> Result<UnixStream, String> {
        let (near, far) = UnixStream::pair().map_err(|e| e.to_string())?;
        let w = far.try_clone().map_err(|e| e.to_string())?;
        std::thread::spawn(move || {
            let _ = serve(far, w);
        });
        Ok(near)
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("fc-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        dir
    }

    #[test]
    fn a_host_source_stats_lists_reads_and_resolves_home_through_the_real_server() {
        let dir = scratch("basic");
        std::fs::write(dir.join("a.txt"), "hello\n").unwrap();
        let src = served("h");
        assert!(!src.is_local());
        let path = |p: &str| dir.join(p).to_string_lossy().into_owned();
        let s = src.stat(&path("a.txt")).unwrap();
        assert_eq!((s.kind, s.size), (EntryKind::File, 6));
        assert_eq!(src.read(&path("a.txt")).unwrap().text(), "hello\n");
        let names: Vec<_> = src
            .list(&dir.to_string_lossy())
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["sub", "a.txt"]);
        assert_eq!(
            src.exists(&[path("a.txt"), path("sub"), path("nope")])
                .unwrap(),
            vec![Some(EntryKind::File), Some(EntryKind::Dir), None]
        );
        assert!(!src.home().unwrap().home.is_empty());
        // The host's errors keep their codes, with the path.
        let err = src.read(&path("missing")).unwrap_err();
        assert_eq!(err.code, Code::NotFound);
        assert!(err.message.contains("missing"));
        assert_eq!(src.read("relative.txt").unwrap_err().code, Code::Io);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_bigger_than_a_chunk_arrives_whole_and_in_order() {
        let dir = scratch("big");
        let big: Vec<u8> = (0..(3 * crate::file_proto::CHUNK + 1234))
            .map(|i| (i % 251) as u8)
            .collect();
        std::fs::write(dir.join("big.bin"), &big).unwrap();
        let got = served("h")
            .read(&dir.join("big.bin").to_string_lossy())
            .unwrap();
        assert_eq!(got.bytes.len(), big.len());
        assert!(got.bytes == big, "the bytes differ");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn requests_in_flight_together_each_get_their_own_reply() {
        let dir = scratch("many");
        let big = vec![7u8; 2 * crate::file_proto::CHUNK];
        std::fs::write(dir.join("slow.bin"), &big).unwrap();
        for i in 0..8 {
            std::fs::write(dir.join(format!("f{i}")), format!("file {i}")).unwrap();
        }
        let src = served("h");
        let mut workers = vec![];
        for i in 0..8 {
            let (src, dir) = (src.clone(), dir.clone());
            workers.push(std::thread::spawn(move || {
                // A big read and small ones interleaved on one connection.
                let slow = src.read(&dir.join("slow.bin").to_string_lossy()).unwrap();
                let small = src
                    .read(&dir.join(format!("f{i}")).to_string_lossy())
                    .unwrap();
                (slow.bytes.len(), small.text())
            }));
        }
        for (i, w) in workers.into_iter().enumerate() {
            let (n, text) = w.join().unwrap();
            assert_eq!(n, big.len());
            assert_eq!(
                text,
                format!("file {i}"),
                "a reply went to the wrong request"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_connection_lost_mid_read_fails_that_request_and_the_next_one_reconnects() {
        let dir = scratch("lost");
        std::fs::write(dir.join("a.txt"), "after").unwrap();
        let made = Arc::new(AtomicUsize::new(0));
        let counted = made.clone();
        let src = HostSource::with_connector(
            "h",
            Box::new(move || {
                if counted.fetch_add(1, Ordering::SeqCst) > 0 {
                    return shake(real_server()?);
                }
                // The first host dies in the middle of an answer: it says ok,
                // sends one data frame of a reply and never the reply, and
                // closes the connection.
                let (near, mut far) = UnixStream::pair().map_err(|e| e.to_string())?;
                std::thread::spawn(move || {
                    let mut b = [0u8; 1];
                    while far.read(&mut b).unwrap_or(0) == 1 && b[0] != b'\n' {}
                    let _ = writeln!(far, "ift-files 1 ok");
                    let mut reader = FrameReader::default();
                    let mut buf = [0u8; 4096];
                    loop {
                        let n = far.read(&mut buf).unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        reader.feed(&buf[..n]);
                        if let Ok(Some(request)) = reader.next() {
                            let half =
                                Frame::new(Kind::Data, request.id, Value::Null, b"half".to_vec());
                            let _ = far.write_all(&half.encode());
                            return; // dropped: the connection ends mid-reply
                        }
                    }
                });
                shake(near)
            }),
        );
        let path = dir.join("a.txt").to_string_lossy().into_owned();
        let err = src.read(&path).unwrap_err();
        assert_eq!(err.code, Code::Lost, "{err}");
        assert_eq!(made.load(Ordering::SeqCst), 1);
        // The next request makes a new connection (no failure to back off
        // from: the host had answered the handshake) and works.
        assert_eq!(src.read(&path).unwrap().text(), "after");
        assert_eq!(made.load(Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_host_that_cannot_be_reached_is_not_asked_again_at_once() {
        let tries = Arc::new(AtomicUsize::new(0));
        let counted = tries.clone();
        let src = HostSource::with_connector(
            "mini",
            Box::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Err("ssh: connect to host mini port 22: Operation timed out".into())
            }),
        );
        let first = src.read("/x").unwrap_err();
        assert_eq!(first.code, Code::Lost);
        assert!(first.message.contains("mini") && first.message.contains("timed out"));
        // Within the back-off the connector is not called again.
        let second = src.read("/x").unwrap_err();
        assert!(second.message.contains("trying again in"), "{second}");
        assert_eq!(tries.load(Ordering::SeqCst), 1);
        assert_eq!(retry_wait(0), Duration::from_secs(2));
        assert_eq!(retry_wait(1), Duration::from_secs(4));
        assert_eq!(retry_wait(9), Duration::from_secs(30));
    }

    #[test]
    fn a_request_the_host_does_not_know_is_unsupported_not_a_hang() {
        let src = served("h");
        // Sent by hand: the typed API only builds requests this build knows.
        let conn = src.conn().unwrap();
        let (tx, rx) = channel();
        conn.waiting.lock().unwrap().insert(900, tx);
        let frame = Frame::new(
            Kind::Request,
            900,
            serde_json::json!({ "op": "teleport" }),
            vec![],
        );
        conn.writer
            .lock()
            .unwrap()
            .write_all(&frame.encode())
            .unwrap();
        let reply = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(reply.kind, Kind::Error);
        assert_eq!(FileError::from_json(&reply.json).code, Code::Unsupported);
    }
}
