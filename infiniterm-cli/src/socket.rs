//! Talking to a running infiniterm.
//!
//! The endpoint's existence IS the answer to "is infiniterm running", which is
//! why there is no pid file, no port and no handshake: if the connect fails, it
//! is not running, and that is a different exit code from a verb that failed.
//!
//! WHAT the endpoint is (a unix socket, a named pipe) is `infiniterm-core`'s
//! `transport`, and where it is is its `paths::socket_path`. This file kept its
//! own copy of both until Windows arrived and the two could no longer agree by
//! looking the same.
//!
//! One request, one response line, then close. `ift` is a dumb pipe — every
//! decision about what a command means is made in the app, so the two surfaces
//! cannot drift.

use infiniterm_core::{paths, transport};
use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

/// Longer than the app's own reply timeout, so a verb that timed out in there
/// reports the app's reason rather than this one's.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Reply {
    pub ok: bool,
    pub text: String,
}

/// Whether an app is listening: a connect that succeeds, the same test
/// `exchange` makes. A stale socket file from a crash fails it.
pub fn running() -> bool {
    transport::is_live(&paths::socket_path())
}

/// Polls until `running()` or `limit` passes; for an app just launched.
pub fn wait_for_socket(limit: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < limit {
        if running() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

/// The card this is running in, from the env var the app exports at spawn.
fn card_id() -> String {
    std::env::var("INFINITERM_CARD_ID").unwrap_or_default()
}

/// One reply line, or the timeout (none: wait for as long as it takes).
///
/// On a thread rather than through a socket option: a named pipe handle has no
/// read timeout to set, and keeping the guarantee matters more than the
/// mechanism it used to come from. `ift` exits right after this, so a thread
/// still blocked on a wedged app goes with the process.
fn read_reply(stream: &transport::Stream, timeout: Option<Duration>) -> Result<String, String> {
    let reply = stream.try_clone().map_err(|e| format!("no answer: {e}"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = tx.send(BufReader::new(reply).read_line(&mut line).map(|_| line));
    });
    let got = match timeout {
        Some(t) => rx.recv_timeout(t).map_err(|_| "infiniterm did not answer".to_string())?,
        None => rx.recv().map_err(|_| "infiniterm closed the connection".to_string())?,
    };
    got.map_err(|e| format!("no answer: {e}"))
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
    let path = paths::socket_path();
    let mut stream =
        transport::connect(&path).map_err(|_| "infiniterm is not running".to_string())?;

    let body = serde_json::json!({ "cmd": cmd, "args": args, "card_id": card_id() });
    writeln!(stream, "{body}").map_err(|e| format!("could not send: {e}"))?;
    stream.flush().map_err(|e| format!("could not send: {e}"))?;

    let line = read_reply(&stream, timeout)?;
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
