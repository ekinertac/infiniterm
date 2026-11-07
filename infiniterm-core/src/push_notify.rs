//! Phone pushes when a card needs you (#616): which state changes become a
//! push, what the push says, and the ntfy request body.
//!
//! Where it fits: `Model::push_tick` compares the card states with the last
//! ones it saw and returns `Effect::PushNotify`; the ui's `runtime.rs` reads
//! the topic file, builds the body here and posts it from a background
//! thread. Pure and without gpui, so every rule below is tested.
//!
//! Why the Mac sends it: the iPhone companion is suspended soon after it
//! leaves the screen, and the host drops a join viewer that stops reading.
//! The Mac is always running and already knows each card's state.
//!
//! Related: `agent_state.rs` (the states), `config.rs` (`Notify`, the
//! `notify.*` settings), `docs/agent/agents-and-state.md`.
//!
//! Constraints:
//! - The topic is a secret (ntfy has no accounts, the topic is the
//!   password), so it is NOT a setting: settings.json is often synced or
//!   committed. It lives in the one-line file `ntfy-topic` in the config
//!   folder, which the app never writes.
//! - JSON publishing, not headers: a card title can be any text and a header
//!   would need RFC 2047 encoding.

use crate::agent_state::AgentState;
use serde_json::json;
use std::collections::BTreeMap;

/// The file in the config folder that holds the topic.
pub const TOPIC_FILE: &str = "ntfy-topic";

/// ntfy's own limit on a topic name.
pub const TOPIC_MAX: usize = 64;

/// A card whose state changed to one that needs you.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub card_id: String,
    pub state: AgentState,
}

/// What the phone shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Push {
    pub title: String,
    pub message: String,
    /// ntfy's 1 to 5 scale: 4 is "high", 3 the default.
    pub priority: u8,
}

/// The states a push is sent for; `working` and `none` never are.
pub fn is_pushable(state: AgentState) -> bool {
    matches!(
        state,
        AgentState::Waiting | AgentState::Failed | AgentState::Done
    )
}

/// The cards that changed to a pushable state since `previous`, sorted by
/// card id so the order is stable. No previous map means this is the first
/// look (startup, or the setting just turned on) and nothing is a change. A
/// card missing from the previous map had no state, so one that appears
/// already waiting counts.
pub fn changes(
    previous: Option<&BTreeMap<String, AgentState>>,
    current: &BTreeMap<String, AgentState>,
) -> Vec<Change> {
    let Some(previous) = previous else {
        return vec![];
    };
    current
        .iter()
        .filter(|(id, state)| {
            is_pushable(**state)
                && previous.get(*id).copied().unwrap_or(AgentState::None) != **state
        })
        .map(|(id, state)| Change {
            card_id: id.clone(),
            state: *state,
        })
        .collect()
}

/// `#<number> <title>`, or `card <number>` when the title is empty; the body
/// says the state in words.
pub fn message(card_number: u32, card_title: &str, state: AgentState) -> Push {
    let title = card_title.trim();
    let title = if title.is_empty() {
        format!("card {card_number}")
    } else {
        format!("#{card_number} {title}")
    };
    let (message, priority) = match state {
        AgentState::Waiting => ("Waiting on you", 4),
        AgentState::Failed => ("Failed", 4),
        _ => ("Done", 3),
    };
    Push {
        title,
        message: message.into(),
        priority,
    }
}

/// Letters, digits, `_` and `-`, one to 64 of them. A slash or a space would
/// change the URL ntfy reads the topic from.
pub fn valid_topic(topic: &str) -> bool {
    !topic.is_empty()
        && topic.len() <= TOPIC_MAX
        && topic
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// What the topic file holds: the text trimmed (an editor adds a newline).
/// Empty means push is off (`Ok(None)`), an invalid topic is an error the
/// caller logs once.
pub fn parse_topic_file(text: &str) -> Result<Option<String>, String> {
    let topic = text.trim();
    if topic.is_empty() {
        return Ok(None);
    }
    if valid_topic(topic) {
        Ok(Some(topic.to_string()))
    } else {
        Err(format!(
            "{TOPIC_FILE}: a topic is letters, digits, _ and -, at most {TOPIC_MAX}"
        ))
    }
}

/// The file of relay destinations, one `<url> <id>` per line.
pub const DEVICES_FILE: &str = "push-devices";

/// One phone registered with a push relay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub url: String,
    pub id: String,
}

/// The same characters and length as a topic: the id ends up in a JSON body
/// and in the log, so nothing else is let through.
pub fn valid_device_id(id: &str) -> bool {
    valid_topic(id)
}

