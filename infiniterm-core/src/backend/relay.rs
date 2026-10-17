//! The host's relay for viewers (#423, `ift join`): one session's output
//! fanned out to every viewer that joined it, with the ring a viewer gets on
//! attach. Pure: no socket, no gpui. The socket server in the app turns these
//! frames into bytes and a viewer's writes back into input.
//!
//! The frames are `session_protocol::Frame`, so a viewer reads the same wire
//! the app reads from `iftd`, and the pane code it already has can take it.
//! Nothing here displaces anyone: the host's own connection to `iftd` is the
//! only one the daemon sees, and every viewer is a subscriber on this side.
//!
//! A viewer that stops reading must not hold up the host's pty reads, so each
//! viewer has a bounded queue. When it fills, the viewer is dropped (its
//! receiver disconnects and the socket writer closes the connection). There
//! is no per-frame acknowledgement, on purpose; see session_protocol.rs.
//!
//! Called by the app's daemon reader (`publish`, `exited`) and by the viewer
//! socket server (`subscribe`). Related: session_protocol.rs (the frames and
//! the ring), daemon.rs (the source of the bytes), remote.rs (the viewer's
//! side of the same splice, reused later).

use std::collections::HashMap;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};

use super::session_protocol::{Frame, Ring, MAX_PAYLOAD};

/// Frames queued for one viewer before it is dropped. Large enough for a
/// full replay (the ring is cut into frames of `MAX_PAYLOAD`) and a burst of
/// output; small enough that a stalled viewer is noticed within a second of
/// a flood.
pub const VIEWER_QUEUE: usize = 512;

/// The bytes a viewer can catch up on: the ring's size per session. Matches
/// the daemon's own ring so a viewer joining late sees what the card would.
pub const RING_BYTES: usize = 256 * 1024;

/// Chunk size for replay frames. Well under `MAX_PAYLOAD`, so a replay of a
/// full ring is a handful of frames and never trips the frame size check.
const REPLAY_CHUNK: usize = 64 * 1024;

struct Session {
    ring: Ring,
    viewers: Vec<SyncSender<Frame>>,
    hello: Frame,
}

/// Every session the host is relaying, by its id (the daemon's session id).
#[derive(Default)]
pub struct Relay {
    sessions: HashMap<String, Session>,
}

impl Relay {
    pub fn new() -> Relay {
        Relay::default()
    }

    /// Starts relaying a session. `hello` is the host's pid and size for it,
    /// the first thing a viewer reads: its grid must be the host's columns
    /// and rows, since the shell wraps to those.
    pub fn open(&mut self, id: &str, hello: Frame) {
        self.sessions
            .entry(id.to_string())
            .or_insert_with(|| Session {
                ring: Ring::new(RING_BYTES),
                viewers: Vec::new(),
                hello,
            });
    }

    /// The host's size changed (its own window resized the pty). Viewers that
    /// join from now on are told this size in the hello, and the viewers
    /// already joined get a `Resize` frame; they apply it to their grid and
    /// never send one back.
    pub fn resize(&mut self, id: &str, cols: u16, rows: u16) {
        let Some(s) = self.sessions.get_mut(id) else {
            return;
        };
        if let Frame::Hello { pid, .. } = s.hello {
            s.hello = Frame::Hello { pid, cols, rows };
        }
        s.viewers
            .retain(|tx| tx.try_send(Frame::Resize { cols, rows }).is_ok());
    }

    /// Bytes the host itself was replayed on attach (the daemon's ring): kept
    /// for viewers that join later, sent to nobody now. Without this a viewer
    /// joining a session the host just adopted would see an empty screen.
    pub fn seed(&mut self, id: &str, bytes: &[u8]) {
        if let Some(s) = self.sessions.get_mut(id) {
            s.ring.push(bytes);
        }
    }

