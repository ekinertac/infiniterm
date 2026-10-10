//! The documentation site's reference pages (keys, commands, settings),
//! rendered as Markdown from the app's own tables.
//!
//! Generated rather than written because a hand-kept list of keys or
//! settings is wrong within a week; the tables here are the ones the app
//! runs on, each already held complete by its own test (`settings_doc.rs`
//! for settings, `keymap.rs` and `shortcuts.rs` for bindings).
//!
//! Called by `examples/site_reference.rs`, which the site's `npm run build`
//! and `npm run dev` run first (site/package.json), so the pages are
//! rebuilt from the code on every build and never committed. Reads
//! `register_commands`, `DEFAULT_KEYMAP`, `SETTINGS_DOC` and
//! `default_config`; reuses `shortcuts::shortcut_sections` for grouping.
//!
//! Commands whose id starts `dev.` are left out: they exist in development
//! builds only. The locked-card tables are functions, not lists
//! (`editor_keys::lock_override` is a `match`), so they are read by probing
//! every chord the keymap can spell; a chord outside `probe_chords` would be
//! missed, which the test below guards for the ones that exist today.
use crate::browser_keys;
use crate::commands::CommandRegistry;
use crate::config::{default_config, flatten};
use crate::editor_keys;
use crate::keymap::{Keymap, DEFAULT_KEYMAP};
use crate::model::register::register_commands;
use crate::model::Model;
use crate::settings_doc::SETTINGS_DOC;
use crate::shortcuts::{chord_keys, shortcut_sections, GESTURES};
use serde_json::Value;

/// A generated page: its file name under `reference/` and its Markdown.
pub struct Page {
    pub file: &'static str,
    pub text: String,
}

pub fn pages() -> Vec<Page> {
    let commands = commands();
    vec![
        Page {
            file: "keys.md",
            text: keys_page(&commands),
        },
        Page {
            file: "commands.md",
            text: commands_page(&commands),
        },
        Page {
            file: "settings.md",
            text: settings_page(),
        },
    ]
}

/// Every command a release build has, as (id, label), in registration order.
fn commands() -> Vec<(String, String)> {
    let mut r: CommandRegistry<Model> = CommandRegistry::new(|_| {});
    register_commands(&mut r);
    r.all()
        .iter()
        .filter(|c| !c.id.starts_with("dev."))
        .map(|c| (c.id.clone(), c.label.clone()))
        .collect()
}

fn keymap() -> Keymap {
    DEFAULT_KEYMAP
        .iter()
        .map(|(chord, id, _)| (chord.to_string(), id.to_string()))
        .collect()
}

const NOTICE: &str = "This page is generated from the app's own tables on every build of the site, so it matches the release it was built with.";

fn frontmatter(title: &str, description: &str) -> String {
    format!("---\ntitle: {title}\ndescription: {description}\n---\n\n{NOTICE}\n\n")
}

/// Markdown table cells must not carry a pipe, and `<` would open a tag.
fn cell(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('|', "\\|")
}

