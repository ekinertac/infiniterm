//! Wiring infiniterm's Cursor adapter into `~/.cursor/hooks.json`.
//!
//! Cursor's format is not Claude/Codex's: `version` 1 and a flat list of
//! `{ "command": "..." }` entries per event. The adapter script translates
//! Cursor's names into the vocabulary `agent_state.rs` already understands.

use serde_json::{json, Map, Value};

/// Cursor hook events infiniterm listens to, via the adapter script.
pub const EVENTS: [&str; 7] = [
    "sessionStart",
    "sessionEnd",
    "beforeSubmitPrompt",
    "preToolUse",
    "postToolUse",
    "postToolUseFailure",
    "stop",
];

/// Command written into hooks.json (user hooks run from `~/.cursor/`).
pub const HOOK_COMMAND: &str = "./hooks/infiniterm-cursor-hook.sh";

fn is_ours(command: &str) -> bool {
    command.contains("infiniterm-cursor-hook")
}

/// Merges infiniterm's Cursor hooks. Returns Cursor event names that changed.
pub fn install(settings: &mut Value) -> Vec<String> {
    let mut changed = Vec::new();

    if !settings.is_object() {
        *settings = Value::Object(Map::new());
    }
    let root = settings.as_object_mut().expect("just made it an object");
    root.entry("version").or_insert(json!(1));

    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    if !hooks.is_object() {
        *hooks = Value::Object(Map::new());
    }
    let hooks = hooks.as_object_mut().expect("just made it an object");

    for &event in &EVENTS {
        let list = hooks
            .entry(event.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if !list.is_array() {
            *list = Value::Array(Vec::new());
        }
        let list = list.as_array_mut().expect("just made it an array");

        let mut found = false;
        for entry in list.iter_mut() {
            let Some(cmd) = entry.get("command").and_then(Value::as_str) else {
                continue;
            };
            if !is_ours(cmd) {
                continue;
            }
            found = true;
            if cmd != HOOK_COMMAND {
                entry["command"] = Value::String(HOOK_COMMAND.to_string());
                changed.push(event.to_string());
            }
        }

        if !found {
            list.push(json!({ "command": HOOK_COMMAND }));
            changed.push(event.to_string());
        }
    }

    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(settings: &Value, event: &str) -> Vec<String> {
        settings["hooks"][event]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["command"].as_str().map(String::from))
            .collect()
    }

    #[test]
    fn installs_every_event_into_an_empty_file() {
        let mut s = json!({});
        let changed = install(&mut s);
        assert_eq!(changed.len(), EVENTS.len());
        assert_eq!(s["version"], 1);
        for event in EVENTS {
            assert_eq!(
                commands(&s, event),
                vec![HOOK_COMMAND.to_string()]
            );
        }
    }

    #[test]
    fn keeps_other_peoples_hooks_on_the_same_event() {
        let mut s = json!({
            "version": 1,
            "hooks": {
                "stop": [{ "command": "./hooks/audit.sh" }]
            }
        });
        install(&mut s);
        assert_eq!(
            commands(&s, "stop"),
            vec!["./hooks/audit.sh".to_string(), HOOK_COMMAND.to_string()]
        );
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let mut s = json!({});
        install(&mut s);
        let before = s.clone();
        assert!(install(&mut s).is_empty());
        assert_eq!(s, before);
    }

    #[test]
    fn repairs_an_old_absolute_path_in_place() {
        let mut s = json!({
            "version": 1,
            "hooks": {
                "stop": [{ "command": "/old/infiniterm-cursor-hook.sh" }]
            }
        });
        let changed = install(&mut s);
        assert_eq!(changed.len(), EVENTS.len());
        assert_eq!(commands(&s, "stop"), vec![HOOK_COMMAND.to_string()]);
    }
}
