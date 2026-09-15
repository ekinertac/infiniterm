//! Request/response for `ift`, over the socket the hooks already use. From
//! the Tauri app's cli.rs.
//!
//! Hook reports go one way, which is right for state. A CLI verb is the
//! opposite: `ift ls` has to come back with something, and every answer it
//! could give lives in the app model on the UI thread (the card list, the
//! groups, what is focused). So a request round-trips: socket thread, then
//! the UI thread over a channel it drains each frame, then `reply`, and the
//! socket thread wakes and writes the line back. The correlation id keeps
//! two concurrent `ift` calls from taking each other's answers; the timeout
//! keeps a shell from hanging forever when the UI is not up yet. The
//! reference waited on a webview; here it waits on the UI thread, which is
//! the same thing one hop shorter.
//!
//! The verb set is FIXED and small; `ift` is not a view onto the command
//! registry (see the CLI crate's main.rs for why).
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;
use std::time::Duration;

/// How long a verb waits for the UI before giving up. Long enough to cover
/// a startup, short enough that a shell does not appear to hang.
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliRequest {
    pub id: u64,
    pub cmd: String,
    pub args: Vec<String>,
    /// The card that ran `ift`, from INFINITERM_CARD_ID. Absent when run
    /// from a terminal outside infiniterm, which some verbs can work without.
    pub card_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CliReply {
    pub ok: bool,
    pub text: String,
}

impl CliReply {
    fn err(text: &str) -> Self {
        Self {
            ok: false,
            text: text.to_string(),
        }
    }
}

/// Reads a request line, or `None` when the line is a hook report or
/// nonsense. A request is told apart by having a `cmd`, which a hook report
/// never has.
pub fn parse_request(line: &str) -> Option<(String, Vec<String>, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let cmd = v.get("cmd")?.as_str()?.to_string();
    let args = v
        .get("args")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let card_id = v
        .get("card_id")
        .and_then(|c| c.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some((cmd, args, card_id))
}

#[derive(Default)]
pub struct CliState {
    /// Where requests go: the UI thread's inbox. Replaceable, so the UI can
    /// resubscribe after a restart of its own.
    sink: Mutex<Option<Sender<CliRequest>>>,
    /// Replies waiting to be delivered, by request id.
    pending: Mutex<HashMap<u64, Sender<CliReply>>>,
    next_id: AtomicU64,
}

impl CliState {
    /// The UI registers here to receive requests.
    pub fn subscribe(&self, sink: Sender<CliRequest>) {
        *self.sink.lock().unwrap() = Some(sink);
    }

    /// The UI's answer to one request. Removed, not read: a second reply to
    /// the same id has nowhere to go, which stops a confused caller from
    /// unblocking an unrelated request.
    pub fn reply(&self, id: u64, ok: bool, text: String) {
        let waiting = self.pending.lock().unwrap().remove(&id);
        if let Some(tx) = waiting {
            let _ = tx.send(CliReply { ok, text });
        }
    }

    /// Sends a request to the UI and waits for its answer. Called from the
    /// socket thread, so blocking here blocks only that connection.
    pub fn dispatch(&self, cmd: String, args: Vec<String>, card_id: Option<String>) -> CliReply {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = channel();
        self.pending.lock().unwrap().insert(id, tx);

        let sent = {
            let sink = self.sink.lock().unwrap();
            match sink.as_ref() {
                Some(ch) => ch
                    .send(CliRequest {
                        id,
                        cmd,
                        args,
                        card_id,
                    })
                    .is_ok(),
                None => false,
            }
        };
        if !sent {
            self.pending.lock().unwrap().remove(&id);
            return CliReply::err("infiniterm is running but its window is not ready");
        }
        match rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(reply) => reply,
            Err(_) => {
                // Cleared so a late reply cannot sit in the map forever.
                self.pending.lock().unwrap().remove(&id);
                CliReply::err("infiniterm did not answer in time")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_request() {
        let (cmd, args, card) =
            parse_request(r#"{"cmd":"group","args":["humbl.ai"],"card_id":"c1"}"#).unwrap();
        assert_eq!(cmd, "group");
        assert_eq!(args, ["humbl.ai"]);
        assert_eq!(card.as_deref(), Some("c1"));
    }

    #[test]
    fn a_request_may_have_no_args_and_no_card() {
        let (cmd, args, card) = parse_request(r#"{"cmd":"ls"}"#).unwrap();
        assert_eq!(cmd, "ls");
        assert!(args.is_empty());
        assert!(card.is_none());
    }

    // An empty INFINITERM_CARD_ID is what a shell outside infiniterm exports.
    #[test]
    fn an_empty_card_id_is_no_card_id() {
        let (_, _, card) = parse_request(r#"{"cmd":"ls","card_id":""}"#).unwrap();
        assert!(card.is_none());
    }

    // One socket serves both, so the two shapes must be unmistakable.
    #[test]
    fn a_hook_report_is_not_a_request() {
        assert!(parse_request(r#"{"card_id":"abc","event":"Stop","payload":{}}"#).is_none());
    }

    #[test]
    fn garbage_is_not_a_request() {
        assert!(parse_request("not json").is_none());
        assert!(parse_request(r#"{"cmd":42}"#).is_none());
        assert!(parse_request("").is_none());
    }

    // With no UI attached the verb fails at once rather than waiting out the
    // timeout for an answer that cannot come.
    #[test]
    fn reports_when_no_window_is_listening() {
        let state = CliState::default();
        let reply = state.dispatch("ls".into(), vec![], None);
        assert!(!reply.ok);
        assert!(reply.text.contains("not ready"), "{}", reply.text);
    }

    // Native check: the round trip through a subscribed UI.
    #[test]
    fn a_subscribed_ui_answers_by_id() {
        let state = std::sync::Arc::new(CliState::default());
        let (tx, rx) = channel();
        state.subscribe(tx);
        let ui = {
            let state = state.clone();
            std::thread::spawn(move || {
                let req: CliRequest = rx.recv().unwrap();
                state.reply(req.id, true, format!("answered {}", req.cmd));
            })
        };
        let reply = state.dispatch("ls".into(), vec![], None);
        ui.join().unwrap();
        assert_eq!(
            reply,
            CliReply {
                ok: true,
                text: "answered ls".into()
            }
        );
    }
}
