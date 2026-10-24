//! The viewer's end of the canvas stream (#423, `ift join`): a thread that
//! joins the host's canvas through `ssh host ift proxy join canvas`, turns the
//! messages into events for the app, and joins again when the connection ends.
//!
//! Pure around one seam: `pump_canvas` reads a stream and sends events, so it
//! is tested with a real `JoinHub` and no ssh. `run_canvas_client` is the
//! thread the app starts; it ends when the app drops its receiver.
//!
//! Called by the viewer's startup in the ui crate. Related: join.rs (the
//! host's end and the message framing), remote.rs (`connect_join_canvas`).

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::join::{
    encode_canvas_message, encode_open, encode_update, read_canvas_message, OpenRequest,
    RemoteUpdate, KIND_AGENTS, KIND_KEYBINDINGS, KIND_LAYOUT, KIND_NOTICE, KIND_OPEN,
    KIND_REVISION, KIND_SETTINGS, KIND_UPDATE,
};
use super::remote::{connect_join_canvas, JoinFailure, RemoteHost};

/// What the app is told about the host's canvas.
#[derive(Debug, PartialEq, Eq)]
pub enum CanvasEvent {
    /// A whole layout snapshot (the host's save text).
    Layout(String),
    /// The host's revision of the layout just before it: an edit sent up
    /// names it, and the host drops an edit made on an older one.
    Revision(u64),
    /// The agent states, `{"<card id>": "working", ...}`.
    Agents(String),
    /// The text of the host's `settings.json` (#433).
    Settings(String),
    /// The text of the host's `keybindings.json` (#433).
    Keybindings(String),
    /// The canvas connection is up (again): the host is reachable, so a card
    /// that lost its own connection tries now (#442).
    Connected,
    /// Something a person should read: connected, lost, refused.
    Status(String),
    /// The host cannot be joined and waiting will not change that (an `ift`
    /// too old, a version refused, no app there, a login refused). Said once;
    /// the client has stopped.
    Fatal(String),
}

/// How long to wait before joining again after the connection ended or could
/// not be made. Long enough not to hammer a host that is down, short enough
/// that a host that comes back is joined within a breath.
const REJOIN_AFTER: Duration = Duration::from_secs(3);

/// Why `pump_canvas` stopped.
#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
    /// The host closed the stream or it broke: join again.
    Stream,
    /// The app stopped listening (its receiver is gone): stop.
    AppGone,
}

/// Reads canvas messages until the stream ends, sending each as an event. A
/// kind this build does not know is skipped, so a newer host may add one.
pub fn pump_canvas(stream: &mut impl Read, tx: &Sender<CanvasEvent>) -> Ended {
    loop {
        let event = match read_canvas_message(stream) {
            Ok(Some((KIND_LAYOUT, text))) => CanvasEvent::Layout(text),
            Ok(Some((KIND_REVISION, text))) => match text.trim().parse() {
                Ok(rev) => CanvasEvent::Revision(rev),
                Err(_) => continue,
            },
            Ok(Some((KIND_AGENTS, text))) => CanvasEvent::Agents(text),
            Ok(Some((KIND_SETTINGS, text))) => CanvasEvent::Settings(text),
            Ok(Some((KIND_KEYBINDINGS, text))) => CanvasEvent::Keybindings(text),
            // The host's answer to a click on a path that is not there: a
            // sentence, shown as any status is.
            Ok(Some((KIND_NOTICE, text))) => CanvasEvent::Status(text),
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => return Ended::Stream,
        };
        if tx.send(event).is_err() {
            return Ended::AppGone;
        }
    }
}

/// What the app sends up to the host, in the order it was made: an edit of
/// the canvas, or a click on a path in a card's terminal (#555).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Uplink {
    Edit(RemoteUpdate),
    Open(OpenRequest),
}

/// How often the uplink writer looks for an edit to send, and for the end of
/// its connection.
const UPLINK_POLL: Duration = Duration::from_millis(100);

