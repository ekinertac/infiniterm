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
//! Usage: infiniterm-hook <EventName>   (the harness pipes its JSON on stdin)

use std::io::{Read, Write};

fn main() {
    let _ = try_report();
    std::process::exit(0);
}

fn try_report() -> Option<()> {
    let event = std::env::args().nth(1)?;
    let card_id = std::env::var("INFINITERM_CARD_ID").ok()?;

    let mut payload = String::new();
    std::io::stdin().read_to_string(&mut payload).ok();
    let payload = payload.trim();
    let payload = if payload.starts_with('{') { payload } else { "{}" };

    let mut stream = connect(&endpoint())?;
    let line = format!(
        "{{\"card_id\":{},\"event\":{},\"payload\":{}}}\n",
        json_string(&card_id),
        json_string(&event),
        payload
    );
    stream.write_all(line.as_bytes()).ok()?;
    Some(())
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
/// No write timeout, unlike the unix side: a `File` has none. The pipe's
/// buffer is 64 KiB against a line of a few hundred bytes and the app reads
/// each connection at once, so a write here does not block in practice.
#[cfg(windows)]
fn connect(path: &std::path::Path) -> Option<impl Write> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .ok()
}

/// The timeout matters here: the app may be wedged, and a hook that waits on
/// it holds up the agent's turn.
#[cfg(unix)]
fn connect(path: &std::path::Path) -> Option<impl Write> {
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