/// Blank lines are skipped and each line is trimmed (CRLF included). Any other
/// line that is not `https://... <id>` makes the whole file invalid, so a typo
/// is reported once instead of silently dropping a phone.
pub fn parse_devices_file(text: &str) -> Result<Vec<Device>, String> {
    let mut devices = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let bad = || format!("{DEVICES_FILE}: each line is an https:// relay url and a push id");
        let mut words = line.split_whitespace();
        let (Some(url), Some(id), None) = (words.next(), words.next(), words.next()) else {
            return Err(bad());
        };
        if !url.starts_with("https://") || !valid_device_id(id) {
            return Err(bad());
        }
        devices.push(Device {
            url: url.to_string(),
            id: id.to_string(),
        });
    }
    Ok(devices)
}

/// One line per device, each ending in a newline.
pub fn devices_file_text(devices: &[Device]) -> String {
    devices
        .iter()
        .map(|d| format!("{} {}\n", d.url, d.id))
        .collect()
}

/// A device with an id already in the list replaces that line (a phone that
/// registers again, maybe with a new relay), keeping its place.
pub fn add_device(devices: &[Device], device: Device) -> Vec<Device> {
    let mut out = devices.to_vec();
    match out.iter_mut().find(|d| d.id == device.id) {
        Some(slot) => *slot = device,
        None => out.push(device),
    }
    out
}

pub fn remove_device(devices: &[Device], id: &str) -> Vec<Device> {
    devices.iter().filter(|d| d.id != id).cloned().collect()
}

/// Drops every device the relay reported as gone.
pub fn remove_gone(devices: &[Device], gone: &[String]) -> Vec<Device> {
    devices
        .iter()
        .filter(|d| !gone.contains(&d.id))
        .cloned()
        .collect()
}

/// The body to POST to the server's root.
pub fn json_body(topic: &str, push: &Push) -> String {
    json!({
        "topic": topic,
        "title": push.title,
        "message": push.message,
        "priority": push.priority,
    })
    .to_string()
}

/// With `notify.skipWhenFocused` and the window in front, nothing is sent:
/// you are at the Mac and can see the card colours. Any other app in front
/// still pushes.
pub fn should_send(skip_when_focused: bool, window_focused: bool) -> bool {
    !(skip_when_focused && window_focused)
}

#[cfg(test)]
mod tests {
    use super::*;
    use AgentState::{Done, Failed, Waiting, Working};

    fn map(rows: &[(&str, AgentState)]) -> BTreeMap<String, AgentState> {
        rows.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn the_first_look_reports_nothing() {
        assert!(changes(None, &map(&[("a", Waiting), ("b", Done)])).is_empty());
    }

    #[test]
    fn waiting_failed_and_done_are_reported_in_card_id_order() {
        let before = map(&[("a", Working), ("b", Working), ("c", Working)]);
        let after = map(&[("c", Done), ("a", Waiting), ("b", Failed)]);
        let got = changes(Some(&before), &after);
        let ids: Vec<_> = got.iter().map(|c| c.card_id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        let states: Vec<_> = got.iter().map(|c| c.state).collect();
        assert_eq!(states, [Waiting, Failed, Done]);
    }

    #[test]
    fn an_unchanged_state_and_working_report_nothing() {
        let before = map(&[("a", Waiting), ("b", Done), ("c", Failed)]);
        assert!(changes(Some(&before), &before).is_empty());
        assert!(changes(Some(&map(&[("a", Done)])), &map(&[("a", Working)])).is_empty());
    }

    #[test]
    fn a_state_that_went_away_reports_nothing() {
        assert!(changes(Some(&map(&[("a", Done)])), &map(&[])).is_empty());
        assert!(changes(Some(&map(&[("a", Done)])), &map(&[("a", AgentState::None)])).is_empty());
    }

    #[test]
    fn a_card_new_to_the_map_and_already_failed_is_reported() {
        let got = changes(Some(&map(&[])), &map(&[("a", Failed)]));
        assert_eq!(
            got,
            [Change {
                card_id: "a".into(),
                state: Failed
            }]
        );
    }

    #[test]
    fn waiting_then_failed_is_one_change_for_the_failure() {
        let got = changes(Some(&map(&[("a", Waiting)])), &map(&[("a", Failed)]));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].state, Failed);
    }

    #[test]
    fn the_message_names_the_card_and_the_state() {
        let p = message(3, "build", Waiting);
        assert_eq!(p.title, "#3 build");
        assert_eq!(p.message, "Waiting on you");
        assert_eq!(p.priority, 4);
        let p = message(3, "build", Failed);
        assert_eq!((p.message.as_str(), p.priority), ("Failed", 4));
        let p = message(3, "build", Done);
        assert_eq!((p.message.as_str(), p.priority), ("Done", 3));
    }

