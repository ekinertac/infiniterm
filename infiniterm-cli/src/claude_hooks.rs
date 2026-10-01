//! Wiring `infiniterm-hook` into `~/.claude/settings.json`, and into Codex's
//! `~/.codex/hooks.json`, which has the same shape and the same event names
//! (Codex 0.145+, issue #26).
//!
//! This replaces eight hand-written JSON entries pointing at an absolute path,
//! which is the one piece of setup infiniterm still asks for by hand — and the
//! one where a mistake is silent, because a wrong path just means the borders
//! never light up.
//!
//! Three rules, and the file itself is why:
//!
//! 1. **Merge, never replace.** A real settings.json holds permissions, env, a
//!    statusLine, enabled plugins and other people's hooks. Writing ours over it
//!    would be unforgivable, and the file is not ours to own.
//! 2. **Idempotent.** Running twice adds nothing the second time, so it is safe
//!    in a dotfiles bootstrap. An entry that already points at infiniterm-hook is
//!    UPDATED rather than duplicated, which also repairs the path after the
//!    binary is rebuilt somewhere else.
//! 3. **Say what changed.** The caller prints the events touched and the file
//!    path, so it is obvious what to undo.
//!
//! The events are the eight the card state machine reads; see
//! `src/lib/agentState.ts` for what each one means to a border.

use serde_json::{json, Map, Value};

/// The hook events infiniterm listens for.
///
/// Kept in the order the state machine documents them rather than alphabetically,
/// so this list reads against agentState.ts.
pub const EVENTS: [&str; 8] = [
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Stop",
    "StopFailure",
    "Notification",
    "SessionStart",
    "SessionEnd",
];

/// The Codex hook events infiniterm reads: Claude's names, plus
/// `PermissionRequest` (its approval prompt). Codex has no failure hook and
/// no `Notification`; a turn that errors never reaches `Stop`.
pub const CODEX_EVENTS: [&str; 7] = [
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Stop",
    "SessionStart",
    "SessionEnd",
];

/// True when a hook entry runs our binary, whatever path it was installed at.
///
/// Two ways to be ours, and both are needed:
///
/// - the program is EXACTLY the binary being installed, which is what makes a
///   second run idempotent whatever the binary is called;
/// - or its file name contains `infiniterm-hook`, which recognises an entry
///   written by hand or by an older install at a different path, so it is
///   corrected instead of duplicated beside a new one.
///
/// Only the first token is examined. Matching anywhere in the line would claim
/// somebody else's hook that merely mentions the word — a wrapper script that
/// logs what it is about to run, say.
fn is_ours(command: &str, hook_binary: &str) -> bool {
    let program = command.split_whitespace().next().unwrap_or("");
    if program == hook_binary {
        return true;
    }
    let name = program.rsplit('/').next().unwrap_or(program);
    name.contains("infiniterm-hook")
}

/// Adds or repairs infiniterm's hooks, returning the events that changed.
///
/// An empty return means the file already said the right thing, and the caller
/// should leave it alone rather than rewriting it byte for byte.
/// Claude Code's events, no agent name; the tests' shorthand for
/// `install_events`.
#[cfg(test)]
pub fn install(settings: &mut Value, hook_binary: &str) -> Vec<String> {
    install_events(settings, hook_binary, &EVENTS, None)
}

