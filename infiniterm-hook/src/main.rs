//! Forwards one agent hook event to a running infiniterm over its unix socket.
//!
//! ZERO dependencies on purpose. This runs inside an agent's hook path, where
//! Claude Code blocks the turn until it returns, so it must be fast and must never
//! fail loudly. It ALWAYS exits 0 — no socket, no card id, bad input, broken pipe:
//! all are success, because a hook that breaks an agent turn is worse than no hook.
//!
//! Usage: infiniterm-hook <EventName> [agent]   (the harness pipes its JSON on stdin)
//!
//! `agent` names the agent that is not Claude Code (`codex`, `opencode`,
//! `pi`, `cursor`), so the card knows which resume command its session takes.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

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

    let path = match std::env::var_os("INFINITERM_DATA_DIR") {
        // A side-by-side instance keeps its socket in its own data dir.
        Some(dir) => std::path::PathBuf::from(dir).join("infiniterm.sock"),
        None => std::env::temp_dir().join("infiniterm.sock"),
    };
    let mut stream = UnixStream::connect(path).ok()?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_millis(200)))
        .ok()?;

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
    stream.write_all(line.as_bytes()).ok()?;
    Some(())
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