/// Sends the viewer's edits up one connection until it ends. The receiver is
/// shared across reconnects, so an edit made while the connection was down is
/// sent on the next one (and dropped by the host if it is stale by then).
fn run_uplink(mut stream: impl Write, edits: Arc<Mutex<Receiver<Uplink>>>, ended: Arc<AtomicBool>) {
    while !ended.load(Ordering::Relaxed) {
        let next = edits.lock().unwrap().recv_timeout(UPLINK_POLL);
        match next {
            Ok(up) => {
                let message = match up {
                    Uplink::Edit(edit) => {
                        encode_canvas_message(KIND_UPDATE, &encode_update(edit.base, &edit.layout))
                    }
                    Uplink::Open(open) => encode_canvas_message(KIND_OPEN, &encode_open(&open)),
                };
                if stream
                    .write_all(&message)
                    .and_then(|_| stream.flush())
                    .is_err()
                {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Joins the host's canvas for as long as the app listens, and sends the
/// edits the app makes (`edits`) up to the host.
pub fn run_canvas_client(host: RemoteHost, tx: Sender<CanvasEvent>, edits: Receiver<Uplink>) {
    run_canvas_client_with(host, tx, edits, rejoin_wait)
}

/// The wait before the next try after `failures` tries in a row failed:
/// `REJOIN_AFTER`, doubling, up to `REJOIN_LONGEST`. A host that is down is
/// asked every half minute, not every three seconds, all night.
fn rejoin_wait(failures: u32) -> Duration {
    (REJOIN_AFTER * 2u32.pow(failures.min(4))).min(REJOIN_LONGEST)
}

const REJOIN_LONGEST: Duration = Duration::from_secs(30);

/// The longest a status line may be: the bar also holds the card's name, and
/// the full ssh sentence ran over it (found on the real pair, #442).
const STATUS_MAX: usize = 48;

/// A failed join as one short line for the status bar. The known causes get
/// a plain phrase; anything else is cut. Pure, so it is tested.
pub fn short_status(target: &str, why: &str) -> String {
    let lower = why.to_lowercase();
    let phrase = if lower.contains("no running infiniterm") {
        "app not running; waiting"
    } else if CONNECTION_WORDS_SHORT.iter().any(|w| lower.contains(w)) {
        "not reachable; retrying"
    } else if lower.contains("permission denied") {
        "login refused; retrying"
    } else {
        return cut(&format!("{target}: {}", why.trim()), STATUS_MAX);
    };
    cut(&format!("{target}: {phrase}"), STATUS_MAX)
}

/// What ssh and the OS say when the host cannot be reached.
const CONNECTION_WORDS_SHORT: [&str; 8] = [
    "timed out",
    "refused",
    "unreachable",
    "no route",
    "could not resolve",
    "host is down",
    "connection reset",
    "connection closed",
];

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// `run_canvas_client` with the wait between tries given, for a test.
///
/// Once the canvas has been joined, a refusal is never final: the host's app
/// quitting (an update, a restart) answers "no running infiniterm", and the
/// viewer must be there when it comes back. Before the first join the same
/// answer is final, because `ift join` has said it once and a window that
/// never connected has nothing to show.
fn run_canvas_client_with(
    host: RemoteHost,
    tx: Sender<CanvasEvent>,
    edits: Receiver<Uplink>,
    wait: impl Fn(u32) -> Duration,
) {
    let edits = Arc::new(Mutex::new(edits));
    let mut joined_once = false;
    let mut failures = 0u32;
    loop {
        match connect_join_canvas(&host) {
            Ok((mut stream, _stderr)) => {
                joined_once = true;
                failures = 0;
                if tx
                    .send(CanvasEvent::Status(format!("joined {}", host.target)))
                    .is_err()
                    || tx.send(CanvasEvent::Connected).is_err()
                {
                    return;
                }
                let ended = Arc::new(AtomicBool::new(false));
                if let Ok(up) = stream.try_clone() {
                    let (edits, ended) = (edits.clone(), ended.clone());
                    std::thread::spawn(move || run_uplink(up, edits, ended));
                }
                let result = pump_canvas(&mut stream, &tx);
                ended.store(true, Ordering::Relaxed);
                if result == Ended::AppGone {
                    return;
                }
                if tx
                    .send(CanvasEvent::Status(format!(
                        "lost {}; joining again",
                        host.target
                    )))
                    .is_err()
                {
                    return;
                }
            }
            Err(JoinFailure::Fatal(why)) if !joined_once => {
                let _ = tx.send(CanvasEvent::Fatal(why));
                return;
            }
            Err(JoinFailure::Fatal(why) | JoinFailure::Retry(why)) => {
                failures += 1;
                // The long text is for the log; the status bar gets a line
                // that fits beside the card's name.
                eprintln!("[infiniterm] {}: {why}", host.target);
                if tx
                    .send(CanvasEvent::Status(short_status(&host.target, &why)))
                    .is_err()
                {
                    return;
                }
            }
        }
        std::thread::sleep(wait(failures));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::join::{JoinHub, WriteSession};
    use crate::backend::relay::Relay;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex};

    fn hub() -> JoinHub {
        let write: WriteSession = Arc::new(|_, _| {});
        JoinHub::new(Arc::new(Mutex::new(Relay::new())), write)
    }

    #[test]
    fn layouts_and_agent_states_become_events_and_unknown_kinds_are_skipped() {
        let hub = hub();
        hub.publish_layout("{\"cards\":[]}", 3);
        hub.publish_agents("{\"c\":\"waiting\"}");
        let path = std::env::temp_dir().join(format!("ij-client-{}.sock", std::process::id()));
        hub.serve(&path).unwrap();
        let mut s = UnixStream::connect(&path).unwrap();
        writeln!(s, "ift-join 2 canvas").unwrap();
        // The answer line, as connect_join_canvas would consume it.
        let mut b = [0u8; 1];
        let mut line = Vec::new();
        while std::io::Read::read(&mut s, &mut b).unwrap() == 1 && b[0] != b'\n' {
            line.push(b[0]);
        }
        assert_eq!(line, b"ift-join 2 ok");

        let (tx, rx) = channel();
        let reader = std::thread::spawn(move || {
            let _ = pump_canvas(&mut s, &tx);
        });
        assert_eq!(
            rx.recv().unwrap(),
            CanvasEvent::Layout("{\"cards\":[]}".into())
        );
        assert_eq!(rx.recv().unwrap(), CanvasEvent::Revision(3));
        assert_eq!(
            rx.recv().unwrap(),
            CanvasEvent::Agents("{\"c\":\"waiting\"}".into())
        );
        hub.publish_layout("{\"cards\":[1]}", 4);
        assert_eq!(
            rx.recv().unwrap(),
            CanvasEvent::Layout("{\"cards\":[1]}".into())
        );
        assert_eq!(rx.recv().unwrap(), CanvasEvent::Revision(4));
        drop(rx);
        // The reader ends when the app stops listening or the host closes.
        let _ = std::fs::remove_file(&path);
        drop(reader);
    }

    #[test]
    fn a_message_of_a_kind_this_build_does_not_know_is_skipped() {
        let mut bytes = Vec::new();
        for (kind, body) in [(b'Z', "future"), (KIND_LAYOUT, "now")] {
            bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
            bytes.push(kind);
            bytes.extend_from_slice(body.as_bytes());
        }
        let (tx, rx) = channel();
        assert_eq!(pump_canvas(&mut bytes.as_slice(), &tx), Ended::Stream);
        assert_eq!(rx.try_recv().unwrap(), CanvasEvent::Layout("now".into()));
        assert!(rx.try_recv().is_err(), "nothing else was sent");
    }

    #[test]
    fn the_pump_reports_a_closed_receiver() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.push(KIND_LAYOUT);
        bytes.push(b'x');
        let (tx, rx) = channel();
        drop(rx);
        assert_eq!(pump_canvas(&mut bytes.as_slice(), &tx), Ended::AppGone);
    }

    #[test]
    fn a_revision_follows_its_layout_as_an_event_and_garbage_is_skipped() {
        let mut bytes = Vec::new();
        for (kind, body) in [
            (KIND_LAYOUT, "{}"),
            (KIND_REVISION, "12"),
            (KIND_REVISION, "not a number"),
        ] {
            bytes.extend(encode_canvas_message(kind, body));
        }
        let (tx, rx) = channel();
        assert_eq!(pump_canvas(&mut bytes.as_slice(), &tx), Ended::Stream);
        assert_eq!(rx.try_recv().unwrap(), CanvasEvent::Layout("{}".into()));
        assert_eq!(rx.try_recv().unwrap(), CanvasEvent::Revision(12));
        assert!(rx.try_recv().is_err());
    }

    /// A stand-in ssh that runs `case $n` arms, `n` being the try's number.
    fn counting_ssh(name: &str, arms: &str) -> RemoteHost {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("jc-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("ssh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nn=$(cat '{c}' 2>/dev/null || echo 0); n=$((n+1)); echo $n > '{c}'\ncase $n in\n{arms}\nesac\n",
                c = dir.join("count").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut host = RemoteHost::new("h");
        host.ssh = script.display().to_string();
        host
    }

    fn events_until(
        rx: &Receiver<CanvasEvent>,
        mut stop: impl FnMut(&CanvasEvent) -> bool,
    ) -> Vec<CanvasEvent> {
        let mut seen = Vec::new();
        while let Ok(e) = rx.recv_timeout(Duration::from_secs(10)) {
            let done = stop(&e);
            seen.push(e);
            if done {
                break;
            }
        }
        seen
    }

    #[test]
    fn a_host_app_that_quits_and_comes_back_is_joined_again() {
        let host = counting_ssh(
            "restart",
            concat!(
                "1) printf 'ift-join 2 ok\\n';;\n",
                "2) echo 'ift: no running infiniterm to join on this host (x): No such file or directory' >&2; exit 1;;\n",
                "*) printf 'ift-join 2 ok\\n'; sleep 3;;\n"
            ),
        );
        let (tx, rx) = channel();
        let (_edits_tx, edits) = channel();
        std::thread::spawn(move || {
            run_canvas_client_with(host, tx, edits, |_| Duration::from_millis(20))
        });
        let mut connected = 0;
        let seen = events_until(&rx, |e| {
            if *e == CanvasEvent::Connected {
                connected += 1;
            }
            connected == 2
        });
        assert_eq!(connected, 2, "joined, lost, joined again: {seen:?}");
        assert!(
            seen.iter()
                .any(|e| matches!(e, CanvasEvent::Status(s) if s.contains("app not running"))),
            "the viewer says the host's app is not running: {seen:?}"
        );
        assert!(
            !seen.iter().any(|e| matches!(e, CanvasEvent::Fatal(_))),
            "{seen:?}"
        );
    }

    #[test]
    fn a_host_that_refuses_before_the_first_join_is_final() {
        let host = counting_ssh(
            "refused",
            "*) echo 'ift: no running infiniterm to join on this host (x): No such file or directory' >&2; exit 1;;",
        );
        let (tx, rx) = channel();
        let (_edits_tx, edits) = channel();
        let worker = std::thread::spawn(move || {
            run_canvas_client_with(host, tx, edits, |_| Duration::from_millis(20))
        });
        let seen = events_until(&rx, |e| matches!(e, CanvasEvent::Fatal(_)));
        assert!(
            matches!(seen.last(), Some(CanvasEvent::Fatal(_))),
            "{seen:?}"
        );
        worker
            .join()
            .expect("the client stops after a final answer");
    }

    #[test]
    fn a_failed_join_is_one_short_line_for_the_status_bar() {
        let long = "ift: no running infiniterm to join on this host (/Users/ekinertac/Library/Application Support/dev.ekinertac.infiniterm/j.sock): Connection refused (os error 61)";
        // Whole, never cut, for a host name up to 20 characters.
        assert_eq!(
            short_status("ekin-mini", long),
            "ekin-mini: app not running; waiting"
        );
        let twenty = "a".repeat(20);
        for why in [
            long,
            "ssh: connect to host h port 22: Operation timed out",
            "Permission denied (publickey).",
        ] {
            let line = short_status(&twenty, why);
            assert!(!line.ends_with('…'), "{line}");
            assert!(line.chars().count() <= STATUS_MAX, "{line}");
        }
        assert_eq!(
            short_status(
                "mini",
                "ssh: connect to host mini port 22: Operation timed out"
            ),
            "mini: not reachable; retrying"
        );
        assert_eq!(
            short_status("mini", "Permission denied (publickey)."),
            "mini: login refused; retrying"
        );
        let odd = short_status("mini", &"x".repeat(200));
        assert_eq!(odd.chars().count(), STATUS_MAX);
        assert!(odd.ends_with('…'));
    }

    #[test]
    fn a_host_that_stays_down_is_asked_less_and_less_often() {
        assert_eq!(rejoin_wait(0), REJOIN_AFTER);
        assert!(rejoin_wait(2) > rejoin_wait(1));
        assert_eq!(rejoin_wait(50), REJOIN_LONGEST);
    }

    #[test]
    fn the_hosts_settings_and_keys_arrive_as_events() {
        let mut bytes = Vec::new();
        bytes.extend(encode_canvas_message(KIND_SETTINGS, "{\"a\":1}"));
        bytes.extend(encode_canvas_message(KIND_KEYBINDINGS, "{}"));
        let (tx, rx) = channel();
        pump_canvas(&mut bytes.as_slice(), &tx);
        assert_eq!(
            rx.try_recv().unwrap(),
            CanvasEvent::Settings("{\"a\":1}".into())
        );
        assert_eq!(
            rx.try_recv().unwrap(),
            CanvasEvent::Keybindings("{}".into())
        );
    }

    #[test]
    fn the_uplink_writes_each_edit_as_an_update_message_the_host_reads() {
        use crate::backend::join::read_canvas_message;
        use std::sync::mpsc::channel as edit_channel;
        let (tx, rx) = edit_channel();
        let ended = Arc::new(AtomicBool::new(false));
        let (near, mut far) = std::os::unix::net::UnixStream::pair().unwrap();
        let (edits, stop) = (Arc::new(Mutex::new(rx)), ended.clone());
        let writer = std::thread::spawn(move || run_uplink(near, edits, stop));
        tx.send(Uplink::Edit(RemoteUpdate {
            base: 5,
            layout: "{\"a\":1}".into(),
        }))
        .unwrap();
        tx.send(Uplink::Open(OpenRequest {
            card: "c1".into(),
            cwd: "/srv".into(),
            path: "src/a.rs".into(),
            line: Some(42),
        }))
        .unwrap();
        far.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let (kind, text) = read_canvas_message(&mut far).unwrap().unwrap();
        assert_eq!(kind, KIND_UPDATE);
        assert_eq!(text, encode_update(5, "{\"a\":1}"));
        // In the order they were made, on the same connection.
        let (kind, text) = read_canvas_message(&mut far).unwrap().unwrap();
        assert_eq!(kind, KIND_OPEN);
        assert_eq!(
            crate::backend::join::decode_open(&text).unwrap().line,
            Some(42)
        );
        ended.store(true, Ordering::Relaxed);
        writer.join().unwrap();
    }
}
