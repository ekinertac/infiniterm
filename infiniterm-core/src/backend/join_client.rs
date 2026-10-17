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

use std::io::Read;
use std::sync::mpsc::Sender;
use std::time::Duration;

use super::join::{read_canvas_message, KIND_AGENTS, KIND_LAYOUT};
use super::remote::{connect_join_canvas, RemoteHost};

/// What the app is told about the host's canvas.
#[derive(Debug, PartialEq, Eq)]
pub enum CanvasEvent {
    /// A whole layout snapshot (the host's save text).
    Layout(String),
    /// The agent states, `{"<card id>": "working", ...}`.
    Agents(String),
    /// Something a person should read: connected, lost, refused.
    Status(String),
}

/// How long to wait before joining again after the connection ended or could
/// not be made. Long enough not to hammer a host that is down, short enough
/// that a host that comes back is joined within a breath.
const REJOIN_AFTER: Duration = Duration::from_secs(3);

/// Reads canvas messages until the stream ends, sending each as an event. A
/// kind this build does not know is skipped, so a newer host may add one.
/// Returns `Err` when the app stopped listening (its receiver is gone).
pub fn pump_canvas(stream: &mut impl Read, tx: &Sender<CanvasEvent>) -> Result<(), ()> {
    loop {
        match read_canvas_message(stream) {
            Ok(Some((KIND_LAYOUT, text))) => tx.send(CanvasEvent::Layout(text)).map_err(|_| ())?,
            Ok(Some((KIND_AGENTS, text))) => tx.send(CanvasEvent::Agents(text)).map_err(|_| ())?,
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return Ok(()),
        }
    }
}

/// Joins the host's canvas for as long as the app listens.
pub fn run_canvas_client(host: RemoteHost, tx: Sender<CanvasEvent>) {
    loop {
        match connect_join_canvas(&host) {
            Ok((mut stream, _stderr)) => {
                if tx
                    .send(CanvasEvent::Status(format!("joined {}", host.target)))
                    .is_err()
                {
                    return;
                }
                if pump_canvas(&mut stream, &tx).is_err() {
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
            Err(why) => {
                if tx
                    .send(CanvasEvent::Status(format!("{}: {why}", host.target)))
                    .is_err()
                {
                    return;
                }
            }
        }
        std::thread::sleep(REJOIN_AFTER);
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
        hub.publish_layout("{\"cards\":[]}");
        hub.publish_agents("{\"c\":\"waiting\"}");
        let path = std::env::temp_dir().join(format!("ij-client-{}.sock", std::process::id()));
        hub.serve(&path).unwrap();
        let mut s = UnixStream::connect(&path).unwrap();
        writeln!(s, "ift-join 1 canvas").unwrap();
        // The answer line, as connect_join_canvas would consume it.
        let mut b = [0u8; 1];
        let mut line = Vec::new();
        while std::io::Read::read(&mut s, &mut b).unwrap() == 1 && b[0] != b'\n' {
            line.push(b[0]);
        }
        assert_eq!(line, b"ift-join 1 ok");

        let (tx, rx) = channel();
        let reader = std::thread::spawn(move || {
            let _ = pump_canvas(&mut s, &tx);
        });
        assert_eq!(rx.recv().unwrap(), CanvasEvent::Layout("{\"cards\":[]}".into()));
        assert_eq!(rx.recv().unwrap(), CanvasEvent::Agents("{\"c\":\"waiting\"}".into()));
        hub.publish_layout("{\"cards\":[1]}");
        assert_eq!(rx.recv().unwrap(), CanvasEvent::Layout("{\"cards\":[1]}".into()));
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
        pump_canvas(&mut bytes.as_slice(), &tx).unwrap();
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
        assert!(pump_canvas(&mut bytes.as_slice(), &tx).is_err());
    }
}
