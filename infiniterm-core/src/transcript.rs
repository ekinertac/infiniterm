//! Lines for the transcript card: what one turn of an agent session looks
//! like in a list, and how a tool call reads as one line. Port of
//! transcript.ts and its tests.
//!
//! Two halves in one file. The parser (from the Tauri app's transcript.rs)
//! walks the session JSONL once and returns TURNS: a human prompt, or one
//! assistant turn holding its text and its tool calls with their results.
//! Thinking blocks are dropped (most of the bytes, none of the story),
//! sidechain records (subagents) are skipped, and every string is capped so
//! a turn that pasted a 2 MB file does not become a 2 MB card; the file
//! grows to tens of MB over a day and the card must never see it raw. The
//! formatting half (from transcript.ts) is what the transcript card lists.
//!
//! The path comes from the agent's own hook payload (`transcript_path` in
//! every Claude Code hook event), stored on the card; `hooks.rs` is where
//! it arrives and `files.rs`'s mtime is how a still-growing file is re-read.
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// The tool's input as the agent wrote it, usually JSON.
    pub input: String,
    pub result: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub role: Role,
    /// An ISO timestamp (Claude Code) or Unix milliseconds (Pi); empty when
    /// there is none.
    pub at: String,
    pub text: String,
    pub tools: Vec<ToolCall>,
}

/// `23:04:12.345` from Unix ms, local time. The agent log needs seconds and
/// milliseconds where a transcript turn only needs the hour and minute: the
/// question it answers is how fast a card is being flipped.
///
/// Here rather than in a module of its own because this is where chrono
/// already is, and one timestamp format per crate is enough.
pub fn clock_ms(at_ms: f64) -> String {
    // `NAN as i64` is 0 in Rust, which would stamp the epoch and look like a
    // real time; a clock that lies is worse than a blank one.
    if !at_ms.is_finite() {
        return String::new();
    }
    Local
        .timestamp_millis_opt(at_ms as i64)
        .single()
        .map(|t| t.format("%H:%M:%S%.3f").to_string())
        .unwrap_or_default()
}

