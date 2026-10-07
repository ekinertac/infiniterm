//! The shortcut list, built from the live keymap and command registry.
//! Port of shortcuts.ts and its tests.
//!
//! Derived rather than written down, because a hand-maintained list of keys
//! is wrong within a week. Anything shown is a binding that exists, and a
//! command with no binding shows up unbound instead of silently missing,
//! the same failure `unregistered_bindings` catches from the other side.
//!
//! Chords are spelled out (`Cmd Shift T`), never as the Mac glyphs, which
//! are only obvious to people who already know the shortcut. Cmd first,
//! matching the order a chord is written in, so the panel and the keymap
//! read the same way. One box per key and no `+` between them; prose keeps
//! the `+`. The shortcut panel element renders the sections.
use crate::fuzzy::fuzzy_match;
use crate::keymap::Keymap;

/// Section order, and the id prefix that fills each one. A command matching
/// no prefix falls into the trailing catch-all rather than disappearing.
const SECTIONS: [(&str, &str); 8] = [
    ("card.", "cards"),
    ("editor.", "editor"),
    ("group.", "groups"),
    // Seventeen commands, the Ctrl+digit jumps among them; lost in "other"
    // they read as leftovers rather than a feature of their own.
    ("workspace.", "workspaces"),
    ("focus.", "moving around"),
    ("canvas.", "zoom"),
    ("theme.", "themes"),
    ("app.", "app"),
];

const OTHER: &str = "other";

const MODIFIERS: [(&str, &str); 4] = [
    ("cmd", "Cmd"),
    ("ctrl", "Ctrl"),
    ("alt", "Alt"),
    ("shift", "Shift"),
];

const KEYS: [(&str, &str); 9] = [
    ("arrowleft", "Left"),
    ("arrowright", "Right"),
    ("arrowup", "Up"),
    ("arrowdown", "Down"),
    ("enter", "Enter"),
    ("escape", "Esc"),
    ("backspace", "Backspace"),
    ("tab", "Tab"),
    (" ", "Space"),
];

/// A chord as separate keys: `cmd+shift+]` gives `["Cmd", "Shift", "]"]`,
/// one per drawn box. The key is the LAST part, so a chord ending in `+`
/// (the literal plus key) still finds it.
pub fn chord_keys(chord: &str) -> Vec<String> {
    let parts: Vec<&str> = chord.split('+').collect();
    let (held, key) = parts.split_at(parts.len() - 1);
    let key = key[0];
    let mut out: Vec<String> = MODIFIERS
        .iter()
        .filter(|(name, _)| held.contains(name))
        .map(|(_, label)| label.to_string())
        .collect();
    out.push(match KEYS.iter().find(|(k, _)| *k == key) {
        Some((_, label)) => label.to_string(),
        None if key.chars().count() == 1 => key.to_uppercase(),
        None => key.to_string(),
    });
    out
}

/// The separator between keys, and what a string-only caller splits back
/// on. Safe because no key label contains a space: the space bar is "Space".
pub const CHORD_SEPARATOR: &str = " ";