    #[test]
    fn an_empty_title_falls_back_to_the_number() {
        assert_eq!(message(7, "", Done).title, "card 7");
        assert_eq!(message(7, "  ", Done).title, "card 7");
    }

    #[test]
    fn a_topic_is_letters_digits_underscore_and_dash() {
        assert!(valid_topic("abc_123-x"));
        assert!(valid_topic(&"a".repeat(64)));
        assert!(!valid_topic(""));
        assert!(!valid_topic("a/b"));
        assert!(!valid_topic("a b"));
        assert!(!valid_topic(&"a".repeat(65)));
        assert!(!valid_topic("kahve\u{131}"));
    }

    #[test]
    fn the_topic_file_is_trimmed_and_empty_means_off() {
        assert_eq!(parse_topic_file("abc-1\n"), Ok(Some("abc-1".into())));
        assert_eq!(parse_topic_file("  abc-1  \r\n"), Ok(Some("abc-1".into())));
        assert_eq!(parse_topic_file(""), Ok(None));
        assert_eq!(parse_topic_file(" \n"), Ok(None));
        assert!(parse_topic_file("a/b").is_err());
        assert!(parse_topic_file("two words").is_err());
    }

    #[test]
    fn the_body_escapes_quotes_and_keeps_utf8() {
        let push = Push {
            title: "#1 say \"hi\" \u{15f}\u{131}".into(),
            message: "Done".into(),
            priority: 3,
        };
        let body = json_body("t-1", &push);
        let back: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(back["topic"], "t-1");
        assert_eq!(back["title"], "#1 say \"hi\" \u{15f}\u{131}");
        assert_eq!(back["message"], "Done");
        assert_eq!(back["priority"], 3);
    }

    #[test]
    fn the_focus_rule_is_a_function_of_the_setting_and_the_window() {
        assert!(!should_send(true, true));
        assert!(should_send(true, false));
        assert!(should_send(false, true));
        assert!(should_send(false, false));
    }

    fn dev(url: &str, id: &str) -> Device {
        Device {
            url: url.into(),
            id: id.into(),
        }
    }

    #[test]
    fn devices_file_parses_url_and_id_per_line() {
        assert_eq!(
            parse_devices_file("https://r.example/ abc_1\n"),
            Ok(vec![dev("https://r.example/", "abc_1")])
        );
    }

    #[test]
    fn devices_file_skips_blank_lines_and_trims_crlf() {
        assert_eq!(
            parse_devices_file("\r\n https://r.example/ abc \r\n"),
            Ok(vec![dev("https://r.example/", "abc")])
        );
        assert_eq!(parse_devices_file(""), Ok(vec![]));
    }

    #[test]
    fn devices_file_rejects_http_url_and_bad_id() {
        assert!(parse_devices_file("http://x a").is_err());
        assert!(parse_devices_file("https://x a/b").is_err());
        assert!(parse_devices_file(&format!("https://x {}", "a".repeat(65))).is_err());
        // One bad line makes the whole file invalid.
        assert!(parse_devices_file("https://x a\nhttp://y b\n").is_err());
    }

    #[test]
    fn devices_file_rejects_a_line_without_two_words() {
        assert!(parse_devices_file("https://x\n").is_err());
        assert!(parse_devices_file("https://x a extra\n").is_err());
    }

    #[test]
    fn add_device_replaces_the_same_id() {
        let list = vec![dev("https://a/", "one"), dev("https://b/", "two")];
        assert_eq!(
            add_device(&list, dev("https://c/", "one")),
            [dev("https://c/", "one"), dev("https://b/", "two")]
        );
        assert_eq!(
            add_device(&list, dev("https://c/", "three")),
            [
                dev("https://a/", "one"),
                dev("https://b/", "two"),
                dev("https://c/", "three")
            ]
        );
    }

    #[test]
    fn remove_device_and_remove_gone() {
        let list = vec![
            dev("https://a/", "one"),
            dev("https://a/", "two"),
            dev("https://a/", "three"),
        ];
        assert_eq!(
            remove_device(&list, "two"),
            [dev("https://a/", "one"), dev("https://a/", "three")]
        );
        assert_eq!(remove_device(&list, "nope"), list);
        assert_eq!(
            remove_gone(&list, &["one".to_string(), "three".to_string()]),
            [dev("https://a/", "two")]
        );
        assert_eq!(remove_gone(&list, &[]), list);
    }

    #[test]
    fn devices_file_text_round_trips() {
        let list = vec![dev("https://a/", "one"), dev("https://b/x", "two")];
        let text = devices_file_text(&list);
        assert_eq!(text, "https://a/ one\nhttps://b/x two\n");
        assert_eq!(parse_devices_file(&text), Ok(list));
        assert_eq!(devices_file_text(&[]), "");
    }
}