/// `14:05` from an ISO timestamp or Unix ms, local time; empty when there
/// is none or it does not parse.
pub fn turn_time(at: &str) -> String {
    let local: Option<DateTime<Local>> = if !at.is_empty() && at.bytes().all(|b| b.is_ascii_digit())
    {
        at.parse::<i64>()
            .ok()
            .and_then(|ms| Local.timestamp_millis_opt(ms).single())
    } else if let Ok(dt) = DateTime::parse_from_rfc3339(at) {
        Some(dt.with_timezone(&Local))
    } else {
        // A timestamp with no zone is local time, as `new Date` reads it.
        NaiveDateTime::parse_from_str(at, "%Y-%m-%dT%H:%M:%S%.f")
            .ok()
            .and_then(|n| Local.from_local_datetime(&n).single())
    };
    local
        .map(|d| d.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// The one-line summary for the list: the first line of text with any
/// length, or, for an assistant turn that only called tools, the first call.
pub fn turn_preview(turn: &Turn, max: usize) -> String {
    let line = turn
        .text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let source = if line.is_empty() {
        turn.tools.first().map(tool_line).unwrap_or_default()
    } else {
        line.to_string()
    };
    if source.chars().count() > max {
        let mut cut: String = source.chars().take(max.saturating_sub(1)).collect();
        cut.push('…');
        cut
    } else {
        source
    }
}

/// The field of a tool's input worth showing beside its name, by the tools
/// an agent runs most: the command, the path, the pattern.
const KEY_FIELDS: [&str; 8] = [
    "command",
    "file_path",
    "pattern",
    "path",
    "query",
    "url",
    "description",
    "prompt",
];

/// Anything else shows its first string value, and an input that is not
/// JSON shows as is.
pub fn tool_line(tool: &ToolCall) -> String {
    let arg = match serde_json::from_str::<Value>(&tool.input) {
        Ok(Value::Object(input)) => {
            let key = KEY_FIELDS
                .iter()
                .copied()
                .find(|k| input.get(*k).is_some_and(Value::is_string))
                .or_else(|| {
                    input
                        .iter()
                        .find(|(_, v)| v.is_string())
                        .map(|(k, _)| k.as_str())
                });
            key.and_then(|k| input[k].as_str())
                .and_then(|s| s.lines().next())
                .unwrap_or("")
                .to_string()
        }
        Ok(_) => String::new(),
        Err(_) => tool.input.clone(),
    };
    if arg.is_empty() {
        tool.name.clone()
    } else {
        format!("{}: {arg}", tool.name)
    }
}

const TEXT_CAP: usize = 20_000;
const INPUT_CAP: usize = 400;
const RESULT_CAP: usize = 4_000;

fn cap(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    // Cut on a char boundary, never inside one.
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// The text of a `content` that may be a string or a list of blocks.
fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// A tool_result block's id and text, whichever of the two formats wrote it.
fn result_parts(block: &Value) -> (&str, String) {
    let id = block
        .get("tool_use_id")
        .or_else(|| block.get("toolCallId"))
        .and_then(Value::as_str)
        .unwrap_or("");
    (id, text_of(block.get("content").unwrap_or(&Value::Null)))
}

/// Claude Code writes `type: user|assistant` with the role implied; Pi
/// writes `type: message` with `message.role` (user, assistant, toolResult),
/// spells a call `toolCall` with `arguments`, and dates in Unix ms. The
/// same turns come out of both, which is the point: an agent card is an
/// agent card whichever harness is in it.
fn role_of(v: &Value) -> &str {
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "message" => v
            .get("message")
            .and_then(|m| m.get("role"))
            .and_then(Value::as_str)
            .unwrap_or(""),
        kind => kind,
    }
}

fn time_of(v: &Value) -> String {
    match v.get("timestamp") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

pub fn parse_transcript(jsonl: &str) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    for line in jsonl.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let kind = role_of(&v);
        let at = time_of(&v);
        let Some(message) = v.get("message") else {
            continue;
        };
        let Some(content) = message.get("content") else {
            continue;
        };
        match kind {
            // Pi's tool results are their own messages rather than user records.
            "toolResult" => {
                let (id, text) = result_parts(message);
                if let Some(turn) = turns.iter_mut().rev().find(|t| t.role == Role::Assistant) {
                    if let Some(call) = turn.tools.iter_mut().find(|c| c.id == id) {
                        call.result = cap(&text, RESULT_CAP);
                    }
                }
            }
            "user" => {
                // A user record made of tool results is the harness answering the
                // assistant, not a person typing; it belongs to the assistant turn.
                let results: Vec<&Value> = match content {
                    Value::Array(blocks) => blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
                        .collect(),
                    _ => Vec::new(),
                };
                if !results.is_empty() {
                    if let Some(turn) = turns.iter_mut().rev().find(|t| t.role == Role::Assistant) {
                        for r in results {
                            let (id, text) = result_parts(r);
                            if let Some(call) = turn.tools.iter_mut().find(|c| c.id == id) {
                                call.result = cap(&text, RESULT_CAP);
                            }
                        }
                    }
                    continue;
                }
                let text = text_of(content);
                if text.trim().is_empty() {
                    continue;
                }
                turns.push(Turn {
                    role: Role::User,
                    at,
                    text: cap(&text, TEXT_CAP),
                    tools: Vec::new(),
                });
            }
            "assistant" => {
                // One assistant turn spans many records (one per content block);
                // they run together until a person speaks again.
                if turns.last().map(|t| t.role) != Some(Role::Assistant) {
                    turns.push(Turn {
                        role: Role::Assistant,
                        at: at.clone(),
                        text: String::new(),
                        tools: Vec::new(),
                    });
                }
                let turn = turns.last_mut().unwrap();
                let Value::Array(blocks) = content else {
                    continue;
                };
                for b in blocks {
                    match b.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let t = b.get("text").and_then(Value::as_str).unwrap_or("");
                            if !turn.text.is_empty() {
                                turn.text.push('\n');
                            }
                            turn.text.push_str(t);
                            turn.text = cap(&turn.text, TEXT_CAP);
                        }
                        Some("tool_use") | Some("toolCall") => turn.tools.push(ToolCall {
                            id: b
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            name: b
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("?")
                                .to_string(),
                            input: cap(
                                &b.get("input")
                                    .or_else(|| b.get("arguments"))
                                    .map(|i| i.to_string())
                                    .unwrap_or_default(),
                                INPUT_CAP,
                            ),
                            result: String::new(),
                        }),
                        _ => {} // thinking, and whatever comes next
                    }
                }
            }
            _ => {}
        }
    }
    turns
}

