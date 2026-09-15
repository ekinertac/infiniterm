//! Lines for the transcript card: what one turn of an agent session looks
//! like in a list, and how a tool call reads as one line. Port of
//! transcript.ts and its tests.
//!
//! The turns themselves come from the backend (`transcript.rs` there, which
//! walks the session JSONL once and caps every string); this is the
//! formatting, kept pure so it is tested. The transcript card element is
//! the wiring around it.
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
