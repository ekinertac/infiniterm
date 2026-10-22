//! The Mac's chords on a keyboard with no Cmd key (Linux and Windows, #464).
//!
//! Decided by Ekin (docs/agent/command-key.md): where Cmd+K would become
//! Ctrl+K and Ctrl+K is input to a terminal (every letter, `[ ] \ /`,
//! Space), the app's chord is Ctrl+Shift+K; where Ctrl+K means nothing to a
//! terminal (digits, `=`, `-`, `,`, Enter, Escape, the arrows with another
//! modifier), plain Ctrl. A Mac chord that already has Shift and whose
//! unshifted twin is bound adds Alt (Cmd+Shift+D is Ctrl+Alt+Shift+D, since
//! Ctrl+Shift+D is Cmd+D's). The workspaces leave Ctrl+digit for Alt+digit
//! (Ctrl+1/2/3 are the fit keys). The Mac's Cmd+Ctrl layer gets explicit
//! keys or none (`CMD_CTRL_LAYER`).
//!
//! The keymap stays written in Mac chords. `PlatformKeys::build` turns the
//! active keymap's chords (plus `BODY_CHORDS`, the ones a body handles
//! itself) into a two-way table: the forward side is what the shortcuts
//! panel and the menus will show, the reverse side is what `input.rs` uses
//! to turn a PHYSICAL chord back into the Mac chord the rest of the app
//! understands, once, where the event enters. One table for both
//! directions, because the twin rule depends on which chords are bound and
//! two separately built maps would drift.
//!
//! Inside a card that owns the keyboard (a locked editor or browser) no
//! terminal is listening, so plain Ctrl is Cmd there, as in every Linux
//! editor and browser (`KeyContext::CardOwnsKeys`).
//!
//! Pure. The ui wiring is `input.rs` (`platform_keystroke`); the setting is
//! `keyboard.commandModifier` (`ctrl` uses this table, `super` keeps the
//! Mac's chords with Super for Cmd and needs none of this).
use std::collections::{HashMap, HashSet};

/// Chords a body handles itself rather than the keymap, which must map like
/// the keymap's: the terminal's copy, paste and cut (`terminal_body.rs`; Ekin:
/// Ctrl+Shift+C and Ctrl+Shift+V) and the own-window chord (`input.rs`).
pub const BODY_CHORDS: &[&str] = &["cmd+c", "cmd+v", "cmd+x", "cmd+shift+o"];

/// The Mac's Cmd+Ctrl chords: no second Cmd-like key exists off the Mac, so
/// each gets a key of its own or none (`None`: palette only, or the system's
/// own picker for the emoji panel). Ekin accepted the palette-only rows.
pub const CMD_CTRL_LAYER: &[(&str, Option<&str>)] = &[
    ("cmd+ctrl+f", Some("f11")),
    ("cmd+ctrl+w", Some("ctrl+alt+w")),
    ("cmd+ctrl+enter", Some("ctrl+alt+enter")),
    ("cmd+ctrl+t", Some("ctrl+alt+p")),
    ("cmd+ctrl+s", Some("ctrl+alt+v")),
    ("cmd+ctrl+space", None),
    ("cmd+ctrl+shift+r", None),
    ("cmd+ctrl+alt+arrowleft", None),
    ("cmd+ctrl+alt+arrowright", None),
    ("cmd+ctrl+alt+arrowup", None),
    ("cmd+ctrl+alt+arrowdown", None),
    ("cmd+ctrl+shift+arrowleft", None),
    ("cmd+ctrl+shift+arrowright", None),
    ("cmd+ctrl+shift+arrowup", None),
    ("cmd+ctrl+shift+arrowdown", None),
];

/// Keys whose plain Ctrl chord is terminal input, so the app takes them
/// with Shift: every letter is a C0 control code, `[` is ESC, `\` is
/// SIGQUIT, `]` is GS, `/` is readline's undo in xterm, Space is NUL.
fn clashes(key: &str) -> bool {
    matches!(key, "[" | "]" | "\\" | "/" | "space" | " ")
        || (key.len() == 1 && key.chars().all(|c| c.is_ascii_lowercase()))
}

fn is_digit(key: &str) -> bool {
    key.len() == 1 && key.chars().all(|c| c.is_ascii_digit())
}

/// A chord split into its modifier set and key, in `keymap::chord_for`'s
/// spelling. A chord ending in `+` (the plus key) keeps it as the key.
fn split(chord: &str) -> (HashSet<&str>, &str) {
    if let Some(head) = chord.strip_suffix("++") {
        return (head.split('+').filter(|s| !s.is_empty()).collect(), "+");
    }
    match chord.rsplit_once('+') {
        Some((mods, key)) => (mods.split('+').collect(), key),
        None => (HashSet::new(), chord),
    }
}

