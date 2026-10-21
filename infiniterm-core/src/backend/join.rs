//! The host's join server (#423, `ift join`): the unix socket a viewer reaches
//! through `ssh host ift proxy join ...`. It serves two topics on one socket,
//! `<data>/j.sock`, mode 0600 (ssh is the only authentication; this is a
//! local socket that nothing else can open):
//!
//! - `canvas`: the host's layout, whole snapshots (the exact JSON the save
//!   file holds), and the agent states, which the save file leaves out on
//!   purpose (a saved state would claim a long-dead agent is working). The
//!   latest of each at once, then every later one as it changes. Each message
//!   is a 4-byte big-endian length of the text, one kind byte (`L` layout,
//!   `A` agent states, a JSON object of card id to state name) and the UTF-8
//!   text. A kind a viewer does not know is skipped by it.
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::relay::Relay;
use super::session_protocol::{Frame, FrameReader};

/// The version this host speaks. A higher one asked for is refused. 2: a
/// viewer can send its edits up (`KIND_UPDATE`); a host that speaks 1 refuses
/// a viewer that asks for 2 with a sentence, so a viewer never believes it is
/// in control of a host that would ignore it.
pub const MAJOR: u32 = 2;

/// The longest handshake line accepted: a topic is a word and a session id.
const MAX_LINE: u64 = 256;

/// How often a connection's writer checks that its reader is still there.
/// A connection closes only when every copy of its socket is closed, and the
/// writer thread holds one: so a writer parked on its queue after the viewer
/// left would keep the viewer's ssh waiting forever (found when a test never
/// ended: `shutdown` on macOS answers "not connected" once the peer has shut
/// its write side, so it cannot be relied on to wake anything).
const WRITER_POLL: Duration = Duration::from_millis(200);

/// Snapshots queued for one canvas viewer before it is dropped. Snapshots are
/// whole, so a dropped viewer loses nothing but the wait: it reconnects and
/// gets the latest.
const CANVAS_QUEUE: usize = 16;

/// A canvas message's kinds: see the module header.
pub const KIND_LAYOUT: u8 = b'L';
pub const KIND_AGENTS: u8 = b'A';
/// Host to viewer (#433): the text of the host's `settings.json` and
/// `keybindings.json`, as they are on disk. The viewer applies them in memory
/// and never writes them. An older viewer skips both kinds.
pub const KIND_SETTINGS: u8 = b'S';
pub const KIND_KEYBINDINGS: u8 = b'K';
/// Host to viewer, right after every layout: the host's revision of it, as
/// decimal text. A viewer's edit names the revision it was made on.
pub const KIND_REVISION: u8 = b'V';
/// Viewer to host: the canvas as the viewer now has it, `{"base": <revision>,
/// "layout": "<the layout text>"}`. The host applies it in place unless the
/// revision is stale (see `Model::apply_remote_layout`).
pub const KIND_UPDATE: u8 = b'U';

/// What a viewer sent up: the revision it edited, and the canvas it made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteUpdate {
    pub base: u64,
    pub layout: String,
}

/// A canvas message as bytes: the length of the text, the kind, the text.
pub fn encode_canvas_message(kind: u8, text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 5);
    out.extend_from_slice(&(text.len() as u32).to_be_bytes());
    out.push(kind);
    out.extend_from_slice(text.as_bytes());
    out
}

/// The text of a `KIND_UPDATE` message: what `encode_canvas_message` carries.
pub fn encode_update(base: u64, layout: &str) -> String {
    serde_json::json!({ "base": base, "layout": layout }).to_string()
}

fn decode_update(text: &str) -> Option<RemoteUpdate> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    Some(RemoteUpdate {
        base: v.get("base")?.as_u64()?,
        layout: v.get("layout")?.as_str()?.to_string(),
    })
}

