//! The "Start here" card a first launch opens beside the first terminal:
//! a Markdown file rendered from the app's own keymap and gesture list, so
//! the keys it teaches are the keys the app has, and which coding agents
//! report their state yet, with the one command for each that does not.
//!
//! infiniterm is not a terminal people already know how to drive: the
//! canvas, Cmd as the app's modifier and the coloured borders all need
//! saying once. Said with a card rather than a tour overlay, because using
//! the canvas is the lesson (Ekin chose this, 2026-10-01).
//!
//! `render` is pure and tested here. `agents_on_this_mac` reads the disk
//! (each agent's config directory, and whether infiniterm's hook is already
//! in it). The ui writes the file at every launch (`runtime.rs`, beside the
//! settings defaults) so the agent list stays current; `Model::seed_first_card`
//! opens it on a first launch and `help.welcome` opens it any time.
use crate::keymap::Keymap;
use crate::paths::app_support_dir;
use crate::shortcuts::format_chord;
use std::path::{Path, PathBuf};

/// Where the file lives: under the data dir, so a scratch instance has its
/// own, and a name that reads well on the card's label.
pub fn welcome_path() -> PathBuf {
    app_support_dir().join("welcome").join("Start here.md")
}

/// One coding agent, as the welcome card describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub name: &'static str,
    /// The `ift` verb that wires it, e.g. `install-claude-hooks`.
    pub install: &'static str,
    /// Its config directory exists: the agent is installed or was.
    pub present: bool,
    /// infiniterm's hook is already in its config.
    pub wired: bool,
}

/// The keys the card teaches, by command id, in the order a newcomer
/// needs them. A command the keymap no longer binds is left out rather
/// than shown with a stale chord.
const KEYS: &[(&str, &str)] = &[
    ("card.new.terminal", "a new terminal card"),
    ("card.close", "close the card"),
    ("canvas.zoom.fitAll", "fit every card in the window"),
    ("canvas.zoom.fitCard", "fit the focused card"),
    (
        "focus.move.right",
        "the card to the right (any arrow works)",
    ),
    ("app.palette", "every command, by name"),
    ("app.shortcuts", "every key and gesture, searchable"),
];

pub fn render(keymap: &Keymap, agents: &[Agent]) -> String {
    let mut out = String::from(
        "# Start here\n\n\
         Every terminal is a card on one canvas. Cards stay where you put them, and a new one takes the next free slot of a grid that starts at the top left.\n\n\
         Cmd is the app's key. Everything else (Ctrl, Alt, bare keys) goes to the terminal, so vim, tmux and your shell work as usual.\n\n\
         Click into a file in an editor card to type in it: the keyboard is the file's until you press Escape twice. This card is read-only, so clicking it never takes the keys; click a line and press `Cmd C` to copy it.\n\n\
         ## Keys to start with\n\n",
    );
    for (id, does) in KEYS {
        if let Some((chord, _)) = keymap.iter().find(|(_, bound)| bound == id) {
            out.push_str(&format!("- `{}`: {does}\n", format_chord(chord)));
        }
    }
    out.push_str(
        "\n## Moving around with the mouse\n\n\
         - `Cmd` + scroll, or a pinch: zoom\n\
         - `Cmd` + drag, or a middle-button drag: pan\n\
         - drag on empty canvas: select the cards it touches\n\
         - drag a card by its top edge or its label: move it\n\
         - double-click empty canvas: fit everything\n\n\
         ## The border colours\n\n\
         - violet: working (an agent mid-turn, or a command running 5 seconds or more)\n\
         - yellow: waiting on you (a permission prompt, a question)\n\
         - red: failed\n\
         - green: done, until you have looked at it\n\n\
         A zsh started in a card reports its commands without any setup. Coding agents report through hooks:\n\n",
    );
    let shown: Vec<&Agent> = agents.iter().filter(|a| a.present).collect();
    if shown.is_empty() {
        out.push_str("No coding agent found on this Mac. When you install one, wire it from any terminal:\n\n");
        for a in agents {
            out.push_str(&format!("- {}: `ift {}`\n", a.name, a.install));
        }
    } else {
        for a in shown {
            if a.wired {
                out.push_str(&format!("- {}: wired\n", a.name));
            } else {
                out.push_str(&format!(
                    "- {}: run `ift {}`, then start a new session\n",
                    a.name, a.install
                ));
            }
        }
    }
    out.push_str(
        "\n## When you are done\n\n\
         Close this card with `Cmd W`. \"Help: open the welcome card\" in the palette brings it back. \
         The full docs are at https://infiniterm.app.\n\n\
         This file is rewritten at every launch, so edits to it do not last.\n",
    );
    out
}