/// `chord_for`'s order: cmd, ctrl, alt, shift, then the key.
fn join(mods: &HashSet<&str>, key: &str) -> String {
    let mut parts: Vec<&str> = ["cmd", "ctrl", "alt", "shift"]
        .into_iter()
        .filter(|m| mods.contains(m))
        .collect();
    parts.push(key);
    parts.join("+")
}

/// Where a keyboard event lands, which decides the rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyContext {
    /// The canvas, or a card whose keys may reach a terminal: the table.
    Canvas,
    /// A locked editor or browser: plain Ctrl is Cmd.
    CardOwnsKeys,
}

/// One Mac chord's platform chord, given every Mac chord in play (for the
/// twin rule). `None` when it has no key off the Mac.
pub fn platform_chord(mac: &str, all: &HashSet<String>) -> Option<String> {
    if let Some((_, to)) = CMD_CTRL_LAYER.iter().find(|(m, _)| *m == mac) {
        return to.map(String::from);
    }
    let (mods, key) = split(mac);
    if !mods.contains("cmd") {
        // The workspaces' Ctrl+digit moves to Alt+digit; the switcher's
        // Ctrl+Tab stays.
        if mods.len() == 1 && mods.contains("ctrl") && is_digit(key) {
            return Some(format!("alt+{key}"));
        }
        return Some(mac.to_string());
    }
    if mods.contains("ctrl") {
        // A Cmd+Ctrl chord the layer does not name: no key here.
        return None;
    }
    let mut rest: HashSet<&str> = mods.iter().copied().filter(|m| *m != "cmd").collect();
    rest.insert("ctrl");
    if clashes(key) {
        if rest.contains("shift") {
            let mut twin = rest.clone();
            twin.remove("shift");
            twin.remove("ctrl");
            twin.insert("cmd");
            if all.contains(&join(&twin, key)) {
                rest.insert("alt");
            }
        } else if !rest.contains("alt") {
            rest.insert("shift");
        }
    }
    Some(join(&rest, key))
}

/// The two-way table for one keymap.
#[derive(Clone, Debug, Default)]
pub struct PlatformKeys {
    forward: HashMap<String, Option<String>>,
    reverse: HashMap<String, String>,
}

impl PlatformKeys {
    /// From the Mac chords in play (the active keymap's and `BODY_CHORDS`).
    /// The second value lists what could not be mapped cleanly, for a
    /// notice: two Mac chords landing on one platform chord, and a Cmd+Ctrl
    /// chord with no key here.
    pub fn build<'a>(mac_chords: impl IntoIterator<Item = &'a str>) -> (Self, Vec<String>) {
        let all: HashSet<String> = mac_chords
            .into_iter()
            .chain(BODY_CHORDS.iter().copied())
            .map(String::from)
            .collect();
        let mut sorted: Vec<&String> = all.iter().collect();
        sorted.sort();
        let mut table = PlatformKeys::default();
        let mut problems = vec![];
        for mac in sorted {
            let to = platform_chord(mac, &all);
            match &to {
                Some(p) => {
                    if let Some(other) = table.reverse.get(p) {
                        problems.push(format!("{mac} and {other} both land on {p}"));
                    } else {
                        table.reverse.insert(p.clone(), mac.clone());
                    }
                }
                None if !CMD_CTRL_LAYER.iter().any(|(m, _)| m == mac) => {
                    problems.push(format!("no key off the Mac for {mac}; palette only"));
                }
                None => {}
            }
            table.forward.insert(mac.clone(), to);
        }
        (table, problems)
    }

    /// What the panel and the menus show for a Mac chord; `None` for a
    /// chord with no key here (and for one this table never saw).
    pub fn platform(&self, mac: &str) -> Option<&str> {
        self.forward.get(mac).and_then(|p| p.as_deref())
    }

    /// The Mac chord a PHYSICAL chord stands for, or `None` to leave the
    /// event as it is (a plain Ctrl+letter on its way to a terminal, a key
    /// nothing binds).
    pub fn mac(&self, physical: &str, ctx: KeyContext) -> Option<String> {
        if let Some(mac) = self.reverse.get(physical) {
            if ctx == KeyContext::Canvas || !physical.contains("ctrl") {
                return Some(mac.clone());
            }
        }
        if ctx == KeyContext::CardOwnsKeys {
            let (mods, key) = split(physical);
            // The switcher's Ctrl+Tab is never the card's.
            if mods.contains("ctrl") && key != "tab" {
                let mut m = mods.clone();
                m.remove("ctrl");
                m.insert("cmd");
                return Some(join(&m, key));
            }
        }
        None
    }
}