#[derive(Default)]
struct Canvas {
    latest_layout: Option<String>,
    latest_rev: Option<u64>,
    latest_agents: Option<String>,
    latest_settings: Option<String>,
    latest_keybindings: Option<String>,
    clients: Vec<SyncSender<(u8, String)>>,
}

/// The next canvas message from the host: its kind and text. `None` at the
/// end of the stream. A viewer's reader; the proxy's bytes arrive here.
pub fn read_canvas_message(r: &mut impl Read) -> std::io::Result<Option<(u8, String)>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_CANVAS_MESSAGE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "a canvas message over the size limit",
        ));
    }
    let mut kind = [0u8; 1];
    r.read_exact(&mut kind)?;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(Some((kind[0], String::from_utf8_lossy(&body).into_owned())))
}

/// A layout is the whole canvas; 64 MiB is far past any real one and keeps a
/// corrupt length from allocating the machine.
const MAX_CANVAS_MESSAGE: usize = 64 * 1024 * 1024;

/// How a session's keys reach the host's own connection to `iftd`.
pub type WriteSession = Arc<dyn Fn(&str, &[u8]) + Send + Sync>;

/// Asks a session's program to repaint (the host's resize jog).
pub type NudgeSession = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Clone)]
pub struct JoinHub {
    relay: Arc<Mutex<Relay>>,
    write: WriteSession,
    /// Run once a viewer has joined a session, after its replay is queued.
    nudge: Option<NudgeSession>,
    canvas: Arc<Mutex<Canvas>>,
    /// Where a viewer's edits go: the app takes the receiving end once
    /// (`take_updates`) and applies them in its own poll, on its own thread.
    updates_tx: std::sync::mpsc::Sender<RemoteUpdate>,
    updates_rx: Arc<Mutex<Option<std::sync::mpsc::Receiver<RemoteUpdate>>>>,
}

impl JoinHub {
    pub fn new(relay: Arc<Mutex<Relay>>, write: WriteSession) -> JoinHub {
        let (updates_tx, updates_rx) = std::sync::mpsc::channel();
        JoinHub {
            relay,
            write,
            nudge: None,
            canvas: Arc::new(Mutex::new(Canvas::default())),
            updates_tx,
            updates_rx: Arc::new(Mutex::new(Some(updates_rx))),
        }
    }

    /// A repaint request for each session a viewer joins. The ring a viewer
    /// replays holds the program's old cursor moves; a program that draws
    /// relative to them garbles until it repaints (#423, Claude Code cards).
    pub fn with_nudge(mut self, nudge: NudgeSession) -> JoinHub {
        self.nudge = Some(nudge);
        self
    }

    /// The edits viewers send up, for the app to apply. Once: the second call
    /// gets `None`.
    pub fn take_updates(&self) -> Option<std::sync::mpsc::Receiver<RemoteUpdate>> {
        self.updates_rx.lock().unwrap().take()
    }

    /// The host saved its layout: the snapshot every canvas viewer gets now
    /// and every later joiner gets first. A viewer too slow to keep up is
    /// dropped here, not waited for.
    pub fn publish_layout(&self, text: &str, rev: u64) {
        let mut c = self.canvas.lock().unwrap();
        c.latest_layout = Some(text.to_string());
        c.latest_rev = Some(rev);
        // The layout, then its revision, in one pass under the lock: a viewer
        // that gets one gets the other, and in this order.
        c.clients.retain(|tx| {
            tx.try_send((KIND_LAYOUT, text.to_string())).is_ok()
                && tx.try_send((KIND_REVISION, rev.to_string())).is_ok()
        });
    }

    /// The agent states changed: `{"<card id>": "working", ...}`. Sent as it
    /// is; the viewer applies what it knows.
    pub fn publish_agents(&self, text: &str) {
        let mut c = self.canvas.lock().unwrap();
        c.latest_agents = Some(text.to_string());
        c.clients
            .retain(|tx| tx.try_send((KIND_AGENTS, text.to_string())).is_ok());
    }

