//! Forwards one agent hook event to a running infiniterm over its local socket.
//!
//! ZERO dependencies on purpose, on Windows too. This runs inside an agent's hook
//! path, where Claude Code blocks the turn until it returns, so it must be fast and
//! must never fail loudly. It ALWAYS exits 0 — no socket, no card id, bad input,
//! broken pipe: all are success, because a hook that breaks an agent turn is worse
//! than no hook.
//!
//! The endpoint is a unix socket on unix and a named pipe on Windows, and this file
//! carries its own copy of both answers. `infiniterm-core`'s `paths::socket_path`
//! and `transport` are the originals; linking them here would drag serde, regex and
//! a pty crate into a binary on an agent's hot path. Two copies, one rule: change
//! one and change the other.
//!
//! Usage: infiniterm-hook <EventName> [agent]   (the harness pipes its JSON on stdin)
//!
//! `agent` names the agent that is not Claude Code (`codex`, `opencode`,
//! `pi`, `cursor`), so the card knows which resume command its session takes.

use std::io::{Read, Write};

fn main() {
    let _ = try_report();
    std::process::exit(0);
}

fn try_report() -> Option<()> {
    let event = std::env::args().nth(1)?;
    let agent = std::env::args().nth(2);
    let card_id = std::env::var("INFINITERM_CARD_ID").ok()?;

    let mut payload = String::new();
    std::io::stdin().read_to_string(&mut payload).ok();
    let payload = payload.trim();
    let payload = if payload.starts_with('{') { payload } else { "{}" };

    let stream = connect(&endpoint())?;

    let agent = match agent {
        Some(a) => format!(",\"agent\":{}", json_string(&a)),
        None => String::new(),
    };
    let line = format!(
        "{{\"card_id\":{},\"event\":{}{},\"payload\":{}}}\n",
        json_string(&card_id),
        json_string(&event),
        agent,
        payload
    );
    send(stream, line.as_bytes()).then_some(())
}

/// How long the hook gives the app, for each of opening its endpoint and
/// writing the line. The hook runs inside an agent's turn, which waits for
/// it: an app that is wedged or busy must cost a turn at most this long, and
/// then the event is dropped, which is cheaper than a stalled agent.
#[cfg(windows)]
const BUDGET: std::time::Duration = std::time::Duration::from_millis(200);

/// Windows' `ERROR_PIPE_BUSY`: every instance of the pipe is serving another
/// client for the moment. Not "nobody is listening", so it is worth a retry.
#[cfg(any(windows, test))]
const ERROR_PIPE_BUSY: i32 = 231;

/// Runs `open` until it works, retrying only while the pipe is busy and only
/// within `budget`. Any other error means there is no app to tell.
#[cfg(any(windows, test))]
fn open_within<T>(
    budget: std::time::Duration,
    mut open: impl FnMut() -> std::io::Result<T>,
) -> Option<T> {
    let start = std::time::Instant::now();
    loop {
        match open() {
            Ok(v) => return Some(v),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && start.elapsed() < budget => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(_) => return None,
        }
    }
}

/// Writes `bytes` on a thread and waits at most `budget` for it. A `File` has
/// no write timeout, so this is where a stuck pipe gets cut off; the thread
/// is left behind and ends with the process, which exits right after.
#[cfg(any(windows, test))]
fn write_within<W: Write + Send + 'static>(
    mut w: W,
    bytes: Vec<u8>,
    budget: std::time::Duration,
) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(w.write_all(&bytes).is_ok());
    });
    rx.recv_timeout(budget).unwrap_or(false)
}

/// The unix socket carries its own write timeout (see `connect`).
#[cfg(unix)]
fn send<W: Write + Send + 'static>(mut w: W, bytes: &[u8]) -> bool {
    w.write_all(bytes).is_ok()
}

#[cfg(windows)]
fn send<W: Write + Send + 'static>(w: W, bytes: &[u8]) -> bool {
    write_within(w, bytes.to_vec(), BUDGET)
}