pub fn transcript_read(path: &str) -> Result<Vec<Turn>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    Ok(parse_transcript(&text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serde_json::json;

    fn tool(name: &str, input: Value) -> ToolCall {
        ToolCall {
            id: "t".into(),
            name: name.into(),
            input: input.to_string(),
            result: String::new(),
        }
    }

    fn turn(text: &str, tools: Vec<ToolCall>) -> Turn {
        Turn {
            role: Role::Assistant,
            at: String::new(),
            text: text.into(),
            tools,
        }
    }

    #[test]
    fn shows_the_field_that_matters_for_the_common_tools() {
        assert_eq!(
            tool_line(&tool(
                "Bash",
                json!({"command": "git status", "description": "x"})
            )),
            "Bash: git status"
        );
        assert_eq!(
            tool_line(&tool("Read", json!({"file_path": "/a/b.ts", "limit": 5}))),
            "Read: /a/b.ts"
        );
        assert_eq!(
            tool_line(&tool("Grep", json!({"pattern": "foo", "path": "/x"}))),
            "Grep: foo"
        );
    }

    #[test]
    fn falls_back_to_the_first_string_field_then_to_the_raw_input_then_to_the_name() {
        assert_eq!(
            tool_line(&tool("Custom", json!({"n": 1, "label": "hi"}))),
            "Custom: hi"
        );
        let odd = ToolCall {
            id: "t".into(),
            name: "Odd".into(),
            input: "not json".into(),
            result: String::new(),
        };
        assert_eq!(tool_line(&odd), "Odd: not json");
        assert_eq!(tool_line(&tool("Empty", json!({}))), "Empty");
    }

    #[test]
    fn keeps_only_the_first_line_of_a_multi_line_command() {
        assert_eq!(
            tool_line(&tool("Bash", json!({"command": "a\nb\nc"}))),
            "Bash: a"
        );
    }

    #[test]
    fn is_the_first_non_blank_line_of_the_text() {
        assert_eq!(
            turn_preview(&turn("\n\n  hello there\nmore", vec![]), 80),
            "hello there"
        );
    }

    #[test]
    fn is_the_first_tool_call_when_the_turn_has_no_text() {
        assert_eq!(
            turn_preview(&turn("", vec![tool("Bash", json!({"command": "ls"}))]), 80),
            "Bash: ls"
        );
    }

    #[test]
    fn truncates_with_an_ellipsis() {
        assert_eq!(turn_preview(&turn("abcdefghij", vec![]), 5), "abcd…");
    }

    #[test]
    fn a_clock_stamp_carries_seconds_and_milliseconds() {
        let t = clock_ms(1_726_520_652_345.);
        assert_eq!(t.len(), 12, "HH:MM:SS.mmm, got {t}");
        assert_eq!(&t[2..3], ":");
        assert_eq!(&t[5..6], ":");
        assert_eq!(&t[8..9], ".");
        assert_eq!(clock_ms(f64::NAN), "");
    }

    #[test]
    fn is_empty_for_a_missing_or_broken_timestamp() {
        assert_eq!(turn_time(""), "");
        assert_eq!(turn_time("nope"), "");
    }

    fn nine_oh_five() -> DateTime<Local> {
        Local
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2026, 9, 14)
                    .unwrap()
                    .and_hms_opt(9, 5, 0)
                    .unwrap(),
            )
            .unwrap()
    }

    #[test]
    fn is_hours_and_minutes_zero_padded() {
        assert_eq!(turn_time(&nine_oh_five().to_rfc3339()), "09:05");
        // As Claude Code writes it: UTC with a Z.
        assert_eq!(
            turn_time(
                &nine_oh_five()
                    .to_utc()
                    .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                    .to_string()
            ),
            "09:05"
        );
    }

    #[test]
    fn reads_unix_milliseconds_too_which_is_what_pi_writes() {
        assert_eq!(
            turn_time(&nine_oh_five().timestamp_millis().to_string()),
            "09:05"
        );
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;
    use Role::{Assistant, User};

    const SAMPLE: &str = r#"{"type":"custom-title","customTitle":"x"}
{"type":"user","timestamp":"t1","message":{"role":"user","content":"hello"}}
{"type":"assistant","timestamp":"t2","message":{"content":[{"type":"thinking","thinking":"hmm"}]}}
{"type":"assistant","timestamp":"t3","message":{"content":[{"type":"text","text":"Looking."}]}}
{"type":"assistant","timestamp":"t4","message":{"content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"ls"}}]}}
{"type":"user","timestamp":"t5","message":{"content":[{"type":"tool_result","tool_use_id":"tu1","content":"a.txt\nb.txt"}]}}
{"type":"assistant","timestamp":"t6","message":{"content":[{"type":"text","text":"Two files."}]}}
{"type":"user","timestamp":"t7","isSidechain":true,"message":{"content":"subagent prompt"}}
{"type":"user","timestamp":"t8","message":{"content":[{"type":"text","text":"thanks"}]}}
not json
"#;

    #[test]
    fn groups_records_into_human_and_assistant_turns() {
        let turns = parse_transcript(SAMPLE);
        let roles: Vec<Role> = turns.iter().map(|t| t.role).collect();
        assert_eq!(roles, [User, Assistant, User]);
        assert_eq!(turns[0].text, "hello");
        assert_eq!(turns[2].text, "thanks");
    }

    #[test]
    fn an_assistant_turn_collects_its_text_and_tools_and_drops_thinking() {
        let turns = parse_transcript(SAMPLE);
        let a = &turns[1];
        assert_eq!(a.at, "t2");
        assert_eq!(a.text, "Looking.\nTwo files.");
        assert_eq!(a.tools.len(), 1);
        assert_eq!(a.tools[0].name, "Bash");
        assert_eq!(a.tools[0].input, r#"{"command":"ls"}"#);
    }

    #[test]
    fn a_tool_result_lands_on_the_call_it_answers() {
        let turns = parse_transcript(SAMPLE);
        assert_eq!(turns[1].tools[0].result, "a.txt\nb.txt");
    }

    #[test]
    fn sidechain_records_are_skipped() {
        let turns = parse_transcript(SAMPLE);
        assert!(turns.iter().all(|t| t.text != "subagent prompt"));
    }

    const PI: &str = r#"{"type":"session_info","name":"x"}
{"type":"message","id":"a","timestamp":"2026-09-08T21:51:00.000Z","message":{"role":"user","content":[{"type":"text","text":"list files"}],"timestamp":1788904000000}}
{"type":"message","id":"b","message":{"role":"assistant","content":[{"type":"thinking","thinking":"..."},{"type":"text","text":"Sure."},{"type":"toolCall","id":"call_1","name":"bash","arguments":{"command":"ls"}}],"timestamp":1788904284549}}
{"type":"message","id":"c","message":{"role":"toolResult","toolCallId":"call_1","toolName":"bash","content":[{"type":"text","text":"a.txt"}],"isError":false,"timestamp":1788904284600}}
{"type":"message","id":"d","message":{"role":"assistant","content":[{"type":"text","text":"One file."}],"timestamp":1788904290000}}
"#;

    #[test]
    fn reads_a_pi_session_into_the_same_turns() {
        let turns = parse_transcript(PI);
        let roles: Vec<Role> = turns.iter().map(|t| t.role).collect();
        assert_eq!(roles, [User, Assistant]);
        assert_eq!(turns[0].text, "list files");
        assert_eq!(turns[0].at, "2026-09-08T21:51:00.000Z");
        let a = &turns[1];
        assert_eq!(a.text, "Sure.\nOne file.");
        assert_eq!(a.tools[0].name, "bash");
        assert_eq!(a.tools[0].input, r#"{"command":"ls"}"#);
        assert_eq!(a.tools[0].result, "a.txt");
    }

    #[test]
    fn caps_cut_on_a_char_boundary() {
        let s = "é".repeat(10);
        let c = cap(&s, 5);
        assert!(c.starts_with("éé"));
        assert!(c.ends_with('…'));
        assert_eq!(cap("short", 10), "short");
    }
}