/// `Cmd Shift ]` as one `<kbd>` per key, the way the in-app panel draws a
/// box per key. `<kbd>` rather than backticks: the backtick key would close
/// a code span.
fn kbd(chord: &str) -> String {
    chord_keys(chord)
        .iter()
        .map(|k| format!("<kbd>{}</kbd>", cell(k)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn label_of<'a>(commands: &'a [(String, String)], id: &'a str) -> &'a str {
    commands
        .iter()
        .find(|(i, _)| i == id)
        .map_or(id, |(_, l)| l.as_str())
}

/// A section title and its rows: (command id, label, raw chords).
type Section = (String, Vec<(String, String, Vec<String>)>);

/// Sections as `shortcuts.rs` groups them, each command with its chords
/// spelled as the keymap writes them (the panel's `format_chord` output is
/// split back into keys by `kbd`).
fn sections(commands: &[(String, String)]) -> Vec<Section> {
    let pairs: Vec<(&str, &str)> = commands
        .iter()
        .map(|(i, l)| (i.as_str(), l.as_str()))
        .collect();
    let map = keymap();
    shortcut_sections(&map, &pairs)
        .into_iter()
        .map(|s| {
            let rows = s
                .shortcuts
                .into_iter()
                .map(|sc| {
                    let chords = map
                        .iter()
                        .filter(|(_, id)| *id == sc.id)
                        .map(|(c, _)| c.clone())
                        .collect();
                    (sc.id, sc.label, chords)
                })
                .collect();
            (s.title, rows)
        })
        .collect()
}

fn capitalised(title: &str) -> String {
    let mut c = title.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn keys_page(commands: &[(String, String)]) -> String {
    let mut out = frontmatter(
        "Keys",
        "Every default key binding, the mouse gestures, and what changes inside a locked card.",
    );
    out.push_str("Every app binding holds Cmd, except Ctrl plus a digit (workspaces) and Ctrl+Tab (the card switcher). Everything else goes to the focused terminal, so shell and TUI keys keep working. Keys are matched by their physical position, so a Turkish Q or AZERTY keyboard presses the same keys this page shows.\n\nTo change a binding, copy its line from `keybindings.default.json` into `keybindings.json` beside it. The palette (<kbd>Cmd</kbd> <kbd>Shift</kbd> <kbd>P</kbd>) lists every command, bound or not; see [Commands](../commands/).\n\n");
    for (title, rows) in sections(commands) {
        let bound: Vec<_> = rows.iter().filter(|(_, _, c)| !c.is_empty()).collect();
        if bound.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "## {}\n\n| Keys | Does |\n| --- | --- |\n",
            capitalised(&title)
        ));
        for (_, label, chords) in bound {
            let keys: Vec<String> = chords.iter().map(|c| kbd(c)).collect();
            out.push_str(&format!("| {} | {} |\n", keys.join(" or "), cell(label)));
        }
        out.push('\n');
    }
    out.push_str("## Mouse\n\n| Gesture | Does |\n| --- | --- |\n");
    for (gesture, does, _) in GESTURES {
        out.push_str(&format!("| {} | {} |\n", cell(gesture), cell(does)));
    }
    out.push('\n');

    out.push_str("## Inside a locked card\n\nA browser or editor card locks the keyboard once you click into it or press Enter on it. In an editor, Escape lets go once it has nothing else to close; in a browser card, Escape twice. <kbd>Cmd</kbd> <kbd>Esc</kbd> lets go of either. While locked, these chords mean what they mean in Chrome or a text editor instead of what the canvas binds them to. <kbd>Cmd</kbd> <kbd>L</kbd>, <kbd>Cmd</kbd> <kbd>Esc</kbd>, Ctrl plus a digit and Ctrl+Tab stay the app's.\n\n");
    locked_table(&mut out, "Locked browser card", commands, |c| {
        browser_keys::lock_override(c).or_else(|| browser_keys::browser_override(c))
    });
    locked_table(
        &mut out,
        "Locked editor card",
        commands,
        editor_keys::lock_override,
    );
    let kept: Vec<String> = probe_chords()
        .into_iter()
        .filter(|c| editor_keys::editor_keeps(c))
        .map(|c| kbd(&c))
        .collect();
    out.push_str(&format!(
        "Inside a locked editor the text itself also keeps {}: find, find next and previous, replace, comment, the buffer's own undo and redo, and select to the line or file boundary.\n",
        kept.join(", ")
    ));
    out
}

fn locked_table(
    out: &mut String,
    title: &str,
    commands: &[(String, String)],
    lookup: impl Fn(&str) -> Option<&'static str>,
) {
    out.push_str(&format!("### {title}\n\n| Keys | Does |\n| --- | --- |\n"));
    for chord in probe_chords() {
        if let Some(id) = lookup(&chord) {
            out.push_str(&format!(
                "| {} | {} |\n",
                kbd(&chord),
                cell(label_of(commands, id))
            ));
        }
    }
    out.push('\n');
}

/// Every chord the keymap can spell with the keys a binding uses: each
/// modifier set, in the keymap's order (cmd, ctrl, alt, shift), over letters,
/// digits, punctuation and the named keys.
fn probe_chords() -> Vec<String> {
    const MODS: [&str; 4] = ["cmd", "ctrl", "alt", "shift"];
    let mut keys: Vec<String> = ('a'..='z').chain('0'..='9').map(String::from).collect();
    keys.extend("[]=-/\\;',.`".chars().map(String::from));
    for named in [
        "arrowleft",
        "arrowright",
        "arrowup",
        "arrowdown",
        "enter",
        "escape",
        "tab",
        "backspace",
        " ",
    ] {
        keys.push(named.into());
    }
    let mut out = vec![];
    for mask in 1..16u8 {
        let held: Vec<&str> = (0..4)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| MODS[i])
            .collect();
        for key in &keys {
            out.push(format!("{}+{key}", held.join("+")));
        }
    }
    out
}

fn commands_page(commands: &[(String, String)]) -> String {
    let mut out = frontmatter(
        "Commands",
        "Every command infiniterm has, with its id and default keys.",
    );
    out.push_str("Everything the app does is a named command. The palette (<kbd>Cmd</kbd> <kbd>Shift</kbd> <kbd>P</kbd>) runs any of them by name, `keybindings.json` binds them by id, and `ift commands` prints this list from the running app. Nothing is mouse-only.\n\n");
    for (title, rows) in sections(commands) {
        out.push_str(&format!(
            "## {}\n\n| Command | Id | Keys |\n| --- | --- | --- |\n",
            capitalised(&title)
        ));
        for (id, label, chords) in rows {
            let keys: Vec<String> = chords.iter().map(|c| kbd(c)).collect();
            out.push_str(&format!(
                "| {} | `{}` | {} |\n",
                cell(&label),
                id,
                keys.join(" or ")
            ));
        }
        out.push('\n');
    }
    out
}

fn settings_page() -> String {
    let mut out = frontmatter("Settings", "Every setting, its default, and what it does.");
    out.push_str("Settings live in `~/.config/infiniterm/settings.json`. Beside it, `settings.default.json` lists every setting with its default and is rewritten at every launch, so edits there are lost; copy a line into `settings.json` and change it there. Keys are flat and dotted, VS Code style: `\"terminal.fontSize\": 16` is a whole override. Comments and trailing commas are allowed.\n\n");
    let value = serde_json::to_value(default_config()).expect("config serialises");
    let flat = flatten(value.as_object().expect("config is an object"));
    let mut group_seen: Option<String> = None;
    for (key, default) in &flat {
        let group = key.rsplit_once('.').map(|(g, _)| g.to_string());
        if group != group_seen {
            if let Some(g) = &group {
                out.push_str(&format!("## {g}\n\n"));
                let lines = doc(g);
                if !lines.is_empty() {
                    out.push_str(&format!("{}\n\n", prose(lines)));
                }
            }
            group_seen = group;
        }
        out.push_str(&format!(
            "### `{key}`\n\nDefault: `{}`\n\n{}\n\n",
            scalar(default),
            prose(doc(key))
        ));
    }
    out
}

fn doc(key: &str) -> &'static [&'static str] {
    SETTINGS_DOC
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(&[], |(_, lines)| lines)
}

