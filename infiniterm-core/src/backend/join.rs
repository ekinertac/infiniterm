//! The host's join server (#423, `ift join`): the unix socket a viewer reaches
//! through `ssh host ift proxy join ...`. It serves two topics on one socket,
//! `<data>/j.sock`, mode 0600 (ssh is the only authentication; this is a
//! local socket that nothing else can open):
//!
//! - `canvas`: the host's layout, whole snapshots (the exact JSON the save
//!   file holds), the latest one at once and every later one as it is saved.
//!   Each is a 4-byte big-endian length and the UTF-8 text.
//! - `session <id>`: one terminal card's output, in the same frames the app
//!   reads from `iftd` (`session_protocol::Frame`): the host's hello (its
//!   columns and rows), the replay, then live output, from `Relay`. What a
//!   viewer sends back: `Data` goes to the session as keys; `Resize`, `Kill`
//!   and anything else is ignored, so a viewer never resizes, starts or ends a
//!   host session.
//!
//! A connection opens with one text line from the viewer,
//! `ift-join <major> <topic>`, and the server answers `ift-join <major> ok` or
//! `ift-join <major> error <sentence>` (and closes). The version lives here
//! and not in `session_protocol`, whose compatibility with old `iftd`s is a
//! project rule: this is the new layer. A viewer asking for a higher major
//! than the host knows is refused with a sentence; an equal or lower one is
//! served.
//!
//! Called by the app at startup (`JoinHub::serve`) and from the save path
//! (`publish_layout`). Related: relay.rs (the fan-out), daemon.rs (the bytes'
//! source and `write_session_now`), remote.rs and `infiniterm-cli/src/proxy.rs`
//! (the stdio splice the viewer's ssh uses).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};

use super::relay::Relay;
use super::session_protocol::{Frame, FrameReader};

/// The version this host speaks. A higher one asked for is refused.
pub const MAJOR: u32 = 1;

/// The longest handshake line accepted: a topic is a word and a session id.
const MAX_LINE: u64 = 256;

/// Snapshots queued for one canvas viewer before it is dropped. Snapshots are
/// whole, so a dropped viewer loses nothing but the wait: it reconnects and
/// gets the latest.
const CANVAS_QUEUE: usize = 4;

#[derive(Default)]
struct Canvas {
    latest: Option<String>,
    clients: Vec<SyncSender<String>>,
}

/// How a session's keys reach the host's own connection to `iftd`.
pub type WriteSession = Arc<dyn Fn(&str, &[u8]) + Send + Sync>;

#[derive(Clone)]
pub struct JoinHub {
    relay: Arc<Mutex<Relay>>,
    write: WriteSession,
    canvas: Arc<Mutex<Canvas>>,
}

impl JoinHub {
    pub fn new(relay: Arc<Mutex<Relay>>, write: WriteSession) -> JoinHub {
        JoinHub {
            relay,
            write,
            canvas: Arc::new(Mutex::new(Canvas::default())),
        }
    }

    /// The host saved its layout: the snapshot every canvas viewer gets now
    /// and every later joiner gets first. A viewer too slow to keep up is
    /// dropped here, not waited for.
    pub fn publish_layout(&self, text: &str) {
        let mut c = self.canvas.lock().unwrap();
        c.latest = Some(text.to_string());
        c.clients.retain(|tx| tx.try_send(text.to_string()).is_ok());
    }