    /// The host's `settings.json` (`KIND_SETTINGS`) or `keybindings.json`
    /// (`KIND_KEYBINDINGS`) as it is now: kept for a viewer that joins later
    /// and sent to the ones joined. Anything else is ignored.
    pub fn publish_config(&self, kind: u8, text: &str) {
        let mut c = self.canvas.lock().unwrap();
        match kind {
            KIND_SETTINGS => c.latest_settings = Some(text.to_string()),
            KIND_KEYBINDINGS => c.latest_keybindings = Some(text.to_string()),
            _ => return,
        }
        c.clients
            .retain(|tx| tx.try_send((kind, text.to_string())).is_ok());
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
            // The settings first, so the first layout is drawn in the host's
            // font and theme.
            let latest: Vec<(u8, String)> = [
                (KIND_SETTINGS, c.latest_settings.clone()),
                (KIND_KEYBINDINGS, c.latest_keybindings.clone()),
                (KIND_LAYOUT, c.latest_layout.clone()),
                (KIND_REVISION, c.latest_rev.map(|r| r.to_string())),
                (KIND_AGENTS, c.latest_agents.clone()),
            ]
            .into_iter()
            .filter_map(|(k, t)| t.map(|t| (k, t)))
            .collect();
            (latest, rx)
        };
        if writeln!(out, "ift-join {MAJOR} ok").is_err() {
            return;
        }
        let writer_out = out.try_clone();
        let Ok(mut w) = writer_out else {
            return;
        };
        let closed = Arc::new(AtomicBool::new(false));
        let writer_closed = closed.clone();
        std::thread::spawn(move || {
            let send = |w: &mut UnixStream, kind: u8, text: &str| -> bool {
                w.write_all(&encode_canvas_message(kind, text)).is_ok()
            };
            for (kind, text) in latest {
                if !send(&mut w, kind, &text) {
                    return;
                }
            }
            while !writer_closed.load(Ordering::Relaxed) {
                match rx.recv_timeout(WRITER_POLL) {
                    Ok((kind, text)) => {
                        if !send(&mut w, kind, &text) {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            let _ = w.shutdown(Shutdown::Both);
        });
        // What comes back: the viewer's edits, as whole canvases. Reading until
        // the viewer closes is also how a closed connection is noticed.
        loop {
            match read_canvas_message(&mut input) {
                Ok(Some((KIND_UPDATE, text))) => {
                    if let Some(update) = decode_update(&text) {
                        let _ = self.updates_tx.send(update);
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => break,
            }
        }
        closed.store(true, Ordering::Relaxed);
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
        // After the replay is queued, so the redraw follows it.
        if let Some(nudge) = &self.nudge {
            nudge(id);
        }
        let Ok(mut w) = out.try_clone() else {
            return;
        };
        let closed = Arc::new(AtomicBool::new(false));
        let writer_closed = closed.clone();
        std::thread::spawn(move || {
            while !writer_closed.load(Ordering::Relaxed) {
                match rx.recv_timeout(WRITER_POLL) {
                    Ok(frame) => {
                        if w.write_all(&frame.encode()).is_err() {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
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
        closed.store(true, Ordering::Relaxed);
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
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// What the fake session writer was asked to write: (session, bytes).
    type Written = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

    fn hub() -> (JoinHub, Arc<Mutex<Relay>>, Written) {
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
            r.open(
                "abc",
                Frame::Hello {
                    pid: 7,
                    cols: 90,
                    rows: 30,
                },
            );
            r.publish("abc", b"old");
        }
        let path = socket("session");
        hub.serve(&path).unwrap();
        let (mut s, answer) = connect(&path, "ift-join 1 session abc");
        assert_eq!(answer, "ift-join 2 ok");
        let mut r = FrameReader::default();
        assert_eq!(
            next_frame(&mut s, &mut r),
            Frame::Hello {
                pid: 7,
                cols: 90,
                rows: 30
            }
        );
        assert!(matches!(next_frame(&mut s, &mut r), Frame::Replay(b) if b.ends_with(b"old")));
        assert_eq!(next_frame(&mut s, &mut r), Frame::ReplayEnd);
        relay.lock().unwrap().publish("abc", b"new");
        assert_eq!(next_frame(&mut s, &mut r), Frame::Data(b"new".to_vec()));

        // Keys go through; a resize and a kill do not.
        s.write_all(&Frame::Resize { cols: 10, rows: 5 }.encode())
            .unwrap();
        s.write_all(&Frame::Kill.encode()).unwrap();
        s.write_all(&Frame::Data(b"ls\n".to_vec()).encode())
            .unwrap();
        for _ in 0..50 {
            if !written.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            *written.lock().unwrap(),
            vec![("abc".to_string(), b"ls\n".to_vec())]
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_viewer_joining_a_session_makes_the_host_ask_it_to_repaint() {
        let (hub, relay, _w) = hub();
        let asked: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = asked.clone();
        let hub = hub.with_nudge(Arc::new(move |id| {
            sink.lock().unwrap().push(id.to_string())
        }));
        relay.lock().unwrap().open(
            "abc",
            Frame::Hello {
                pid: 7,
                cols: 90,
                rows: 30,
            },
        );
        let path = socket("nudge");
        hub.serve(&path).unwrap();
        let (_missing, _) = connect(&path, "ift-join 2 session nope");
        let (_s, answer) = connect(&path, "ift-join 2 session abc");
        assert_eq!(answer, "ift-join 2 ok");
        assert_eq!(*asked.lock().unwrap(), vec!["abc".to_string()]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_session_the_host_does_not_have_is_refused_with_a_sentence() {
        let (hub, _relay, _w) = hub();
        let path = socket("nosession");
        hub.serve(&path).unwrap();
        let (_s, answer) = connect(&path, "ift-join 1 session nope");
        assert_eq!(answer, "ift-join 2 error no session nope on this host");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_higher_major_is_refused_and_a_lower_one_is_served() {
        let (hub, relay, _w) = hub();
        relay.lock().unwrap().open(
            "abc",
            Frame::Hello {
                pid: 1,
                cols: 80,
                rows: 24,
            },
        );
        let path = socket("major");
        hub.serve(&path).unwrap();
        let (_s, answer) = connect(&path, "ift-join 3 canvas");
        assert!(
            answer.starts_with("ift-join 2 error this host speaks ift-join 2"),
            "{answer}"
        );
        let (_s, answer) = connect(&path, "ift-join 0 session abc");
        assert_eq!(answer, "ift-join 2 ok");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_canvas_viewer_gets_the_latest_snapshot_then_every_later_one() {
        let (hub, _relay, _w) = hub();
        hub.publish_layout("{\"v\":1}", 7);
        let path = socket("canvas");
        hub.serve(&path).unwrap();
        let (mut s, answer) = connect(&path, "ift-join 2 canvas");
        assert_eq!(answer, "ift-join 2 ok");
        let next = |s: &mut UnixStream| read_canvas_message(s).unwrap().unwrap();
        assert_eq!(next(&mut s), (KIND_LAYOUT, "{\"v\":1}".to_string()));
        assert_eq!(next(&mut s), (KIND_REVISION, "7".to_string()));
        hub.publish_layout("{\"v\":2}", 8);
        assert_eq!(next(&mut s), (KIND_LAYOUT, "{\"v\":2}".to_string()));
        assert_eq!(next(&mut s), (KIND_REVISION, "8".to_string()));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn agent_states_ride_the_canvas_beside_the_layout_and_a_late_viewer_gets_both() {
        let (hub, _relay, _w) = hub();
        hub.publish_layout("{\"v\":1}", 1);
        hub.publish_agents("{\"c1\":\"working\"}");
        let path = socket("agents");
        hub.serve(&path).unwrap();
        let (mut s, _) = connect(&path, "ift-join 2 canvas");
        let first = read_canvas_message(&mut s).unwrap().unwrap();
        let _revision = read_canvas_message(&mut s).unwrap().unwrap();
        let second = read_canvas_message(&mut s).unwrap().unwrap();
        assert_eq!(first, (KIND_LAYOUT, "{\"v\":1}".to_string()));
        assert_eq!(second, (KIND_AGENTS, "{\"c1\":\"working\"}".to_string()));
        hub.publish_agents("{\"c1\":\"done\"}");
        assert_eq!(
            read_canvas_message(&mut s).unwrap().unwrap(),
            (KIND_AGENTS, "{\"c1\":\"done\"}".to_string())
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_hosts_settings_and_keys_reach_a_late_viewer_first_and_every_change_after() {
        let (hub, _relay, _w) = hub();
        hub.publish_layout("{\"v\":1}", 1);
        hub.publish_config(KIND_SETTINGS, "{\"terminal.fontSize\": 19}");
        hub.publish_config(KIND_KEYBINDINGS, "{}");
        // Not a config kind: ignored, not stored.
        hub.publish_config(KIND_LAYOUT, "nope");
        let path = socket("config");
        hub.serve(&path).unwrap();
        let (mut s, _) = connect(&path, "ift-join 2 canvas");
        assert_eq!(
            read_canvas_message(&mut s).unwrap().unwrap(),
            (KIND_SETTINGS, "{\"terminal.fontSize\": 19}".to_string())
        );
        assert_eq!(
            read_canvas_message(&mut s).unwrap().unwrap(),
            (KIND_KEYBINDINGS, "{}".to_string())
        );
        assert_eq!(
            read_canvas_message(&mut s).unwrap().unwrap(),
            (KIND_LAYOUT, "{\"v\":1}".to_string())
        );
        let _revision = read_canvas_message(&mut s).unwrap().unwrap();
        hub.publish_config(KIND_SETTINGS, "{\"terminal.fontSize\": 21}");
        assert_eq!(
            read_canvas_message(&mut s).unwrap().unwrap(),
            (KIND_SETTINGS, "{\"terminal.fontSize\": 21}".to_string())
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_viewers_edit_reaches_the_host_with_the_revision_it_was_made_on() {
        let (hub, _relay, _w) = hub();
        let updates = hub.take_updates().expect("the app takes the receiver once");
        assert!(hub.take_updates().is_none(), "only once");
        let path = socket("update");
        hub.serve(&path).unwrap();
        let (mut s, answer) = connect(&path, "ift-join 2 canvas");
        assert_eq!(answer, "ift-join 2 ok");
        // Noise first: a kind the host does not know, and an update that is
        // not one. Neither reaches the app.
        s.write_all(&encode_canvas_message(b'Z', "future")).unwrap();
        s.write_all(&encode_canvas_message(KIND_UPDATE, "not json"))
            .unwrap();
        s.write_all(&encode_canvas_message(
            KIND_UPDATE,
            &encode_update(7, "{\"cards\":[]}"),
        ))
        .unwrap();
        let got = updates.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(
            got,
            RemoteUpdate {
                base: 7,
                layout: "{\"cards\":[]}".into()
            }
        );
        assert!(updates.try_recv().is_err(), "the noise was dropped");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_update_survives_the_round_trip_through_its_text() {
        let layout = "{\"cards\":[{\"id\":\"a \\\"q\\\"\"}]}";
        assert_eq!(
            decode_update(&encode_update(42, layout)),
            Some(RemoteUpdate {
                base: 42,
                layout: layout.into()
            })
        );
        assert_eq!(decode_update("{}"), None);
        assert_eq!(decode_update("{\"base\": -1, \"layout\": \"x\"}"), None);
    }

    #[test]
    fn a_canvas_length_past_the_limit_is_an_error_not_an_allocation() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&u32::MAX.to_be_bytes());
        let err = read_canvas_message(&mut bytes.as_slice()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
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
