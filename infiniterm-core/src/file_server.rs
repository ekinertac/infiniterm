//! The host's end of the join file service (#560): answers a viewer's
//! requests from this machine's own disk. `ift proxy files` is this over
//! stdin and stdout; tests run it over a socket pair.
//!
//! One thread per request, so a slow read never holds up a `stat` behind it;
//! replies leave under one lock, a whole frame at a time, so two replies never
//! mix inside a frame (their order between frames is free, the viewer matches
//! by id). It keeps no state between requests.
//!
//! Called by the cli's `proxy files`. Related: file_proto.rs (the frames),
//! file_source.rs (`LocalSource`, which does the reading),
//! backend/file_client.rs (the viewer's end).

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::file_proto::{
    parse_hello, Code, FileError, Frame, FrameReader, Kind, Request, CHUNK, MAJOR,
};
use crate::file_source::{FileSource, LocalSource};

/// One reply, whole, under the output lock.
fn send<W: Write>(out: &Mutex<W>, frame: &Frame) -> std::io::Result<()> {
    let bytes = frame.encode();
    let mut w = out.lock().unwrap();
    w.write_all(&bytes)?;
    w.flush()
}

/// How often a watch looks at its path. The helper looks, the viewer does
/// not: only a change crosses the link. shortcut: a poll of one `stat` or
/// one folder listing per watch, fine for the handful a card shows; swap in
/// the OS's file events if a host ever carries hundreds.
const WATCH_POLL: std::time::Duration = std::time::Duration::from_millis(400);

/// The live watches of one connection, by request id; true stops one.
type Watches = Arc<Mutex<std::collections::HashMap<u32, Arc<AtomicBool>>>>;

/// Stops every watch when the connection's loop ends, however it ends.
struct EndWatches(Watches);

impl Drop for EndWatches {
    fn drop(&mut self) {
        for stop in self.0.lock().unwrap().values() {
            stop.store(true, Ordering::SeqCst);
        }
    }
}

/// What a watch compares: a file's size and mtime, or a folder's names and
/// kinds (an entry's own edits are its file's watch's business). A path that
/// is not there is a state too, so its coming and going fires.
fn fingerprint(path: &str) -> String {
    let source = LocalSource;
    match source.stat(path) {
        Err(e) => format!("gone {}", e.code.name()),
        Ok(s) if s.kind == crate::file_proto::EntryKind::Dir => match source.list(path) {
            Ok(entries) => entries
                .iter()
                .map(|e| format!("{}{}", e.name, if e.is_dir { "/" } else { "" }))
                .collect::<Vec<_>>()
                .join("\n"),
            Err(e) => format!("unlistable {}", e.code.name()),
        },
        Ok(s) => format!("{} {:?}", s.size, s.mtime_ms),
    }
}

/// Starts a watch: the state now, then a thread that tells the viewer when
/// it differs. It ends when the viewer unwatches, or a send fails (the
/// connection is gone).
fn start_watch<W: Write + Send + 'static>(
    out: &Arc<Mutex<W>>,
    watches: &Watches,
    id: u32,
    path: String,
) {
    let stop = Arc::new(AtomicBool::new(false));
    watches.lock().unwrap().insert(id, stop.clone());
    let mut seen = fingerprint(&path);
    let (out, watches) = (out.clone(), watches.clone());
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(WATCH_POLL);
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let now = fingerprint(&path);
            if now != seen {
                seen = now;
                let event = Frame::new(Kind::Event, id, json!({ "paths": [path] }), vec![]);
                if send(&out, &event).is_err() {
                    break;
                }
            }
        }
        watches.lock().unwrap().remove(&id);
    });
}

