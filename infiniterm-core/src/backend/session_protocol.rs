//! The wire between the app and `iftd`, and the ring of output bytes the
//! daemon holds for a card that nothing is attached to. Pure: bytes in,
//! frames out, so all of it is tested without a daemon anywhere.
//!
//! Length-prefixed BINARY frames, deliberately. tmux control mode is text
//! with three-digit octal escapes, and getting that wrong cost a day: a
//! byte that looks like a line ending, an escape that is not one, a reply
//! block that belongs to a command nobody queued. Here a frame is a kind, a
//! length and that many bytes, so there is no escaping to get wrong and no
//! framing to lose synchronisation with.
//!
//! There is no acknowledgement frame and there will not be one. When the
//! app stops reading, the daemon's write blocks, so it stops reading the
//! pty, so the child blocks on the kernel's pty buffer. That chain IS the
//! backpressure. tmux needed `refresh-client -A` for the same job and its
//! `pause` verb DISCARDS the output it holds back, which lost bytes for an
//! evening.
//!
//! Called by `backend/daemon.rs` (the app's end) and `infiniterm-session`
//! (the daemon's end). Related: `tmux_protocol.rs`, the same job for the
//! backend this replaced, and
//! docs/superpowers/specs/2026-09-17-session-daemon-design.md for why.

/// The largest payload a single frame may carry. Anything above this is a
/// corrupt stream rather than a big message: the daemon chunks a replay to
/// fit, and no other frame comes close.
pub const MAX_PAYLOAD: usize = 1024 * 1024;

/// Kind plus a big-endian u32 length.
const HEADER: usize = 5;

pub const KIND_DATA: u8 = 1;
pub const KIND_RESIZE: u8 = 2;
const KIND_KILL: u8 = 3;
const KIND_HELLO: u8 = 4;
const KIND_REPLAY: u8 = 5;
const KIND_REPLAY_END: u8 = 6;
const KIND_EXITED: u8 = 7;

/// Everything either end can say. App to daemon: `Data`, `Resize`, `Kill`.
/// Daemon to app: `Hello`, `Replay`, `ReplayEnd`, `Data`, `Exited`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    /// Keystrokes going down, or the child's output coming up.
    Data(Vec<u8>),
    Resize {
        cols: u16,
        rows: u16,
    },
    /// End the child and go away. Closing the socket instead DETACHES.
    Kill,
    /// First frame on every attach. The pid is the child's, for the label
    /// and the remote-session poller.
    Hello {
        pid: u32,
        cols: u16,
        rows: u16,
    },
    /// Part of the ring: what the card missed. Sent in as many frames as it
    /// takes, always before `ReplayEnd`.
    Replay(Vec<u8>),
    /// Everything after this is live.
    ReplayEnd,
    Exited(i32),
}

/// A stream that is not ours, or is no longer in step with us. Every one of
/// these is terminal: the caller drops the connection rather than trying to
/// resynchronise, because there is no resynchronisation point to find.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtoError {
    /// A kind byte no version of this protocol ever wrote.
    Kind(u8),
    TooLarge(usize),
    /// A fixed-width payload that was not that width.
    Malformed(&'static str),
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtoError::Kind(k) => write!(f, "frame kind {k} is not one of ours"),
            ProtoError::TooLarge(n) => write!(f, "frame claims {n} bytes"),
            ProtoError::Malformed(what) => write!(f, "malformed {what} frame"),
        }
    }
}

impl Frame {
    fn kind(&self) -> u8 {
        match self {
            Frame::Data(_) => KIND_DATA,
            Frame::Resize { .. } => KIND_RESIZE,
            Frame::Kill => KIND_KILL,
            Frame::Hello { .. } => KIND_HELLO,
            Frame::Replay(_) => KIND_REPLAY,
            Frame::ReplayEnd => KIND_REPLAY_END,
            Frame::Exited(_) => KIND_EXITED,
        }
    }

    fn payload(&self) -> Vec<u8> {
        match self {
            Frame::Data(b) | Frame::Replay(b) => b.clone(),
            Frame::Resize { cols, rows } => {
                let mut v = cols.to_be_bytes().to_vec();
                v.extend(rows.to_be_bytes());
                v
            }
            Frame::Hello { pid, cols, rows } => {
                let mut v = pid.to_be_bytes().to_vec();
                v.extend(cols.to_be_bytes());
                v.extend(rows.to_be_bytes());
                v
            }
            Frame::Exited(code) => code.to_be_bytes().to_vec(),
            Frame::Kill | Frame::ReplayEnd => Vec::new(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let payload = self.payload();
        let mut out = Vec::with_capacity(HEADER + payload.len());
        out.push(self.kind());
        out.extend((payload.len() as u32).to_be_bytes());
        out.extend(payload);
        out
    }

    fn decode(kind: u8, payload: &[u8]) -> Result<Frame, ProtoError> {
        let fixed = |want: usize, what: &'static str| -> Result<(), ProtoError> {
            if payload.len() == want {
                Ok(())
            } else {
                Err(ProtoError::Malformed(what))
            }
        };
        let u16_at = |i: usize| u16::from_be_bytes([payload[i], payload[i + 1]]);
        match kind {
            KIND_DATA => Ok(Frame::Data(payload.to_vec())),
            KIND_REPLAY => Ok(Frame::Replay(payload.to_vec())),
            KIND_KILL => {
                fixed(0, "kill")?;
                Ok(Frame::Kill)
            }
            KIND_REPLAY_END => {
                fixed(0, "replay-end")?;
                Ok(Frame::ReplayEnd)
            }
            KIND_RESIZE => {
                fixed(4, "resize")?;
                Ok(Frame::Resize {
                    cols: u16_at(0),
                    rows: u16_at(2),
                })
            }
            KIND_HELLO => {
                fixed(8, "hello")?;
                Ok(Frame::Hello {
                    pid: u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]),
                    cols: u16_at(4),
                    rows: u16_at(6),
                })
            }
            KIND_EXITED => {
                fixed(4, "exited")?;
                Ok(Frame::Exited(i32::from_be_bytes([
                    payload[0], payload[1], payload[2], payload[3],
                ])))
            }
            other => Err(ProtoError::Kind(other)),
        }
    }
}