/// `install` for any agent with Claude's hook file shape: `events` to wire,
/// and the `agent` name passed to the hook binary after the event, so the
/// card knows whose session it is (none for Claude Code).
pub fn install_events(
    settings: &mut Value,
    hook_binary: &str,
    events: &[&str],
    agent: Option<&str>,
) -> Vec<String> {
    let mut changed = Vec::new();

    // `settings.json` may be `{}` on a first run, or may have no hooks key.
    if !settings.is_object() {
        *settings = Value::Object(Map::new());
    }
    let root = settings.as_object_mut().expect("just made it an object");
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    if !hooks.is_object() {
        *hooks = Value::Object(Map::new());
    }
    let hooks = hooks.as_object_mut().expect("just made it an object");

    for &event in events {
        let want = match agent {
            Some(a) => format!("{hook_binary} {event} {a}"),
            None => format!("{hook_binary} {event}"),
        };
        let list = hooks
            .entry(event.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if !list.is_array() {
            *list = Value::Array(Vec::new());
        }
        let list = list.as_array_mut().expect("just made it an array");

        // An existing entry of ours is corrected in place. Only the command is
        // touched: a matcher somebody added stays exactly as they wrote it.
        let mut found = false;
        for group in list.iter_mut() {
            let Some(inner) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                continue;
            };
            for hook in inner.iter_mut() {
                let is_match = hook
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|c| is_ours(c, hook_binary));
                if !is_match {
                    continue;
                }
                found = true;
                if hook.get("command").and_then(Value::as_str) != Some(want.as_str()) {
                    hook["command"] = Value::String(want.clone());
                    changed.push(event.to_string());
                }
            }
        }

        if !found {
            // Appended, not prepended: hooks run in order and somebody else's
            // was here first. Ours only reports, so it has no reason to go first.
            list.push(json!({ "hooks": [{ "type": "command", "command": want }] }));
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
            .flat_map(|g| g["hooks"].as_array().unwrap())
            .map(|h| h["command"].as_str().unwrap().to_string())
            .collect()
    }

    /// Codex takes the same file shape: its own events, each naming the
    /// agent after the event, so the card knows it is a Codex session.
    #[test]
    fn codex_gets_its_events_with_the_agent_named() {
        let mut s = json!({});
        let changed = install_events(&mut s, "/bin/ih", &CODEX_EVENTS, Some("codex"));
        assert_eq!(changed.len(), CODEX_EVENTS.len());
        assert_eq!(
            commands(&s, "PermissionRequest"),
            vec!["/bin/ih PermissionRequest codex".to_string()]
        );
        assert!(install_events(&mut s, "/bin/ih", &CODEX_EVENTS, Some("codex")).is_empty());
    }

    #[test]
    fn installs_every_event_into_an_empty_file() {
        let mut s = json!({});
        let changed = install(&mut s, "/bin/ih");
        assert_eq!(changed.len(), EVENTS.len());
        for event in EVENTS {
            assert_eq!(commands(&s, event), vec![format!("/bin/ih {event}")]);
        }
    }

    /// A real settings.json is full of things that are none of our business.
    #[test]
    fn leaves_every_other_setting_alone() {
        let mut s = json!({
            "env": { "FOO": "bar" },
            "permissions": { "allow": ["Bash"] },
            "statusLine": { "type": "command", "command": "x" },
        });
        install(&mut s, "/bin/ih");
        assert_eq!(s["env"]["FOO"], "bar");
        assert_eq!(s["permissions"]["allow"][0], "Bash");
        assert_eq!(s["statusLine"]["command"], "x");
    }

    #[test]
    fn keeps_other_peoples_hooks_on_the_same_event() {
        let mut s = json!({
            "hooks": { "Stop": [{ "matcher": "", "hooks": [
                { "type": "command", "command": "/somebody/else.sh" }
            ]}]}
        });
        install(&mut s, "/bin/ih");
        assert_eq!(
            commands(&s, "Stop"),
            vec!["/somebody/else.sh".to_string(), "/bin/ih Stop".to_string()]
        );
    }

    /// Matching anywhere in the line would claim a wrapper that merely logs the
    /// name of what it is about to run.
    #[test]
    fn does_not_claim_somebody_elses_hook_that_mentions_us() {
        let mut s = json!({
            "hooks": { "Stop": [{ "hooks": [
                { "type": "command", "command": "/their/wrapper.sh --label infiniterm-hook" }
            ]}]}
        });
        install(&mut s, "/bin/infiniterm-hook");
        assert_eq!(
            commands(&s, "Stop"),
            vec![
                "/their/wrapper.sh --label infiniterm-hook".to_string(),
                "/bin/infiniterm-hook Stop".to_string(),
            ]
        );
    }

    /// Safe in a dotfiles bootstrap that runs it on every shell. Idempotent
    /// whatever the binary is called, not only when it is named infiniterm-hook.
    #[test]
    fn a_second_run_changes_nothing() {
        let mut s = json!({});
        install(&mut s, "/bin/ih");
        let before = s.clone();
        assert!(install(&mut s, "/bin/ih").is_empty());
        assert_eq!(s, before);
    }

    /// Rebuilding the binary somewhere else should repair the wiring, not double it.
    #[test]
    fn a_moved_binary_is_corrected_in_place() {
        let mut s = json!({});
        install(&mut s, "/old/infiniterm-hook");
        let changed = install(&mut s, "/new/infiniterm-hook");
        assert_eq!(changed.len(), EVENTS.len());
        for event in EVENTS {
            assert_eq!(commands(&s, event), vec![format!("/new/infiniterm-hook {event}")]);
        }
    }

    /// Hand-written entries are ours too, and must not be duplicated beside.
    #[test]
    fn recognises_an_entry_written_by_hand() {
        let mut s = json!({
            "hooks": { "Stop": [{ "hooks": [
                { "type": "command", "command": "~/src/crates/infiniterm-hook/target/release/infiniterm-hook Stop" }
            ]}]}
        });
        install(&mut s, "/bin/infiniterm-hook");
        assert_eq!(commands(&s, "Stop"), vec!["/bin/infiniterm-hook Stop".to_string()]);
    }

    #[test]
    fn a_matcher_somebody_added_survives() {
        let mut s = json!({
            "hooks": { "Stop": [{ "matcher": "Bash", "hooks": [
                { "type": "command", "command": "/old/infiniterm-hook Stop" }
            ]}]}
        });
        install(&mut s, "/new/infiniterm-hook");
        assert_eq!(s["hooks"]["Stop"][0]["matcher"], "Bash");
    }

    /// A file that is not an object, or whose hooks key is junk, must not panic.
    #[test]
    fn survives_a_malformed_file() {
        let mut s = json!([1, 2, 3]);
        install(&mut s, "/bin/ih");
        assert_eq!(commands(&s, "Stop"), vec!["/bin/ih Stop".to_string()]);

        let mut s = json!({ "hooks": "nonsense" });
        install(&mut s, "/bin/ih");
        assert_eq!(commands(&s, "Stop"), vec!["/bin/ih Stop".to_string()]);

        let mut s = json!({ "hooks": { "Stop": 42 } });
        install(&mut s, "/bin/ih");
        assert_eq!(commands(&s, "Stop"), vec!["/bin/ih Stop".to_string()]);
    }
}