/// Answers one request: its `D` chunks, then its `R`, or an `E`.
fn answer<W: Write + Send + 'static>(
    out: &Arc<Mutex<W>>,
    watches: &Watches,
    id: u32,
    request: Value,
    body: Vec<u8>,
) -> std::io::Result<()> {
    let source = LocalSource;
    let result: Result<(Value, Vec<u8>), FileError> = Request::from_json(&request).and_then(|r| {
        match r {
            Request::Home => {
                let h = source.home()?;
                Ok((json!({ "home": h.home, "config_dir": h.config_dir }), vec![]))
            }
            Request::Stat { path } => {
                let s = source.stat(&path)?;
                Ok((
                    json!({
                        "kind": s.kind.name(),
                        "size": s.size,
                        "mtime_ms": s.mtime_ms,
                        "symlink": s.symlink,
                    }),
                    vec![],
                ))
            }
            Request::Exists { paths } => {
                let kinds = source.exists(&paths)?;
                Ok((
                    json!({ "kinds": kinds.iter().map(|k| k.map(|k| k.name())).collect::<Vec<_>>() }),
                    vec![],
                ))
            }
            Request::List { path } => {
                let entries = source.list(&path)?;
                Ok((
                    json!({
                        "entries": entries
                            .iter()
                            .map(|e| json!({ "name": e.name, "is_dir": e.is_dir }))
                            .collect::<Vec<_>>()
                    }),
                    vec![],
                ))
            }
            Request::Read { path, offset, len } => {
                let (bytes, size, mtime_ms) = source.read_range(&path, offset, len)?;
                Ok((json!({ "size": size, "mtime_ms": mtime_ms }), bytes))
            }
            Request::Write {
                path,
                expect_mtime_ms,
            } => {
                let mtime_ms = source.write(&path, &body, expect_mtime_ms)?;
                Ok((json!({ "mtime_ms": mtime_ms }), vec![]))
            }
            Request::Watch { path } => {
                // Relative paths are refused like any request's.
                let full = crate::file_source::expand(&path)?;
                start_watch(out, watches, id, full.to_string_lossy().into_owned());
                Ok((json!({ "watch": id }), vec![]))
            }
            Request::GitHead { path } => {
                let head = source.git_head(&path)?;
                Ok((
                    json!({ "repo": head.repo, "rel": head.rel }),
                    head.text.into_bytes(),
                ))
            }
            Request::PutDrop { id, name } => {
                let path = source.put_drop(&id, &name, &body)?;
                Ok((json!({ "path": path }), vec![]))
            }
            Request::GitChanges { path } => {
                let changes = source.git_changes(&path)?;
                Ok((crate::file_proto::changes_to_json(&changes), vec![]))
            }
            Request::GitShow { repo, path } => {
                let text = source.git_show(&repo, &path)?;
                Ok((json!({}), text.into_bytes()))
            }
            Request::GitBlame { repo, path } => {
                let lines = source.git_blame(&repo, &path)?;
                Ok((crate::file_proto::blame_to_json(&lines), vec![]))
            }
            Request::Unwatch { watch } => {
                if let Some(stop) = watches.lock().unwrap().remove(&watch) {
                    stop.store(true, Ordering::SeqCst);
                }
                Ok((json!({}), vec![]))
            }
        }
    });
    match result {
        Ok((reply, bytes)) => {
            // The file's bytes in chunks, each its own frame, before the reply
            // that says it is complete.
            for chunk in bytes.chunks(CHUNK) {
                send(
                    out,
                    &Frame::new(Kind::Data, id, Value::Null, chunk.to_vec()),
                )?;
            }
            send(out, &Frame::new(Kind::Reply, id, reply, vec![]))
        }
        Err(e) => send(out, &Frame::new(Kind::Error, id, e.to_json(), vec![])),
    }
}

/// Why a read-only service refuses this request, or `None` when it may run.
/// What counts as a write is `Request::writes`, decided on the parsed
/// request, so a new request cannot slip through by name.
fn read_only_refusal(request: &Request) -> Option<FileError> {
    request
        .writes()
        .then(|| FileError::new(Code::Denied, "this file service is read-only"))
}