/// Where a running infiniterm listens. Must match `paths::socket_path`.
fn endpoint() -> std::path::PathBuf {
    // A side-by-side instance keeps its endpoint to itself.
    let data_dir = std::env::var_os("INFINITERM_DATA_DIR");
    #[cfg(windows)]
    {
        // A pipe has no directory to live in, so the data dir goes into its
        // NAME as a hash: a pipe name may not contain a backslash past the
        // prefix, and every data dir does.
        match data_dir {
            Some(dir) => std::path::PathBuf::from(format!(
                r"\\.\pipe\infiniterm-{:016x}",
                fnv1a(&dir.to_string_lossy())
            )),
            None => std::path::PathBuf::from(r"\\.\pipe\infiniterm"),
        }
    }
    #[cfg(unix)]
    {
        match data_dir {
            Some(dir) => std::path::PathBuf::from(dir).join("infiniterm.sock"),
            None => std::env::temp_dir().join("infiniterm.sock"),
        }
    }
}

/// FNV-1a, the copy of `paths::path_hash`. Not a security boundary: it only
/// has to keep two data dirs on one machine from colliding.
#[cfg(windows)]
fn fnv1a(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A byte-mode named pipe opens as a file; nothing beyond std is needed for
/// the client half, which is the whole reason this binary can stay
/// dependency-free on Windows.
///
/// A `File` has no write timeout, unlike the unix side, so `send` writes on
/// a thread and gives up after `BUDGET`. Opening retries while every pipe
/// instance is busy with another client (another hook, say), and gives up at
/// the same budget: an event dropped there was a lost agent state change.
#[cfg(windows)]
fn connect(path: &std::path::Path) -> Option<impl Write + Send + 'static> {
    open_within(BUDGET, || {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
    })
}

/// The timeout matters here: the app may be wedged, and a hook that waits on
/// it holds up the agent's turn.
#[cfg(unix)]
fn connect(path: &std::path::Path) -> Option<impl Write + Send + 'static> {
    let stream = std::os::unix::net::UnixStream::connect(path).ok()?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_millis(200)))
        .ok()?;
    Some(stream)
}

/// Minimal JSON string escaping — enough for a uuid and an event name, and cheaper
/// than a serde dependency in a binary on an agent's hot path.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::time::{Duration, Instant};

    fn busy() -> io::Error {
        io::Error::from_raw_os_error(ERROR_PIPE_BUSY)
    }

    // Another client holds every instance of the pipe for a moment; the next
    // try is free.
    #[test]
    fn a_busy_pipe_is_tried_again_until_it_opens() {
        let mut calls = 0;
        let got = open_within(Duration::from_secs(1), || {
            calls += 1;
            if calls < 4 {
                Err(busy())
            } else {
                Ok("open")
            }
        });
        assert_eq!(got, Some("open"));
        assert_eq!(calls, 4);
    }

    // No pipe at all means no app: one try, no waiting.
    #[test]
    fn any_other_error_gives_up_at_once() {
        let mut calls = 0;
        let got: Option<()> = open_within(Duration::from_secs(1), || {
            calls += 1;
            Err(io::Error::from(io::ErrorKind::NotFound))
        });
        assert_eq!(got, None);
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_pipe_that_stays_busy_is_given_up_on_at_the_budget() {
        let start = Instant::now();
        let got: Option<()> = open_within(Duration::from_millis(40), || Err(busy()));
        assert_eq!(got, None);
        assert!(start.elapsed() < Duration::from_secs(1), "{:?}", start.elapsed());
    }

    /// A pipe nobody reads: the write never returns.
    struct Stuck;
    impl Write for Stuck {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            std::thread::sleep(Duration::from_secs(30));
            Ok(0)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    // The point of #469: a wedged app costs the agent's turn 200 ms, not
    // however long the app stays wedged.
    #[test]
    fn a_write_that_never_returns_is_cut_off_at_the_budget() {
        let start = Instant::now();
        assert!(!write_within(Stuck, b"x".to_vec(), Duration::from_millis(50)));
        assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
    }

    #[test]
    fn a_write_that_works_reports_success() {
        assert!(write_within(Vec::<u8>::new(), b"line\n".to_vec(), Duration::from_secs(1)));
    }

    #[test]
    fn a_json_string_escapes_what_would_break_the_line() {
        assert_eq!(json_string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }
}