    /// Binds `socket` (0600) and serves it on a thread of its own. The path
    /// is checked first, so a data dir too long for a unix socket is said so
    /// rather than failing at bind.
    pub fn serve(&self, socket: &Path) -> anyhow::Result<()> {
        super::daemon::check_socket_path(socket)?;
        // A stale file from a host that did not exit cleanly; the app's own
        // socket lock has already said this data dir is ours.
        let _ = std::fs::remove_file(socket);
        let listener = UnixListener::bind(socket)?;
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
        let hub = self.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let hub = hub.clone();
                std::thread::spawn(move || hub.handle(stream));
            }
        });
        Ok(())
    }

    fn handle(&self, stream: UnixStream) {
        let Ok(read_half) = stream.try_clone() else {
            return;
        };
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        if reader
            .by_ref()
            .take(MAX_LINE)
            .read_line(&mut line)
            .unwrap_or(0)
            == 0
        {
            return;
        }
        let mut out = stream;
        match parse_hello(line.trim_end()) {
            Err(why) => {
                let _ = writeln!(out, "ift-join {MAJOR} error {why}");
            }
            Ok(Topic::Canvas) => self.serve_canvas(out, reader),
            Ok(Topic::Session(id)) => self.serve_session(&id, out, reader),
        }
    }

    fn serve_canvas(&self, mut out: UnixStream, mut input: impl Read) {
        // Registered and the latest taken under one lock, so a snapshot saved
        // meanwhile is neither missed nor sent twice.
        let (latest, rx) = {
            let mut c = self.canvas.lock().unwrap();
            let (tx, rx) = sync_channel(CANVAS_QUEUE);
            c.clients.push(tx);
            (c.latest.clone(), rx)
        };
        if writeln!(out, "ift-join {MAJOR} ok").is_err() {
            return;
        }
        let writer_out = out.try_clone();
        let Ok(mut w) = writer_out else {
            return;
        };
        std::thread::spawn(move || {
            let send = |w: &mut UnixStream, text: &str| -> bool {
                let bytes = text.as_bytes();
                w.write_all(&(bytes.len() as u32).to_be_bytes()).is_ok()
                    && w.write_all(bytes).is_ok()
            };
            if let Some(text) = latest {
                if !send(&mut w, &text) {
                    return;
                }
            }
            while let Ok(text) = rx.recv() {
                if !send(&mut w, &text) {
                    break;
                }
            }
            let _ = w.shutdown(Shutdown::Both);
        });
        // Nothing meaningful comes back on this topic; reading until the
        // viewer closes is how a closed connection is noticed.
        let mut sink = [0u8; 256];
        while matches!(input.read(&mut sink), Ok(n) if n > 0) {}
        let _ = out.shutdown(Shutdown::Both);
    }

    fn serve_session(&self, id: &str, mut out: UnixStream, mut input: impl Read) {
        let rx = self.relay.lock().unwrap().subscribe(id);
        let Some(rx) = rx else {
            let _ = writeln!(out, "ift-join {MAJOR} error no session {id} on this host");
            return;
        };
        if writeln!(out, "ift-join {MAJOR} ok").is_err() {
            return;
        }
        let Ok(mut w) = out.try_clone() else {
            return;
        };
        std::thread::spawn(move || {
            while let Ok(frame) = rx.recv() {
                if w.write_all(&frame.encode()).is_err() {
                    break;
                }
            }
            // The relay dropped this viewer (it stopped reading, or the
            // session ended): closing is how the viewer finds out.
            let _ = w.shutdown(Shutdown::Both);
        });
        let mut frames = FrameReader::default();
        let mut buf = [0u8; 8192];
        'read: loop {
            match input.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => frames.feed(&buf[..n]),
            }
            loop {
                match frames.next() {
                    Ok(Some(Frame::Data(bytes))) => (self.write)(id, &bytes),
                    // A viewer does not resize, start or end a host session.
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(_) => break 'read,
                }
            }
        }
        let _ = out.shutdown(Shutdown::Both);
    }
}

#[derive(Debug, PartialEq)]
enum Topic {
    Canvas,
    Session(String),
}

/// `ift-join <major> canvas` or `ift-join <major> session <id>`.
fn parse_hello(line: &str) -> Result<Topic, String> {
    let mut words = line.split_whitespace();
    if words.next() != Some("ift-join") {
        return Err("this is not an ift-join connection".into());
    }
    let major: u32 = words
        .next()
        .and_then(|w| w.parse().ok())
        .ok_or_else(|| "the version is missing".to_string())?;
    if major > MAJOR {
        return Err(format!(
            "this host speaks ift-join {MAJOR} and you asked for {major}; update infiniterm on the host"
        ));
    }
    match (words.next(), words.next(), words.next()) {
        (Some("canvas"), None, None) => Ok(Topic::Canvas),
        (Some("session"), Some(id), None) if valid_id(id) => Ok(Topic::Session(id.to_string())),
        _ => Err("the topic is canvas, or session and an id".into()),
    }
}