/// The key a rewritten keystroke must carry, in gpui's names, or `None` to
/// keep the key gpui reported. A new key is needed when the Mac chord's key
/// or Shift differs from the physical chord's: gpui reports shifted
/// punctuation as the SHIFTED character (`{` for Ctrl+Shift+[), which reads
/// back as Shift and picks the Cmd+Shift+[ command instead of Cmd+[.
pub fn rewritten_key<'a>(physical: &str, mac: &'a str) -> Option<&'a str> {
    let (p_mods, p_key) = split(physical);
    let (m_mods, m_key) = split(mac);
    if p_key == m_key && p_mods.contains("shift") == m_mods.contains("shift") {
        return None;
    }
    Some(match m_key {
        "arrowleft" => "left",
        "arrowright" => "right",
        "arrowup" => "up",
        "arrowdown" => "down",
        other => other,
    })
}

/// The default keymap as a markdown table, macOS chord beside the Linux and
/// Windows chord, for docs/agent/command-key.md (a test holds the doc to
/// this, so the doc cannot drift from the code).
pub fn default_table_markdown() -> String {
    use crate::keymap::DEFAULT_KEYMAP;
    let (table, _) = PlatformKeys::build(DEFAULT_KEYMAP.iter().map(|(c, _, _)| *c));
    let mut out = String::from("| macOS | command | Linux / Windows |\n|---|---|---|\n");
    for (mac, id, _) in DEFAULT_KEYMAP {
        let platform = table
            .platform(mac)
            .map_or("palette only".to_string(), |p| format!("`{p}`"));
        out.push_str(&format!("| `{mac}` | `{id}` | {platform} |\n"));
    }
    for mac in BODY_CHORDS {
        let platform = table
            .platform(mac)
            .map_or("palette only".to_string(), |p| format!("`{p}`"));
        out.push_str(&format!("| `{mac}` | (body chord) | {platform} |\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::DEFAULT_KEYMAP;

    #[test]
    fn the_doc_table_is_the_one_the_code_builds() {
        let doc = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/agent/command-key.md"),
        )
        .unwrap();
        let begin = "<!-- table:begin -->\n";
        let end = "<!-- table:end -->";
        let start = doc.find(begin).expect("no table:begin marker") + begin.len();
        let stop = doc[start..].find(end).expect("no table:end marker") + start;
        assert_eq!(
            &doc[start..stop],
            default_table_markdown(),
            "regenerate the table in docs/agent/command-key.md from platform_keys::default_table_markdown"
        );
    }

    fn defaults() -> (PlatformKeys, Vec<String>) {
        PlatformKeys::build(DEFAULT_KEYMAP.iter().map(|(c, _, _)| *c))
    }

    #[test]
    fn the_default_keymap_maps_with_no_collision() {
        let (_, problems) = defaults();
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn forward_then_reverse_is_the_mac_chord_for_every_mapped_chord() {
        let (t, _) = defaults();
        let mut seen = HashSet::new();
        for chord in DEFAULT_KEYMAP
            .iter()
            .map(|(c, _, _)| *c)
            .chain(BODY_CHORDS.iter().copied())
        {
            let Some(p) = t.platform(chord) else { continue };
            assert!(
                seen.insert(p.to_string())
                    || t.mac(p, KeyContext::Canvas).as_deref() == Some(chord),
                "{p} is shared"
            );
            assert_eq!(
                t.mac(p, KeyContext::Canvas).as_deref(),
                Some(chord),
                "{chord} -> {p}"
            );
        }
    }

    #[test]
    fn the_decided_chords() {
        let (t, _) = defaults();
        for (mac, platform) in [
            ("cmd+t", "ctrl+shift+t"),
            ("cmd+shift+t", "ctrl+alt+shift+t"),
            ("cmd+d", "ctrl+shift+d"),
            // Split down keeps Ctrl+Alt+Shift+D: Alt+Shift+D is readline's
            // Meta-d (kill-word) in a terminal.
            ("cmd+shift+d", "ctrl+alt+shift+d"),
            ("cmd+shift+p", "ctrl+shift+p"),
            ("cmd+shift+h", "ctrl+shift+h"),
            ("cmd+c", "ctrl+shift+c"),
            ("cmd+v", "ctrl+shift+v"),
            ("cmd+x", "ctrl+shift+x"),
            ("cmd+shift+c", "ctrl+alt+shift+c"),
            ("cmd+1", "ctrl+1"),
            ("ctrl+1", "alt+1"),
            ("ctrl+9", "alt+9"),
            ("cmd+=", "ctrl+="),
            // `-` is free (Ekin): gnome-terminal, Alacritty and WezTerm too.
            ("cmd+-", "ctrl+-"),
            ("cmd+shift+-", "ctrl+shift+-"),
            ("cmd+shift+=", "ctrl+shift+="),
            ("cmd+alt+arrowleft", "ctrl+alt+arrowleft"),
            ("cmd+shift+arrowleft", "ctrl+shift+arrowleft"),
            ("cmd+alt+shift+arrowleft", "ctrl+alt+shift+arrowleft"),
            ("cmd+alt+t", "ctrl+alt+t"),
            ("cmd+[", "ctrl+shift+["),
            ("cmd+shift+[", "ctrl+alt+shift+["),
            ("cmd+ctrl+f", "f11"),
            ("cmd+ctrl+t", "ctrl+alt+p"),
            ("cmd+ctrl+s", "ctrl+alt+v"),
            ("ctrl+tab", "ctrl+tab"),
            ("ctrl+shift+tab", "ctrl+shift+tab"),
        ] {
            assert_eq!(t.platform(mac), Some(platform), "{mac}");
        }
        for mac in [
            "cmd+ctrl+space",
            "cmd+ctrl+shift+r",
            "cmd+ctrl+alt+arrowleft",
            "cmd+ctrl+shift+arrowup",
        ] {
            assert_eq!(t.platform(mac), None, "{mac} is palette only");
        }
    }

    #[test]
    fn a_terminal_keeps_plain_ctrl_and_the_switcher_keeps_ctrl_tab() {
        let (t, _) = defaults();
        for physical in ["ctrl+c", "ctrl+d", "ctrl+r", "ctrl+arrowleft", "ctrl+4"] {
            assert_eq!(t.mac(physical, KeyContext::Canvas), None, "{physical}");
        }
        assert_eq!(
            t.mac("ctrl+tab", KeyContext::Canvas).as_deref(),
            Some("ctrl+tab")
        );
        assert_eq!(
            t.mac("ctrl+shift+t", KeyContext::Canvas).as_deref(),
            Some("cmd+t")
        );
        assert_eq!(
            t.mac("alt+3", KeyContext::Canvas).as_deref(),
            Some("ctrl+3")
        );
        assert_eq!(
            t.mac("f11", KeyContext::Canvas).as_deref(),
            Some("cmd+ctrl+f")
        );
        assert_eq!(
            t.mac("ctrl+alt+p", KeyContext::Canvas).as_deref(),
            Some("cmd+ctrl+t")
        );
    }

    #[test]
    fn in_a_card_that_owns_the_keys_plain_ctrl_is_cmd() {
        let (t, _) = defaults();
        let card = KeyContext::CardOwnsKeys;
        assert_eq!(t.mac("ctrl+c", card).as_deref(), Some("cmd+c"));
        assert_eq!(t.mac("ctrl+shift+t", card).as_deref(), Some("cmd+shift+t"));
        assert_eq!(t.mac("ctrl+]", card).as_deref(), Some("cmd+]"));
        // The switcher and the workspaces are never the card's.
        assert_eq!(t.mac("ctrl+tab", card), None);
        assert_eq!(t.mac("alt+2", card).as_deref(), Some("ctrl+2"));
        assert_eq!(t.mac("f11", card).as_deref(), Some("cmd+ctrl+f"));
        // Nothing with no Ctrl and no binding changes.
        assert_eq!(t.mac("shift+a", card), None);
    }

    #[test]
    fn a_user_binding_that_lands_on_a_bound_chord_is_reported() {
        let chords: Vec<&str> = DEFAULT_KEYMAP
            .iter()
            .map(|(c, _, _)| *c)
            // Ctrl+Alt+Shift+D is split down's.
            .chain(["cmd+alt+shift+d", "cmd+ctrl+k"])
            .collect();
        let (_, problems) = PlatformKeys::build(chords);
        assert!(
            problems.iter().any(|p| p.contains("ctrl+alt+shift+d")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("cmd+ctrl+k")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_rewrite_that_drops_shift_or_changes_the_key_carries_the_mac_key() {
        // Ctrl+Shift+[ is Cmd+[: gpui's `{` would read back as Shift.
        assert_eq!(rewritten_key("ctrl+shift+[", "cmd+["), Some("["));
        assert_eq!(rewritten_key("ctrl+shift+/", "cmd+/"), Some("/"));
        // Alt+Shift twin to Cmd+Shift: same key, same Shift.
        assert_eq!(rewritten_key("ctrl+alt+shift+k", "cmd+shift+k"), None);
        // Free keys keep gpui's key.
        assert_eq!(rewritten_key("ctrl+=", "cmd+="), None);
        assert_eq!(rewritten_key("alt+2", "ctrl+2"), None);
        // A changed key comes in gpui's name.
        assert_eq!(
            rewritten_key("ctrl+alt+f", "cmd+ctrl+arrowleft"),
            Some("left")
        );
        assert_eq!(rewritten_key("f11", "cmd+ctrl+f"), Some("f"));
    }
}