/// Once the consumed prefix passes this, the buffer is compacted. Without
/// it a long-lived connection keeps every byte it ever read; draining on
/// every frame instead would memmove the remainder per frame, which a pane
/// producing thousands of small frames a second would pay for.
const COMPACT_AT: usize = 64 * 1024;

/// Bytes arrive in whatever sizes the socket felt like. This holds the
/// leftovers between reads, so a frame split across two reads is rejoined
/// and several frames in one read all come back.
#[derive(Debug, Default)]
pub struct FrameReader {
    buf: Vec<u8>,
    /// How much of `buf` has been handed out already.
    start: usize,
}

impl FrameReader {
    pub fn feed(&mut self, bytes: &[u8]) {
        if self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        } else if self.start >= COMPACT_AT {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole frame, `None` while one is still arriving. An error
    /// means the stream is not ours and the connection should be dropped.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Frame>, ProtoError> {
        let rest = &self.buf[self.start..];
        if rest.len() < HEADER {
            return Ok(None);
        }
        let kind = rest[0];
        let len = u32::from_be_bytes([rest[1], rest[2], rest[3], rest[4]]) as usize;
        if len > MAX_PAYLOAD {
            return Err(ProtoError::TooLarge(len));
        }
        if rest.len() < HEADER + len {
            return Ok(None);
        }
        let frame = Frame::decode(kind, &rest[HEADER..HEADER + len])?;
        self.start += HEADER + len;
        Ok(Some(frame))
    }
}

/// How far to look for a line boundary after a trim.
///
/// Unbounded this is O(buffer) on every push once full, which a pane
/// printing binary (no newline anywhere) would pay on all 4 MiB forever.
/// Past this we accept a raw cut: the replayed first line is then partial,
/// which costs one line of history and nothing else.
const LINE_SCAN: usize = 64 * 1024;

/// The last `cap` bytes of a pane's output, exactly as the pty produced
/// them. Not a grid: replaying the bytes into a fresh emulator gives back
/// the screen that made them, because it is the same parser that drew it
/// the first time. tmux's `capture-pane` could only hand back a flattened,
/// padded copy of ITS grid, measured with ITS character widths, and a line
/// it padded differently wrapped in a place ours had not.
pub struct Ring {
    buf: std::collections::VecDeque<u8>,
    cap: usize,
}