/// A session id is the socket's filename stem: letters, digits, dash and
/// underscore, and not long. Never a path.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn hub() -> (JoinHub, Arc<Mutex<Relay>>, Arc<Mutex<Vec<(String, Vec<u8>)>>>) {
        let relay = Arc::new(Mutex::new(Relay::new()));
        let written = Arc::new(Mutex::new(Vec::new()));
        let sink = written.clone();
        let write: WriteSession =
            Arc::new(move |id, bytes| sink.lock().unwrap().push((id.to_string(), bytes.to_vec())));
        (JoinHub::new(relay.clone(), write), relay, written)
    }

    fn socket(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ij-{}-{name}.sock", std::process::id()))
    }

    fn connect(path: &Path, hello: &str) -> (UnixStream, String) {
        let mut s = UnixStream::connect(path).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        writeln!(s, "{hello}").unwrap();
        let mut line = Vec::new();
        let mut b = [0u8; 1];
        while s.read(&mut b).unwrap() == 1 && b[0] != b'\n' {
            line.push(b[0]);
        }
        (s, String::from_utf8(line).unwrap())
    }

    fn next_frame(s: &mut UnixStream, r: &mut FrameReader) -> Frame {
        loop {
            if let Some(f) = r.next().unwrap() {
                return f;
            }
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap();
            assert!(n > 0, "closed before a frame");
            r.feed(&buf[..n]);
        }
    }

    #[test]
    fn a_session_viewer_gets_hello_replay_then_live_and_its_keys_go_to_the_session() {
        let (hub, relay, written) = hub();
        {
            let mut r = relay.lock().unwrap();
            r.open("abc", Frame::Hello { pid: 7, cols: 90, rows: 30 });
            r.publish("abc", b"old");
        }
        let path = socket("session");
        hub.serve(&path).unwrap();
        let (mut s, answer) = connect(&path, "ift-join 1 session abc");
        assert_eq!(answer, "ift-join 1 ok");
        let mut r = FrameReader::default();
        assert_eq!(next_frame(&mut s, &mut r), Frame::Hello { pid: 7, cols: 90, rows: 30 });
        assert!(matches!(next_frame(&mut s, &mut r), Frame::Replay(b) if b.ends_with(b"old")));
        assert_eq!(next_frame(&mut s, &mut r), Frame::ReplayEnd);
        relay.lock().unwrap().publish("abc", b"new");
        assert_eq!(next_frame(&mut s, &mut r), Frame::Data(b"new".to_vec()));

        // Keys go through; a resize and a kill do not.
        s.write_all(&Frame::Resize { cols: 10, rows: 5 }.encode()).unwrap();
        s.write_all(&Frame::Kill.encode()).unwrap();
        s.write_all(&Frame::Data(b"ls\n".to_vec()).encode()).unwrap();
        for _ in 0..50 {
            if !written.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(*written.lock().unwrap(), vec![("abc".to_string(), b"ls\n".to_vec())]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_session_the_host_does_not_have_is_refused_with_a_sentence() {
        let (hub, _relay, _w) = hub();
        let path = socket("nosession");
        hub.serve(&path).unwrap();
        let (_s, answer) = connect(&path, "ift-join 1 session nope");
        assert_eq!(answer, "ift-join 1 error no session nope on this host");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_higher_major_is_refused_and_a_lower_one_is_served() {
        let (hub, relay, _w) = hub();
        relay
            .lock()
            .unwrap()
            .open("abc", Frame::Hello { pid: 1, cols: 80, rows: 24 });
        let path = socket("major");
        hub.serve(&path).unwrap();
        let (_s, answer) = connect(&path, "ift-join 2 canvas");
        assert!(answer.starts_with("ift-join 1 error this host speaks ift-join 1"), "{answer}");
        let (_s, answer) = connect(&path, "ift-join 0 session abc");
        assert_eq!(answer, "ift-join 1 ok");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_canvas_viewer_gets_the_latest_snapshot_then_every_later_one() {
        let (hub, _relay, _w) = hub();
        hub.publish_layout("{\"v\":1}");
        let path = socket("canvas");
        hub.serve(&path).unwrap();
        let (mut s, answer) = connect(&path, "ift-join 1 canvas");
        assert_eq!(answer, "ift-join 1 ok");
        let read_snapshot = |s: &mut UnixStream| -> String {
            let mut len = [0u8; 4];
            s.read_exact(&mut len).unwrap();
            let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
            s.read_exact(&mut body).unwrap();
            String::from_utf8(body).unwrap()
        };
        assert_eq!(read_snapshot(&mut s), "{\"v\":1}");
        hub.publish_layout("{\"v\":2}");
        assert_eq!(read_snapshot(&mut s), "{\"v\":2}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_socket_is_owner_only() {
        let (hub, _relay, _w) = hub();
        let path = socket("mode");
        hub.serve(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_path_over_the_unix_socket_limit_is_refused_before_binding() {
        let (hub, _relay, _w) = hub();
        let long = std::env::temp_dir().join("x".repeat(120)).join("j.sock");
        let err = hub.serve(&long).unwrap_err().to_string();
        assert!(err.contains("unix socket path limit"), "{err}");
    }

    #[test]
    fn the_handshake_line_names_one_topic() {
        assert_eq!(parse_hello("ift-join 1 canvas"), Ok(Topic::Canvas));
        assert_eq!(
            parse_hello("ift-join 1 session a-b_9"),
            Ok(Topic::Session("a-b_9".into()))
        );
        assert!(parse_hello("hello").is_err());
        assert!(parse_hello("ift-join 1 session ../etc").is_err());
        assert!(parse_hello("ift-join 1 session a b").is_err());
        assert!(parse_hello("ift-join x canvas").is_err());
    }
}
