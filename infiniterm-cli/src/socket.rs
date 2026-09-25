//! Talking to a running infiniterm.
//!
//! The socket's existence IS the answer to "is infiniterm running", which is why
//! there is no pid file, no port and no handshake: if the connect fails, it is not
//! running, and that is a different exit code from a verb that failed.
//!
//! One request, one response line, then close. `ift` is a dumb pipe — every
//! decision about what a command means is made in the app, so the two surfaces
//! cannot drift.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// Longer than the app's own reply timeout, so a verb that timed out in there
/// reports the app's reason rather than this one's.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Reply {
    pub ok: bool,
    pub text: String,
}

fn socket_path() -> std::path::PathBuf {
    // Must match the app's socket_path. Both run as the same user, so the
    // per-user TMPDIR macOS hands out resolves identically. A side-by-side
    // instance on INFINITERM_DATA_DIR keeps its socket there, and an `ift`
    // run with the same variable set talks to that one.
    if let Some(dir) = std::env::var_os("INFINITERM_DATA_DIR") {
        return std::path::PathBuf::from(dir).join("infiniterm.sock");
    }
    std::env::temp_dir().join("infiniterm.sock")
}

/// The card this is running in, from the env var the app exports at spawn.
fn card_id() -> String {
    std::env::var("INFINITERM_CARD_ID").unwrap_or_default()
}

/// Run from a shell inside an infiniterm card.
pub fn in_a_card() -> bool {
    !card_id().is_empty()
}

pub fn request(cmd: &str, args: Vec<String>) -> Result<Reply, String> {
    exchange(cmd, args, Some(READ_TIMEOUT))
}

/// A request answered when the user is done (`edit`): no timeout.
pub fn request_waiting(cmd: &str, args: Vec<String>) -> Result<Reply, String> {
    exchange(cmd, args, None)
}

fn exchange(cmd: &str, args: Vec<String>, timeout: Option<Duration>) -> Result<Reply, String> {
    let path = socket_path();
    let mut stream =
        UnixStream::connect(&path).map_err(|_| "infiniterm is not running".to_string())?;
    let _ = stream.set_read_timeout(timeout);

    let body = serde_json::json!({ "cmd": cmd, "args": args, "card_id": card_id() });
    writeln!(stream, "{body}").map_err(|e| format!("could not send: {e}"))?;
    stream.flush().map_err(|e| format!("could not send: {e}"))?;

    let mut line = String::new();
    BufReader::new(&stream)
        .read_line(&mut line)
        .map_err(|e| format!("no answer: {e}"))?;
    if line.trim().is_empty() {
        return Err("infiniterm closed the connection without answering".into());
    }

    let v: serde_json::Value =
        serde_json::from_str(&line).map_err(|e| format!("bad answer: {e}"))?;
    Ok(Reply {
        ok: v.get("ok").and_then(serde_json::Value::as_bool).unwrap_or(false),
        text: v
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}