impl Ring {
    pub fn new(cap: usize) -> Ring {
        Ring {
            buf: std::collections::VecDeque::new(),
            cap,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend(bytes);
        if self.buf.len() <= self.cap {
            return;
        }
        // The newest output is what a program just drew; the oldest is what
        // scrolled away. Drop from the front.
        self.buf.drain(..self.buf.len() - self.cap);
        // Then forward to just past a line ending, so a replay never begins
        // halfway through a line somebody will read.
        let boundary = self
            .buf
            .iter()
            .take(LINE_SCAN)
            .position(|&b| b == b'\n')
            .map(|i| i + 1);
        if let Some(n) = boundary {
            self.buf.drain(..n);
        }
    }

    /// What an attaching client is given before anything live.
    ///
    /// Prefixed with a graphic reset, because a front that was cut may have
    /// been wearing colours set by a sequence that is no longer in the ring.
    /// Nothing is prefixed to an empty ring: a card that never ran anything
    /// must not be handed a stray escape sequence.
    pub fn replay(&self) -> Vec<u8> {
        if self.buf.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(self.buf.len() + 4);
        out.extend(b"\x1b[0m");
        out.extend(self.buf.iter().copied());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_survives_a_round_trip() {
        for f in [
            Frame::Data(b"ls -la\n".to_vec()),
            Frame::Resize {
                cols: 120,
                rows: 40,
            },
            Frame::Kill,
            Frame::Hello {
                pid: 4321,
                cols: 80,
                rows: 24,
            },
            Frame::Replay(vec![0, 27, 255]),
            Frame::ReplayEnd,
            Frame::Exited(-1),
        ] {
            let mut r = FrameReader::default();
            r.feed(&f.encode());
            assert_eq!(r.next().unwrap(), Some(f.clone()), "{f:?}");
            assert_eq!(r.next().unwrap(), None, "nothing left after {f:?}");
        }
    }

    // A socket read boundary lands wherever it lands; a frame split across
    // two reads must not be lost or misread.
    #[test]
    fn a_frame_split_across_reads_is_rejoined() {
        let bytes = Frame::Data(b"hello".to_vec()).encode();
        let mut r = FrameReader::default();
        for chunk in bytes.chunks(1) {
            r.feed(chunk);
            // Every prefix but the last is still not a frame.
        }
        assert_eq!(r.next().unwrap(), Some(Frame::Data(b"hello".to_vec())));
    }

    #[test]
    fn a_partial_frame_is_not_a_frame_yet() {
        let bytes = Frame::Data(b"hello".to_vec()).encode();
        let mut r = FrameReader::default();
        r.feed(&bytes[..4]);
        assert_eq!(r.next().unwrap(), None);
    }

    // Two frames can arrive in one read, and both must come back.
    #[test]
    fn several_frames_in_one_read_all_arrive() {
        let mut bytes = Frame::Data(b"a".to_vec()).encode();
        bytes.extend(Frame::ReplayEnd.encode());
        bytes.extend(Frame::Exited(3).encode());
        let mut r = FrameReader::default();
        r.feed(&bytes);
        assert_eq!(r.next().unwrap(), Some(Frame::Data(b"a".to_vec())));
        assert_eq!(r.next().unwrap(), Some(Frame::ReplayEnd));
        assert_eq!(r.next().unwrap(), Some(Frame::Exited(3)));
        assert_eq!(r.next().unwrap(), None);
    }

    // A length nothing could have meant means the stream is not ours.
    #[test]
    fn an_oversized_length_is_an_error() {
        let mut r = FrameReader::default();
        let mut bad = vec![KIND_DATA];
        bad.extend((MAX_PAYLOAD as u32 + 1).to_be_bytes());
        r.feed(&bad);
        assert!(matches!(r.next(), Err(ProtoError::TooLarge(_))));
    }

    #[test]
    fn an_unknown_kind_is_an_error() {
        let mut r = FrameReader::default();
        r.feed(&[200, 0, 0, 0, 0]);
        assert!(matches!(r.next(), Err(ProtoError::Kind(200))));
    }

    // A fixed-width payload that is the wrong width is a corrupt stream, not
    // a resize to a garbage size.
    #[test]
    fn a_fixed_width_payload_of_the_wrong_length_is_an_error() {
        let mut r = FrameReader::default();
        let mut bad = vec![KIND_RESIZE];
        bad.extend(3u32.to_be_bytes());
        bad.extend([0, 80, 0]);
        r.feed(&bad);
        assert!(matches!(r.next(), Err(ProtoError::Malformed(_))));
    }

    #[test]
    fn a_ring_under_its_cap_replays_everything_it_was_given() {
        let mut ring = Ring::new(1024);
        ring.push(b"one\n");
        ring.push(b"two\n");
        assert_eq!(ring.replay(), b"\x1b[0mone\ntwo\n".to_vec());
    }

    #[test]
    fn an_empty_ring_replays_nothing_at_all() {
        // Not even the reset: a fresh card must not be handed a stray escape.
        assert!(Ring::new(1024).replay().is_empty());
    }

    // Anything older than the cap is gone, and what is left starts at a line
    // boundary so a replay never begins halfway through one.
    #[test]
    fn an_overflowing_ring_drops_whole_lines_from_the_front() {
        let mut ring = Ring::new(16);
        ring.push(b"aaaa\nbbbb\ncccc\ndddd\n");
        let out = ring.replay();
        assert!(out.starts_with(b"\x1b[0m"), "the reset is always first");
        let body = &out[4..];
        assert!(body.len() <= 16, "never over the cap: {}", body.len());
        assert!(
            body.starts_with(b"bbbb\n") || body.starts_with(b"cccc\n"),
            "a whole line, not half of one: {body:?}"
        );
        assert!(
            body.ends_with(b"dddd\n"),
            "the newest output is never the part dropped"
        );
    }

    // Binary output has no newlines to cut at; the ring must still be bounded
    // rather than scanning itself to death looking for one.
    #[test]
    fn a_ring_with_no_newlines_is_still_bounded() {
        let mut ring = Ring::new(64);
        for _ in 0..100 {
            ring.push(&[0xffu8; 32]);
        }
        assert!(ring.replay().len() <= 64 + 4);
    }

    // One push larger than the whole ring keeps the END of it: the newest
    // bytes are the ones a program just drew.
    #[test]
    fn a_push_bigger_than_the_ring_keeps_the_newest_bytes() {
        let mut ring = Ring::new(8);
        ring.push(b"0123456789abcdef");
        let out = ring.replay();
        assert!(out.ends_with(b"f"), "got {:?}", out);
        assert!(out.len() <= 8 + 4);
    }
}