/// The four agents with hooks, looked up under `home`. `xdg_config` and
/// `codex_home` are the overrides those agents honour, when set.
pub fn agents_on_this_mac(
    home: &Path,
    xdg_config: Option<&Path>,
    codex_home: Option<&Path>,
) -> Vec<Agent> {
    let has = |path: PathBuf, needle: &str| {
        std::fs::read_to_string(path).is_ok_and(|t| t.contains(needle))
    };
    let claude = home.join(".claude");
    let codex = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".codex"));
    let opencode = xdg_config
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".config"))
        .join("opencode");
    let pi = home.join(".pi");
    vec![
        Agent {
            name: "Claude Code",
            install: "install-claude-hooks",
            present: claude.is_dir(),
            wired: has(claude.join("settings.json"), "infiniterm-hook"),
        },
        Agent {
            name: "Codex",
            install: "install-codex-hooks",
            present: codex.is_dir(),
            wired: has(codex.join("hooks.json"), "infiniterm-hook"),
        },
        Agent {
            name: "OpenCode",
            install: "install-opencode-hooks",
            present: opencode.is_dir(),
            wired: opencode.join("plugins").join("infiniterm.js").is_file(),
        },
        Agent {
            name: "Pi",
            install: "install-pi-hooks",
            present: pi.is_dir(),
            wired: pi
                .join("agent")
                .join("extensions")
                .join("infiniterm.ts")
                .is_file(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::default_keymap;

    fn agent(name: &'static str, present: bool, wired: bool) -> Agent {
        Agent {
            name,
            install: "install-x-hooks",
            present,
            wired,
        }
    }

    #[test]
    fn the_keys_come_from_the_keymap() {
        let text = render(&default_keymap(), &[]);
        let km = default_keymap();
        let (chord, _) = km.iter().find(|(_, id)| id == "card.new.terminal").unwrap();
        assert!(text.contains(&format!("`{}`: a new terminal card", format_chord(chord))));
    }

    // A key that stops being bound drops off the card instead of being
    // taught wrong.
    #[test]
    fn an_unbound_command_is_left_out() {
        let km: Keymap = vec![("cmd+t".into(), "card.new.terminal".into())];
        let text = render(&km, &[]);
        assert!(text.contains("a new terminal card"));
        assert!(!text.contains("close the card"));
    }

    #[test]
    fn every_key_on_the_card_is_a_registered_command() {
        let mut r = crate::commands::CommandRegistry::new(|_| {});
        crate::model::register::register_commands(&mut r);
        for (id, _) in KEYS {
            assert!(r.get(id).is_some(), "{id} is not a command");
        }
        // And the defaults bind all of them, so a fresh install shows each.
        let text = render(&default_keymap(), &[]);
        for (_, does) in KEYS {
            assert!(text.contains(does), "{does} missing from the card");
        }
    }

    #[test]
    fn agents_found_are_listed_with_what_to_run() {
        let text = render(
            &default_keymap(),
            &[
                agent("Claude Code", true, true),
                agent("Codex", true, false),
                agent("Pi", false, false),
            ],
        );
        assert!(text.contains("- Claude Code: wired"));
        assert!(text.contains("- Codex: run `ift install-x-hooks`, then start a new session"));
        assert!(
            !text.contains("- Pi"),
            "an agent not on this Mac is not listed"
        );
    }

    #[test]
    fn with_no_agent_found_every_installer_is_listed() {
        let text = render(
            &default_keymap(),
            &[
                agent("Claude Code", false, false),
                agent("Pi", false, false),
            ],
        );
        assert!(text.contains("No coding agent found"));
        assert!(text.contains("- Claude Code: `ift install-x-hooks`"));
        assert!(text.contains("- Pi: `ift install-x-hooks`"));
    }

    #[test]
    fn agents_are_found_by_their_config_dirs() {
        let home = std::env::temp_dir().join(format!("ift-welcome-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"hooks":{"Stop":[{"command":"/x/infiniterm-hook Stop"}]}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        let found = agents_on_this_mac(&home, None, None);
        let by = |n: &str| found.iter().find(|a| a.name == n).unwrap().clone();
        assert!(by("Claude Code").present && by("Claude Code").wired);
        assert!(by("Codex").present && !by("Codex").wired);
        assert!(!by("OpenCode").present);
        assert!(!by("Pi").present);
        let _ = std::fs::remove_dir_all(&home);
    }
}
