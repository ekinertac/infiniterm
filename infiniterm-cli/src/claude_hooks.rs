//! Wiring `infiniterm-hook` into `~/.claude/settings.json`.
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

/// The name the hook binary is built under, which Windows spells with an
/// extension. `ift` looks for this beside itself and on PATH.
pub const HOOK_BINARY: &str = if cfg!(windows) {
    "infiniterm-hook.exe"
} else {
    "infiniterm-hook"
};

/// The command Claude Code runs for one event.
///
/// Quoted when the path has a space in it. The command is handed to a shell,
/// so `C:\Program Files\infiniterm\infiniterm-hook.exe Stop` would try to
/// run `C:\Program`; a Mac path for this is under /Applications or a
/// checkout and rarely has one, and Windows puts things in Program Files.
pub fn hook_command(hook_binary: &str, event: &str) -> String {
    if hook_binary.contains(' ') {
        format!("\"{hook_binary}\" {event}")
    } else {
        format!("{hook_binary} {event}")
    }
}

/// The program a hook command runs, with its quotes taken off.
fn program_of(command: &str) -> &str {
    let text = command.trim_start();
    match text.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(""),
        None => text.split_whitespace().next().unwrap_or(""),
    }
}

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
    let program = program_of(command);
    if program == hook_binary {
        return true;
    }
    // Either separator: an entry written on one platform can be read on the
    // other, and a settings.json travels between machines.
    infiniterm_core::paths::base_name(program).contains("infiniterm-hook")
}

/// Adds or repairs infiniterm's hooks, returning the events that changed.
///
/// An empty return means the file already said the right thing, and the caller
/// should leave it alone rather than rewriting it byte for byte.
pub fn install(settings: &mut Value, hook_binary: &str) -> Vec<String> {
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

    for event in EVENTS {
        let want = hook_command(hook_binary, event);
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

    // A hook command is handed to a shell, so a path with a space in it has
    // to arrive quoted or the shell runs only the first word. Windows puts
    // programs in Program Files; this never came up on a Mac.
    #[test]
    fn a_path_with_a_space_is_quoted_and_still_recognised_as_ours() {
        let win = r"C:\Program Files\infiniterm\infiniterm-hook.exe";
        let cmd = hook_command(win, "Stop");
        assert_eq!(cmd, format!("\"{win}\" Stop"));
        assert_eq!(program_of(&cmd), win);
        assert!(is_ours(&cmd, win), "its own entry must be idempotent");
        // And an entry an older install wrote at another path.
        assert!(is_ours(&cmd, "/somewhere/else/infiniterm-hook"));
    }

    #[test]
    fn a_path_without_a_space_is_left_bare() {
        let unix = "/Applications/infiniterm.app/Contents/MacOS/infiniterm-hook";
        assert_eq!(hook_command(unix, "Stop"), format!("{unix} Stop"));
        assert_eq!(program_of("/a/b Stop"), "/a/b");
    }

    // The base name is taken with either separator, because a settings.json
    // travels between machines and an entry written on one is read on the
    // other.
    #[test]
    fn an_entry_is_recognised_whichever_platform_wrote_it() {
        assert!(is_ours(r"C:\tools\infiniterm-hook.exe Stop", "/usr/bin/other"));
        assert!(is_ours("/usr/local/bin/infiniterm-hook Stop", r"C:\x\other.exe"));
        // Somebody else's hook that merely mentions the word is not ours.
        assert!(!is_ours("/usr/bin/logger ran infiniterm-hook", "/x/infiniterm-hook"));
    }
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