/// The doc lines are wrapped for a JSON comment; a web page reflows them.
fn prose(lines: &[&str]) -> String {
    cell(&lines.join(" "))
}

/// `14`, not `14.0`, as `settings_doc` prints it.
fn scalar(value: &Value) -> String {
    match value.as_f64() {
        Some(n) => n.to_string(),
        None => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(file: &str) -> String {
        pages().into_iter().find(|p| p.file == file).unwrap().text
    }

    #[test]
    fn every_release_command_is_listed_and_no_dev_one() {
        let text = page("commands.md");
        let all = commands();
        assert!(!all.is_empty());
        for (id, _) in &all {
            assert!(text.contains(&format!("`{id}`")), "{id} missing");
        }
        assert!(!text.contains("`dev."));
    }

    #[test]
    fn every_default_binding_is_on_the_keys_page() {
        let text = page("keys.md");
        let released: Vec<String> = commands().into_iter().map(|(id, _)| id).collect();
        for (chord, id, _) in DEFAULT_KEYMAP {
            if released.iter().any(|r| r == id) {
                assert!(text.contains(&kbd(chord)), "{chord} for {id} missing");
            }
        }
    }

    // The lock tables are read by probing; a chord the probe cannot spell
    // would silently drop off the page.
    #[test]
    fn the_probe_finds_the_lock_chords_that_exist() {
        let probed = probe_chords();
        for chord in [
            "cmd+t",
            "cmd+shift+]",
            "ctrl+g",
            "cmd+9",
            "cmd+shift+arrowleft",
        ] {
            assert!(probed.iter().any(|c| c == chord), "{chord}");
        }
        let text = page("keys.md");
        let row = format!(
            "| {} | {} |",
            kbd("ctrl+g"),
            label_of(&commands(), "editor.goToLine")
        );
        assert!(text.contains(&row), "{row}");
    }

    #[test]
    fn every_setting_is_on_the_settings_page_with_its_default() {
        let text = page("settings.md");
        let value = serde_json::to_value(default_config()).unwrap();
        for (key, default) in flatten(value.as_object().unwrap()) {
            assert!(
                text.contains(&format!("### `{key}`\n\nDefault: `{}`", scalar(&default))),
                "{key}"
            );
        }
    }

    #[test]
    fn a_pipe_or_a_tag_cannot_break_a_table() {
        assert_eq!(cell("a|b<c>"), "a\\|b&lt;c&gt;");
        assert_eq!(kbd("cmd+`"), "<kbd>Cmd</kbd> <kbd>`</kbd>");
    }

    #[test]
    fn every_page_opens_with_starlight_frontmatter() {
        for p in pages() {
            assert!(p.text.starts_with("---\ntitle: "), "{}", p.file);
        }
    }
}