/// The same chord as one string, for the palette's hint field.
pub fn format_chord(chord: &str) -> String {
    chord_keys(chord).join(CHORD_SEPARATOR)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub id: String,
    pub label: String,
    /// Every chord bound to this command, formatted, in keymap order. Empty
    /// when unbound.
    pub chords: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutSection {
    pub title: String,
    pub shortcuts: Vec<Shortcut>,
}

/// Commands grouped into sections, each carrying every chord bound to it.
/// Commands keep REGISTRATION order within a section (roughly how the
/// feature reads: new, close, move); alphabetical would interleave them.
/// Empty sections are dropped. `commands` are (id, label) pairs.
pub fn shortcut_sections(bindings: &Keymap, commands: &[(&str, &str)]) -> Vec<ShortcutSection> {
    let mut sections: Vec<ShortcutSection> = SECTIONS
        .iter()
        .map(|(_, title)| title)
        .chain(std::iter::once(&OTHER))
        .map(|title| ShortcutSection {
            title: title.to_string(),
            shortcuts: vec![],
        })
        .collect();
    for (id, label) in commands {
        let title = SECTIONS
            .iter()
            .find(|(prefix, _)| id.starts_with(prefix))
            .map_or(OTHER, |(_, t)| t);
        let chords = bindings
            .iter()
            .filter(|(_, bound)| bound == id)
            .map(|(chord, _)| format_chord(chord))
            .collect();
        let section = sections
            .iter_mut()
            .find(|s| s.title == title)
            .expect("every title has a section");
        section.shortcuts.push(Shortcut {
            id: id.to_string(),
            label: label.to_string(),
            chords,
        });
    }
    sections.retain(|s| !s.shortcuts.is_empty());
    sections
}

/// Gestures, which are not commands and so cannot be derived. They are the
/// least discoverable thing in the app, which is exactly why they belong in
/// the panel even though they break its one-source rule. The panel and the
/// site's keys page both read this list.
///
/// Each row: what you do, what it does, and the id `usage_log` records the
/// gesture under ("" when it records none). The ids are what keep the list
/// whole: a test fails when `usage_log::MOUSE_GESTURES` names a gesture no
/// row here carries, which is how marquee selection shipped and stayed off
/// this panel for days (2026-10-01).
pub const GESTURES: &[(&str, &str, &str)] = &[
    (
        "Cmd scroll or pinch",
        "Zoom about the pointer",
        "mouse.canvas.zoom",
    ),
    ("Cmd drag", "Pan the canvas", "mouse.canvas.pan"),
    ("middle drag", "Pan the canvas", "mouse.canvas.pan"),
    (
        "drag the top edge or the label",
        "Move a card",
        "mouse.card.drag",
    ),
    ("drag any other edge", "Resize a card", "mouse.card.resize"),
    (
        "drag the name tab",
        "Move a whole group",
        "mouse.group.drag",
    ),
    (
        "drag on empty canvas",
        "Select every card it touches",
        "mouse.marquee",
    ),
    (
        "Shift drag on empty canvas",
        "Add to the selection",
        "mouse.marquee",
    ),
    (
        "Cmd click a card",
        "Add it to the selection or take it out",
        "mouse.card.cmdclick",
    ),
    (
        "Shift click a label or frame",
        "Add it to the selection or take it out",
        "",
    ),
    (
        "drag a selected card",
        "Move the whole selection",
        "mouse.selection.drag",
    ),
    (
        "double-click a frame or label",
        "Fit the card",
        "mouse.fitCard.doubleclick",
    ),
    (
        "double-click empty canvas",
        "Fit everything",
        "mouse.fitAll.doubleclick",
    ),
    (
        "hold left, click right",
        "Fit everything",
        "mouse.fitAll.chord",
    ),
];

/// The sections with only the shortcuts matching `query`, empty sections
/// gone. Matches on the label, the keys or the command id, since "close",
/// "cmd w" and "card.close" are the questions a cheat sheet gets asked (the
/// id is what keybindings.json binds, #74). Fuzzy like the palette but
/// unranked: a list you scan wants stable positions more than a best guess.
pub fn filter_shortcuts(sections: &[ShortcutSection], query: &str) -> Vec<ShortcutSection> {
    let q = query.trim();
    if q.is_empty() {
        return sections.to_vec();
    }
    sections
        .iter()
        .map(|s| ShortcutSection {
            title: s.title.clone(),
            shortcuts: s
                .shortcuts
                .iter()
                .filter(|sc| {
                    fuzzy_match(q, &sc.label).is_some()
                        || sc.chords.iter().any(|c| fuzzy_match(q, c).is_some())
                        || fuzzy_match(q, &sc.id).is_some()
                })
                .cloned()
                .collect(),
        })
        .filter(|s| !s.shortcuts.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The id is searchable: what you type into keybindings.json finds its
    // row (#74).
    #[test]
    fn the_filter_matches_a_command_id() {
        let km: Keymap = vec![];
        let s = shortcut_sections(
            &km,
            &[
                ("theme.pick", "Theme: switch…"),
                ("card.close", "Card: close"),
            ],
        );
        let hits = filter_shortcuts(&s, "theme.pick");
        let ids: Vec<&str> = hits
            .iter()
            .flat_map(|s| s.shortcuts.iter().map(|sc| sc.id.as_str()))
            .collect();
        assert_eq!(ids, ["theme.pick"]);
    }

    // Every gesture the usage log counts has a row in the panel: a gesture
    // that is recorded but not listed is one a user cannot find.
    #[test]
    fn every_recorded_mouse_gesture_is_in_the_panel() {
        for (id, label) in crate::usage_log::MOUSE_GESTURES {
            assert!(
                GESTURES.iter().any(|(_, _, g)| g == id),
                "{id} ({label}) is recorded in usage.log but missing from GESTURES"
            );
        }
    }

    #[test]
    fn every_usage_id_in_the_panel_is_one_the_log_knows() {
        for (keys, _, id) in GESTURES {
            assert!(
                id.is_empty()
                    || crate::usage_log::MOUSE_GESTURES
                        .iter()
                        .any(|(g, _)| g == id),
                "{keys}: {id} is not in usage_log::MOUSE_GESTURES"
            );
        }
    }
    use crate::keymap::DEFAULT_KEYMAP;

    fn km(pairs: &[(&str, &str)]) -> Keymap {
        pairs
            .iter()
            .map(|(c, i)| (c.to_string(), i.to_string()))
            .collect()
    }

    // chordFor normalises to cmd, ctrl, alt, shift; a different display order
    // would make the panel hard to compare against the keymap being edited.
    #[test]
    fn format_chord_puts_cmd_first_matching_how_a_chord_is_written() {
        assert_eq!(format_chord("cmd+shift+g"), "Cmd Shift G");
        assert_eq!(format_chord("cmd+alt+t"), "Cmd Alt T");
        assert_eq!(format_chord("cmd+alt+arrowleft"), "Cmd Alt Left");
    }

    #[test]
    fn format_chord_names_the_keys_that_have_no_printable_character() {
        assert_eq!(format_chord("cmd+shift+enter"), "Cmd Shift Enter");
        assert_eq!(format_chord("cmd+arrowdown"), "Cmd Down");
    }

    #[test]
    fn format_chord_keeps_punctuation_and_digits_as_they_are_labelled() {
        assert_eq!(format_chord("cmd+shift+]"), "Cmd Shift ]");
        assert_eq!(format_chord("cmd+,"), "Cmd ,");
        assert_eq!(format_chord("cmd+3"), "Cmd 3");
    }

    #[test]
    fn sections_follow_the_command_id_prefix() {
        let sections = shortcut_sections(
            &km(&[("cmd+t", "card.new.terminal")]),
            &[
                ("card.new.terminal", "card.new.terminal"),
                ("group.new", "group.new"),
            ],
        );
        assert_eq!(
            sections
                .iter()
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>(),
            ["cards", "groups"]
        );
    }

    #[test]
    fn workspace_commands_have_a_section_of_their_own() {
        let sections = shortcut_sections(
            &km(&[("ctrl+1", "workspace.show.1")]),
            &[("workspace.show.1", "Workspace: go to 1")],
        );
        assert_eq!(sections[0].title, "workspaces");
    }

    #[test]
    fn a_command_matching_no_prefix_still_appears() {
        let sections = shortcut_sections(&km(&[]), &[("weird.thing", "weird.thing")]);
        assert_eq!(
            sections,
            [ShortcutSection {
                title: "other".into(),
                shortcuts: vec![Shortcut {
                    id: "weird.thing".into(),
                    label: "weird.thing".into(),
                    chords: vec![]
                }]
            }]
        );
    }

    // Both Cmd+Alt+Left and Cmd+Alt+J run focus.move.left; the panel must show both.
    #[test]
    fn every_chord_bound_to_one_command_is_collected() {
        let sections = shortcut_sections(
            &km(&[
                ("cmd+alt+arrowleft", "focus.move.left"),
                ("cmd+alt+j", "focus.move.left"),
            ]),
            &[("focus.move.left", "focus.move.left")],
        );
        assert_eq!(
            sections[0].shortcuts[0].chords,
            ["Cmd Alt Left", "Cmd Alt J"]
        );
    }

    // The mirror of unregistered_bindings: this catches a command with no chord.
    #[test]
    fn an_unbound_command_is_listed_with_no_chords() {
        let sections = shortcut_sections(&km(&[]), &[("card.close", "Close active card")]);
        assert_eq!(
            sections[0].shortcuts[0],
            Shortcut {
                id: "card.close".into(),
                label: "Close active card".into(),
                chords: vec![]
            }
        );
    }

    #[test]
    fn a_section_with_no_commands_is_dropped() {
        assert_eq!(
            shortcut_sections(&km(&[]), &[("card.close", "card.close")]).len(),
            1
        );
    }

    // The real keymap must survive formatting: no chord may render as empty.
    #[test]
    fn every_default_binding_formats_to_something() {
        for (chord, _, _) in DEFAULT_KEYMAP {
            assert!(!format_chord(chord).is_empty(), "{chord}");
        }
    }

    #[test]
    fn chord_keys_gives_each_key_separately_in_macos_order() {
        assert_eq!(chord_keys("cmd+shift+g"), ["Cmd", "Shift", "G"]);
        assert_eq!(chord_keys("cmd+arrowleft"), ["Cmd", "Left"]);
    }

    // The palette's hint is a plain string and splits back apart on the separator.
    #[test]
    fn format_chord_round_trips_through_a_split() {
        let split: Vec<String> = format_chord("cmd+alt+arrowleft")
            .split(' ')
            .map(String::from)
            .collect();
        assert_eq!(split, chord_keys("cmd+alt+arrowleft"));
    }

    fn sections() -> Vec<ShortcutSection> {
        let sc = |id: &str, label: &str, chord: &str| Shortcut {
            id: id.into(),
            label: label.into(),
            chords: vec![chord.into()],
        };
        vec![
            ShortcutSection {
                title: "cards".into(),
                shortcuts: vec![
                    sc("card.close", "Close active card", "Cmd W"),
                    sc("card.new", "New terminal card", "Cmd T"),
                ],
            },
            ShortcutSection {
                title: "canvas".into(),
                shortcuts: vec![sc("zoom", "Zoom in", "Cmd =")],
            },
        ]
    }

    fn ids(sections: &[ShortcutSection]) -> Vec<&str> {
        sections[0]
            .shortcuts
            .iter()
            .map(|s| s.id.as_str())
            .collect()
    }

    #[test]
    fn returns_everything_for_an_empty_query_in_order() {
        assert_eq!(filter_shortcuts(&sections(), "  "), sections());
    }

    #[test]
    fn matches_on_the_label() {
        let out = filter_shortcuts(&sections(), "close");
        assert_eq!(
            out.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
            ["cards"]
        );
        assert_eq!(ids(&out), ["card.close"]);
    }

    // "What is Cmd W?" is the other question a cheat sheet answers.
    #[test]
    fn matches_on_the_keys() {
        assert_eq!(ids(&filter_shortcuts(&sections(), "cmd w")), ["card.close"]);
    }

    #[test]
    fn drops_sections_left_empty() {
        assert_eq!(
            filter_shortcuts(&sections(), "zoom")
                .iter()
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>(),
            ["canvas"]
        );
    }
}
