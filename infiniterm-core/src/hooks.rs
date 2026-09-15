//! The unix socket: hook reports in, and `ift` requests in and out. From
//! the Tauri app's hooks.rs.
//!
//! A unix socket rather than TCP: no port negotiation, no auth layer, and
//! filesystem permissions are the access control. The sender is the
//! separate zero-dependency `infiniterm-hook` binary, because hook programs
//! are executed by the agent harness, not by this app.
//!
//! ONE socket serves both. A second would need its own path, stale-file
//! handling and lifetime for nothing, and the socket's existence is already
//! the answer to "is infiniterm running", which is what `ift` needs to know
//! first. The two message shapes are unmistakable: a hook report has a
//! `card_id` and an `event`, a request has a `cmd` (`cli.rs`). Reports are
//! forwarded to the UI thread over a channel it drains each frame;
//! `agent_state.rs` turns them into card state.
use crate::cli::CliState;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookReport {
    pub card_id: String,
    pub event: String,
    pub tool: Option<String>,
    /// The session's JSONL, which every Claude Code hook event names.
    pub transcript: Option<String>,
}

/// A report with no `card_id` is dropped: without it there is no card to
/// update, and guessing from pid or tty is exactly the fragility the env var
/// avoids.
pub fn parse_hook_line(line: &str) -> Option<HookReport> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let card_id = v.get("card_id")?.as_str()?.to_string();
    let event = v.get("event")?.as_str()?.to_string();
    let payload = v.get("payload");
    let field = |name: &str| {
        payload
            .and_then(|p| p.get(name))
            .and_then(|t| t.as_str())
            .map(str::to_string)
    };
    Some(HookReport {
        card_id,
        event,
        tool: field("tool_name"),
        transcript: field("transcript_path"),
    })
}

/// Binds `path` and serves it on a thread for the life of the process.
/// A stale socket file from a previous run is removed first, or bind fails.
/// Fails only when the bind does; the app then runs with hooks and `ift`
/// disabled rather than not at all.
pub fn listen(path: &Path, reports: Sender<HookReport>, cli: Arc<CliState>) -> Result<(), String> {
    // A live socket means another infiniterm owns it: a second copy would
    // steal the path and leave the first unreachable by `ift` and hooks
    // until restart (the reference enforces one instance for this reason).
    if std::os::unix::net::UnixStream::connect(path).is_ok() {
        return Err(format!(
            "{} is in use: another infiniterm is running",
            path.display()
        ));
    }
    let _ = std::fs::remove_file(path);
    let listener =
        UnixListener::bind(path).map_err(|e| format!("could not bind {}: {e}", path.display()))?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let reports = reports.clone();
            let cli = cli.clone();
            // A connection per invocation, on its own thread: an `ift` verb
            // waits on the UI for up to two seconds, and a hook report
            // arriving meanwhile must not queue behind it.
            std::thread::spawn(move || {
                let Ok(mut out) = stream.try_clone() else {
                    return;
                };
                // Read to EOF, so a caller batching several lines is not
                // silently truncated.
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    if let Some(report) = parse_hook_line(&line) {
                        // Dropped if the UI is gone. The hook still exited 0.
                        let _ = reports.send(report);
                        continue;
                    }
                    if let Some((cmd, args, card_id)) = crate::cli::parse_request(&line) {
                        let reply = cli.dispatch(cmd, args, card_id);
                        let body = serde_json::to_string(&reply).unwrap_or_default();
                        let _ = writeln!(out, "{body}");
                        let _ = out.flush();
                    }
                }
            });
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A second copy must not steal a live socket from the first.
    #[test]
    fn refuses_a_socket_another_instance_holds() {
        let path =
            std::env::temp_dir().join(format!("infiniterm-test-live-{}.sock", std::process::id()));
        let (tx, _rx) = std::sync::mpsc::channel();
        let cli = Arc::new(CliState::default());
        listen(&path, tx.clone(), cli.clone()).unwrap();
        let err = listen(&path, tx, cli).unwrap_err();
        assert!(err.contains("another infiniterm"), "{err}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parses_a_well_formed_report() {
        let r = parse_hook_line(
            r#"{"card_id":"abc","event":"PreToolUse","payload":{"tool_name":"Bash"}}"#,
        )
        .unwrap();
        assert_eq!(r.card_id, "abc");
        assert_eq!(r.event, "PreToolUse");
        assert_eq!(r.tool.as_deref(), Some("Bash"));
    }

    #[test]
    fn parses_a_report_without_a_tool() {
        let r = parse_hook_line(r#"{"card_id":"abc","event":"Stop","payload":{}}"#).unwrap();
        assert_eq!(r.event, "Stop");
        assert!(r.tool.is_none());
    }

    #[test]
    fn drops_a_report_with_no_card_id() {
        assert!(parse_hook_line(r#"{"event":"Stop","payload":{}}"#).is_none());
    }

    #[test]
    fn drops_garbage_without_panicking() {
        assert!(parse_hook_line("not json at all").is_none());
        assert!(parse_hook_line("").is_none());
        assert!(parse_hook_line(r#"{"card_id":"abc"}"#).is_none());
    }

    #[test]
    fn tolerates_a_truncated_line() {
        assert!(parse_hook_line(r#"{"card_id":"abc","event":"Sto"#).is_none());
    }

    // Harnesses differ; a scalar payload must not panic the parser.
    #[test]
    fn ignores_a_payload_that_is_not_an_object() {
        let r = parse_hook_line(r#"{"card_id":"a","event":"Stop","payload":7}"#).unwrap();
        assert!(r.tool.is_none());
    }
}