    /// Output from the child, as the host read it. Kept in the ring for late
    /// viewers and sent to every live viewer. A viewer whose queue is full is
    /// dropped here, not waited for.
    pub fn publish(&mut self, id: &str, bytes: &[u8]) {
        let Some(s) = self.sessions.get_mut(id) else {
            return;
        };
        s.ring.push(bytes);
        s.viewers
            .retain(|tx| match tx.try_send(Frame::Data(bytes.to_vec())) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
            });
    }

    /// A viewer joins. It gets the host's hello, the ring as replay frames,
    /// the end of the replay, then live output. Returns `None` for a session
    /// this host is not relaying.
    pub fn subscribe(&mut self, id: &str) -> Option<Receiver<Frame>> {
        let s = self.sessions.get_mut(id)?;
        let (tx, rx) = sync_channel(VIEWER_QUEUE);
        let replay = s.ring.replay();
        let mut frames = vec![s.hello.clone()];
        frames.extend(
            replay
                .chunks(REPLAY_CHUNK.min(MAX_PAYLOAD))
                .map(|c| Frame::Replay(c.to_vec())),
        );
        frames.push(Frame::ReplayEnd);
        for f in frames {
            // A queue this new has room for a replay of a full ring; if it
            // ever did not, the viewer is refused rather than sent half a
            // screen.
            if tx.try_send(f).is_err() {
                return None;
            }
        }
        s.viewers.push(tx);
        Some(rx)
    }

    /// The child ended. Every viewer gets `Exited` and is dropped.
    pub fn exited(&mut self, id: &str, code: i32) {
        if let Some(mut s) = self.sessions.remove(id) {
            for tx in s.viewers.drain(..) {
                let _ = tx.try_send(Frame::Exited(code));
            }
        }
    }

    /// Stops relaying a session without telling anyone (the host closed it).
    pub fn close(&mut self, id: &str) {
        self.sessions.remove(id);
    }

    /// How many viewers are joined to a session right now.
    pub fn viewers(&self, id: &str) -> usize {
        self.sessions.get(id).map_or(0, |s| s.viewers.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::session_protocol::PREAMBLE_BYTES;

    fn hello() -> Frame {
        Frame::Hello {
            pid: 42,
            cols: 80,
            rows: 24,
        }
    }

    fn drain(rx: &Receiver<Frame>) -> Vec<Frame> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    #[test]
    fn a_viewer_gets_hello_replay_end_then_live_output() {
        let mut r = Relay::new();
        r.open("s", hello());
        r.publish("s", b"before");
        let rx = r.subscribe("s").expect("relayed");
        r.publish("s", b"after");
        assert_eq!(
            drain(&rx),
            vec![
                hello(),
                Frame::Replay(b"\x1b[0mbefore".to_vec()),
                Frame::ReplayEnd,
                Frame::Data(b"after".to_vec()),
            ]
        );
    }

    #[test]
    fn every_viewer_gets_the_same_live_output() {
        let mut r = Relay::new();
        r.open("s", hello());
        let a = r.subscribe("s").unwrap();
        let b = r.subscribe("s").unwrap();
        r.publish("s", b"x");
        assert!(drain(&a).ends_with(&[Frame::Data(b"x".to_vec())]));
        assert!(drain(&b).ends_with(&[Frame::Data(b"x".to_vec())]));
        assert_eq!(r.viewers("s"), 2);
    }

    #[test]
    fn a_viewer_that_stops_reading_is_dropped_and_the_others_keep_going() {
        let mut r = Relay::new();
        r.open("s", hello());
        let stalled = r.subscribe("s").unwrap();
        let live = r.subscribe("s").unwrap();
        // Read the live one as we go; never read the stalled one.
        let mut seen = 0;
        for _ in 0..(VIEWER_QUEUE + 10) {
            r.publish("s", b"y");
            seen += drain(&live).len();
        }
        assert_eq!(r.viewers("s"), 1, "the stalled viewer is gone");
        assert!(seen >= VIEWER_QUEUE, "the live one kept receiving");
        drop(stalled);
    }

    #[test]
    fn a_full_ring_replays_in_chunks_under_the_frame_limit() {
        let mut r = Relay::new();
        r.open("s", hello());
        let big = vec![b'z'; RING_BYTES];
        r.publish("s", &big);
        let rx = r.subscribe("s").unwrap();
        let frames = drain(&rx);
        let replayed: usize = frames
            .iter()
            .filter_map(|f| match f {
                Frame::Replay(b) => Some(b.len()),
                _ => None,
            })
            .sum();
        // The ring adds its reset prefix, and the preamble once it has
        // trimmed (session_protocol.rs `Ring::replay`).
        assert!(replayed > 0 && replayed <= RING_BYTES + PREAMBLE_BYTES + 4);
        assert!(frames.iter().all(|f| match f {
            Frame::Replay(b) => b.len() <= REPLAY_CHUNK,
            _ => true,
        }));
    }

    #[test]
    fn exit_tells_every_viewer_and_forgets_the_session() {
        let mut r = Relay::new();
        r.open("s", hello());
        let rx = r.subscribe("s").unwrap();
        r.exited("s", 3);
        assert!(drain(&rx).ends_with(&[Frame::Exited(3)]));
        assert!(r.subscribe("s").is_none());
    }

    #[test]
    fn a_session_nobody_relays_is_refused_and_ignored() {
        let mut r = Relay::new();
        assert!(r.subscribe("nope").is_none());
        r.publish("nope", b"ignored");
        assert_eq!(r.viewers("nope"), 0);
    }

    #[test]
    fn a_viewer_joining_after_a_resize_is_told_the_new_size() {
        let mut r = Relay::new();
        r.open("s", hello());
        r.resize("s", 120, 40);
        let rx = r.subscribe("s").unwrap();
        assert_eq!(
            drain(&rx)[0],
            Frame::Hello {
                pid: 42,
                cols: 120,
                rows: 40
            }
        );
    }

    #[test]
    fn viewers_already_joined_are_told_when_the_host_resizes() {
        let mut r = Relay::new();
        r.open("s", hello());
        let rx = r.subscribe("s").unwrap();
        drain(&rx);
        r.resize("s", 100, 30);
        assert_eq!(
            drain(&rx),
            vec![Frame::Resize {
                cols: 100,
                rows: 30
            }]
        );
    }

    #[test]
    fn seeded_bytes_reach_a_later_viewer_but_no_live_one() {
        let mut r = Relay::new();
        r.open("s", hello());
        let early = r.subscribe("s").unwrap();
        drain(&early);
        r.seed("s", b"from the daemon ring");
        assert!(drain(&early).is_empty(), "seed is not live output");
        let late = r.subscribe("s").unwrap();
        let frames = drain(&late);
        assert!(frames
            .iter()
            .any(|f| matches!(f, Frame::Replay(b) if b.ends_with(b"from the daemon ring"))));
    }
}
