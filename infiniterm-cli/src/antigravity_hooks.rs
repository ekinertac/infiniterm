//! Wiring infiniterm's Antigravity adapter into `~/.gemini/config/hooks.json`.
//!
//! Antigravity's shape: top-level keys are hook NAMES, each mapping event
//! names to handlers. PreToolUse and PostToolUse take `{matcher, hooks}`
//! entries; PreInvocation and Stop take handlers directly. Everything is ours
//! under the one name `infiniterm`; other names in the file are never touched.
//! The adapter script (`adapters/antigravity-hook.sh`) translates the events
//! into the vocabulary `agent_state.rs` already understands.

use serde_json::{json, Map, Value};

/// The hook name every infiniterm entry lives under.
pub const HOOK_NAME: &str = "infiniterm";

/// Events wired. PostInvocation is left out: Stop already ends the turn.
pub const EVENTS: [&str; 4] = ["PreInvocation", "PreToolUse", "PostToolUse", "Stop"];

/// Seconds Antigravity waits for a handler (it defaults to 30). The adapter
/// answers in well under one.
const TIMEOUT_SECS: u64 = 5;

fn handler(command_with_event: &str) -> Value {
    json!({ "type": "command", "command": command_with_event, "timeout": TIMEOUT_SECS })
}

/// What one event's value must be: tool events carry a matcher for all tools.
fn wanted(event: &str, script: &str) -> Value {
    let h = handler(&format!("{script} {event}"));
    match event {
        "PreToolUse" | "PostToolUse" => json!([{ "matcher": "*", "hooks": [h] }]),
        _ => json!([h]),
    }
}

/// Merges infiniterm's hooks into `settings`. Returns the events that were
/// added or changed.
pub fn install(settings: &mut Value, script: &str) -> Vec<String> {
    if !settings.is_object() {
        *settings = Value::Object(Map::new());
    }
    let root = settings.as_object_mut().expect("just made it an object");
    let ours = root
        .entry(HOOK_NAME)
        .or_insert_with(|| Value::Object(Map::new()));
    if !ours.is_object() {
        *ours = Value::Object(Map::new());
    }
    let ours = ours.as_object_mut().expect("just made it an object");
    let mut changed = Vec::new();
    for &event in &EVENTS {
        let want = wanted(event, script);
        if ours.get(event) != Some(&want) {
            ours.insert(event.to_string(), want);
            changed.push(event.to_string());
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "/Users/me/.gemini/hooks/infiniterm-antigravity-hook.sh";

    #[test]
    fn an_empty_file_gets_every_event_under_our_name() {
        let mut v = json!({});
        let changed = install(&mut v, SCRIPT);
        assert_eq!(changed, EVENTS);
        let ours = &v[HOOK_NAME];
        assert_eq!(
            ours["PreInvocation"][0]["command"],
            format!("{SCRIPT} PreInvocation")
        );
        assert_eq!(ours["PreToolUse"][0]["matcher"], "*");
        assert_eq!(
            ours["PostToolUse"][0]["hooks"][0]["command"],
            format!("{SCRIPT} PostToolUse")
        );
        assert_eq!(ours["Stop"][0]["timeout"], TIMEOUT_SECS);
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let mut v = json!({});
        install(&mut v, SCRIPT);
        let once = v.clone();
        assert!(install(&mut v, SCRIPT).is_empty());
        assert_eq!(v, once);
    }

    #[test]
    fn other_hooks_in_the_file_are_left_alone() {
        let mut v = json!({
            "my-linter": { "PostToolUse": [{ "matcher": "run_command", "hooks": [{ "command": "./lint.sh" }] }] }
        });
        let linter = v["my-linter"].clone();
        install(&mut v, SCRIPT);
        assert_eq!(v["my-linter"], linter);
        assert!(v[HOOK_NAME].is_object());
    }

    #[test]
    fn a_moved_script_is_repaired_in_place() {
        let mut v = json!({});
        install(&mut v, "/old/infiniterm-antigravity-hook.sh");
        let changed = install(&mut v, SCRIPT);
        assert_eq!(changed, EVENTS);
        assert_eq!(
            v[HOOK_NAME]["Stop"][0]["command"],
            format!("{SCRIPT} Stop")
        );
    }

    #[test]
    fn a_file_that_is_not_an_object_is_replaced_by_one() {
        let mut v = json!([1, 2]);
        install(&mut v, SCRIPT);
        assert!(v[HOOK_NAME]["Stop"].is_array());
    }
}
