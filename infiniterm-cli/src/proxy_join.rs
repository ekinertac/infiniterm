//! The parts of `ift proxy` that do not depend on a socket: what a session id
//! may look like, what `proxy join` is asked for, and the byte copy both
//! halves are built on. Compiled on every platform: `proxy.rs` (unix sockets)
//! and `proxy_win.rs` (a named pipe) share it, so the id check and the usage
//! line cannot drift between them.
//!
//! The id check is a trust boundary: an id arrives over ssh from another
//! machine and becomes part of a path (unix) or is sent on to the host app
//! (Windows), so it is a short plain word and nothing else.

use std::io::{Read, Write};

/// A session id is the socket's file name stem: letters, digits, `-` and `_`.
pub const MAX_ID_LEN: usize = 64;

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The topic the host's handshake line names: `canvas` or `session <id>`.
pub fn parse_join(args: &[String]) -> Result<String, String> {
    match args {
        [t] if t == "canvas" => Ok("canvas".into()),
        [t, id] if t == "session" && valid_id(id) => Ok(format!("session {id}")),
        [t, id] if t == "session" => Err(format!("{id:?} is not a session id")),
        _ => Err("usage: ift proxy join canvas | session <id>".into()),
    }
}

/// Copies `from` to `to` until `from` ends, then runs `then` (closing `to`'s
/// write side so the other end sees the end too).
pub fn pump(mut from: impl Read, mut to: impl Write, then: impl FnOnce()) {
    let mut buf = [0u8; 16 * 1024];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if to.write_all(&buf[..n]).and_then(|_| to.flush()).is_err() {
                    break;
                }
            }
        }
    }
    then();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn an_id_is_a_short_plain_word() {
        assert!(valid_id("ccc081c9a2c44d6a"));
        assert!(valid_id("a-b_C9"));
        assert!(!valid_id(""));
        assert!(!valid_id("../x"));
        assert!(!valid_id("a/b"));
        assert!(!valid_id("a b"));
        assert!(!valid_id(&"a".repeat(MAX_ID_LEN + 1)));
    }

    #[test]
    fn join_names_the_canvas_or_one_session() {
        assert_eq!(parse_join(&a(&["canvas"])).unwrap(), "canvas");
        assert_eq!(
            parse_join(&a(&["session", "abc-1"])).unwrap(),
            "session abc-1"
        );
        assert!(parse_join(&a(&[])).is_err());
        assert!(parse_join(&a(&["session"])).is_err());
        assert!(parse_join(&a(&["session", "../x"])).is_err());
        assert!(parse_join(&a(&["canvas", "extra"])).is_err());
    }

    #[test]
    fn the_copy_carries_every_byte_value_and_then_runs_its_end_action() {
        let data: Vec<u8> = (0..=255u8).cycle().take(100_000).collect();
        let mut out = Vec::new();
        let mut ended = false;
        pump(std::io::Cursor::new(data.clone()), &mut out, || ended = true);
        assert_eq!(out, data, "binary clean");
        assert!(ended);
    }
}