/// The handshake and then the request loop, until the viewer closes its end.
/// With `read_only` (`ift proxy files --read-only`, the only form a phone's
/// key may run) a `write` or `put_drop` is answered `denied` and runs nothing.
pub fn serve<R: Read, W: Write + Send + 'static>(
    mut input: R,
    output: W,
    read_only: bool,
) -> std::io::Result<()> {
    // The viewer's line, a byte at a time: nothing after it is consumed.
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    while line.len() < 256 {
        if input.read(&mut b)? == 0 {
            return Ok(());
        }
        if b[0] == b'\n' {
            break;
        }
        line.push(b[0]);
    }
    let out = Arc::new(Mutex::new(output));
    let said = String::from_utf8_lossy(&line).to_string();
    match parse_hello(&said) {
        Some(major) if major <= MAJOR => {
            let mut w = out.lock().unwrap();
            writeln!(w, "ift-files {MAJOR} ok")?;
            w.flush()?;
        }
        Some(_) => {
            let mut w = out.lock().unwrap();
            writeln!(
                w,
                "ift-files {MAJOR} error this host's infiniterm is older than yours (it speaks file service {MAJOR}); update it there"
            )?;
            return w.flush();
        }
        None => {
            let mut w = out.lock().unwrap();
            writeln!(w, "ift-files {MAJOR} error expected an ift-files line")?;
            return w.flush();
        }
    }

    let mut reader = FrameReader::default();
    let mut buf = vec![0u8; 64 * 1024];
    // The bytes of writes whose request has not arrived yet, by id. A
    // connection that ends drops them; nothing is written from a half.
    let mut partial: std::collections::HashMap<u32, Vec<u8>> = std::collections::HashMap::new();
    let watches = Watches::default();
    let _ends_watches = EndWatches(watches.clone());
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        reader.feed(&buf[..n]);
        loop {
            match reader.next() {
                // The bytes of a write come first, as data frames of the
                // request's id; they are kept until its request arrives.
                Ok(Some(frame)) if frame.kind == Kind::Data => {
                    partial.entry(frame.id).or_default().extend(frame.bytes);
                }
                Ok(Some(frame)) if frame.kind == Kind::Request => {
                    let mut body = partial.remove(&frame.id).unwrap_or_default();
                    // A request that does not parse goes on to `answer`,
                    // which replies with its error.
                    if let Some(refusal) = Request::from_json(&frame.json)
                        .ok()
                        .and_then(|r| read_only.then(|| read_only_refusal(&r)).flatten())
                    {
                        // Answered here, in order, with no thread: nothing
                        // of the request ever reaches the disk.
                        send(
                            &out,
                            &Frame::new(Kind::Error, frame.id, refusal.to_json(), vec![]),
                        )?;
                        continue;
                    }
                    let (out, watches) = (out.clone(), watches.clone());
                    body.extend(frame.bytes);
                    std::thread::spawn(move || {
                        let _ = answer(&out, &watches, frame.id, frame.json, body);
                    });
                }
                // Nothing else comes up from a viewer; it is ignored.
                Ok(Some(_)) => {}
                Ok(None) => break,
                // Not our stream any more: end it.
                Err(e) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        e.to_string(),
                    ))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer the test can read back after `serve` took it.
    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn read_only_refuses_write_and_put_drop_and_nothing_else() {
        let p = || "/x".to_string();
        for r in [
            Request::Write {
                path: p(),
                expect_mtime_ms: None,
            },
            Request::PutDrop {
                id: "i".into(),
                name: "n".into(),
            },
        ] {
            assert_eq!(read_only_refusal(&r).unwrap().code, Code::Denied);
        }
        // `Request::writes` has no `_` arm, so a new request must be
        // classed there; this lists the reads that exist now.
        for r in [
            Request::Home,
            Request::Stat { path: p() },
            Request::Exists { paths: vec![p()] },
            Request::List { path: p() },
            Request::Read {
                path: p(),
                offset: 0,
                len: None,
            },
            Request::Watch { path: p() },
            Request::Unwatch { watch: 1 },
            Request::GitHead { path: p() },
            Request::GitChanges { path: p() },
            Request::GitShow {
                repo: p(),
                path: p(),
            },
            Request::GitBlame {
                repo: p(),
                path: p(),
            },
        ] {
            assert!(read_only_refusal(&r).is_none(), "{r:?}");
        }
    }

    #[test]
    fn a_read_only_server_answers_a_write_with_denied_and_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("ift-ro-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("scratch.txt");
        let _ = std::fs::remove_file(&target);

        let mut input = format!("ift-files {MAJOR}\n").into_bytes();
        input.extend(Frame::new(Kind::Data, 7, json!({}), b"bytes".to_vec()).encode());
        let request = json!({ "op": "write", "path": target.to_string_lossy() });
        input.extend(Frame::new(Kind::Request, 7, request, vec![]).encode());

        let sink = Sink::default();
        serve(std::io::Cursor::new(input), sink.clone(), true).unwrap();

        let got = sink.0.lock().unwrap().clone();
        let hello = format!("ift-files {MAJOR} ok\n");
        assert!(got.starts_with(hello.as_bytes()));
        let mut reader = FrameReader::default();
        reader.feed(&got[hello.len()..]);
        let frame = reader.next().unwrap().unwrap();
        assert_eq!(frame.kind, Kind::Error);
        assert_eq!(frame.id, 7);
        assert_eq!(FileError::from_json(&frame.json).code, Code::Denied);
        assert!(reader.next().unwrap().is_none(), "one frame only");
        assert!(!target.exists(), "a refused write leaves no file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
