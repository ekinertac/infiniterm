//! Chord parsing, the default keymap, and the one hard constraint in the
//! whole keyboard story: APP BINDINGS MUST INCLUDE CMD, except Ctrl plus a
//! digit. Port of keymap.ts and its tests.
//!
//! A focused terminal consumes Ctrl, Alt, bare keys and function keys and
//! has to keep consuming them or TUI apps break; macOS terminals leave Cmd
//! alone, so claiming it needs no prefix key and no modal state, the
//! reasoning iTerm2 uses. Ctrl+LETTER always means something to a shell;
//! Ctrl+DIGIT mostly does not, which is the whole width of the exception
//! (Ctrl+3 sends ESC on an xterm; rebind it if you live in vim).
//!
//! Chords are built from the PHYSICAL key, not the character the layout
//! produced: Option+J reports `∆` and Shift+= reports `+`, and a binding
//! written `cmd+shift+=` never matched an event whose key was `+` (it sat
//! dead in the reference's default keymap for exactly that reason). The
//! reference reads `e.code`; gpui's `Keystroke` has no key code and gives
//! the layout's unshifted character for letters and the SHIFTED one, with
//! shift cleared, for punctuation and digits. `key_name` takes either: a
//! physical code when the platform has one, else the character, which the
//! table below un-shifts. On a non-US layout gpui uses the layout's Cmd
//! layer (Apple layouts map it to ASCII QWERTY, which is why shortcuts work
//! in Zed for Russian users); Turkish-Q is the unverified case, see
//! HANDOVER.md.
//!
//! Every binding carries the reason beside it. Do not restate chords in
//! other files: name the command, not the key. `shortcuts.rs` renders this
//! for the panel; the ui crate dispatches with `chord_for`.
use serde_json::Value;

/// A chord and the command it runs, in the order the generated file lists them.
pub type Keymap = Vec<(String, String)>;

/// The default bindings. Each entry: chord, command id, and the reason.
pub const DEFAULT_KEYMAP: &[(&str, &str, &str)] = &[
    // Workspaces are the closest thing here to browser tabs, and this is the
    // browser's prev/next tab chord. Cmd plus a bare arrow used to do it and
    // was given back to the shell: in every Mac text field Cmd+Left is the
    // start of the line, and a terminal is where that reflex is strongest.
    (
        "cmd+shift+[",
        "workspace.prev",
        "browser prev-tab chord; Cmd+Arrow went back to the shell",
    ),
    ("cmd+shift+]", "workspace.next", "browser next-tab chord"),
    // A new canvas: Cmd+N is a new document, the heavier chord the heavier thing.
    (
        "cmd+shift+n",
        "workspace.new",
        "the heavier chord for the heavier thing, as browsers do",
    ),
    // Cmd+W closes the card, the heavier chord the canvas; asks first, since
    // every shell on it goes too.
    (
        "cmd+shift+w",
        "workspace.close",
        "asks first: every shell on it goes",
    ),
    // Straight to a workspace by position, the way every tabbed app numbers
    // tabs. Ctrl rather than Cmd because Cmd+1..3 are the zoom targets, and
    // ctrl+DIGIT is the one Ctrl range a terminal mostly leaves alone.
    (
        "ctrl+1",
        "workspace.show.1",
        "Cmd+1..3 are zoom targets; ctrl+digit is the one Ctrl range a shell leaves alone",
    ),
    ("ctrl+2", "workspace.show.2", ""),
    (
        "ctrl+3",
        "workspace.show.3",
        "sends ESC on an xterm; rebind if you live in vim",
    ),
    ("ctrl+4", "workspace.show.4", ""),
    ("ctrl+5", "workspace.show.5", ""),
    ("ctrl+6", "workspace.show.6", ""),
    ("ctrl+7", "workspace.show.7", ""),
    ("ctrl+8", "workspace.show.8", ""),
    ("ctrl+9", "workspace.show.9", ""),
    // Shift plus a direction SELECTS MORE, the way it does in every text
    // field. That reflex is the rule behind the whole keymap: a chord does
    // what the same chord does in the text you type all day, with Cmd in
    // front because a terminal owns everything else.
    (
        "cmd+shift+arrowleft",
        "focus.extend.left",
        "Shift+direction selects more, as in every text field",
    ),
    ("cmd+shift+arrowright", "focus.extend.right", ""),
    ("cmd+shift+arrowup", "focus.extend.up", ""),
    ("cmd+shift+arrowdown", "focus.extend.down", ""),
    // iTerm2's split-pane navigation. Plain Option+Arrow is unavailable: a
    // terminal reads it as word movement.
    (
        "cmd+alt+arrowleft",
        "focus.move.left",
        "iTerm2's pane navigation; bare Option+Arrow is word movement in a shell",
    ),
    ("cmd+alt+arrowright", "focus.move.right", ""),
    ("cmd+alt+arrowup", "focus.move.up", ""),
    ("cmd+alt+arrowdown", "focus.move.down", ""),
    // Swaps the active card with its neighbour, whole rect for whole rect.
    // The heaviest chord for the rarest move: it gave Cmd+Shift+Arrow up to
    // selection because selecting happens ten times as often. card.move.*
    // and card.resize.* stay registered and unbound: a new card already
    // lands in the first free slot, so the question is which SLOT, not
    // where its corner sits.
    (
        "cmd+alt+shift+arrowleft",
        "card.swap.left",
        "the heaviest chord for the rarest move",
    ),
    ("cmd+alt+shift+arrowright", "card.swap.right", ""),
    ("cmd+alt+shift+arrowup", "card.swap.up", ""),
    ("cmd+alt+shift+arrowdown", "card.swap.down", ""),
    // Same commands on JKL, so switching cards does not need the arrow
    // cluster. Cmd+Alt+I was WebKit's inspector in the reference and is
    // left unbound here too so the two keymaps stay the same file.
    (
        "cmd+alt+j",
        "focus.move.left",
        "JKL so switching cards needs no arrow cluster; I stays free",
    ),
    ("cmd+alt+k", "focus.move.down", ""),
    ("cmd+alt+l", "focus.move.right", ""),
    // Cmd+0 is actual size everywhere else (browsers, editors).
    (
        "cmd+0",
        "canvas.zoom.actual",
        "actual size, as in every browser and editor",
    ),
    ("cmd+1", "canvas.zoom.fitCard", ""),
    ("cmd+2", "canvas.zoom.fitAll", ""),
    // Keyboard zoom, since scroll-zoom needs Cmd held and a pinch is awkward
    // mid-keystroke.
    (
        "cmd+=",
        "canvas.zoom.in",
        "scroll-zoom needs Cmd held; a pinch is awkward mid-keystroke",
    ),
    ("cmd+-", "canvas.zoom.out", ""),
    // Shift on the same keys sizes the INTERFACE rather than the canvas:
    // labels, borders and the status bar are screen-constant by design and
    // so are the one thing zoom cannot reach.
    (
        "cmd+shift+=",
        "ui.scale.up",
        "the interface, not the canvas: the one thing zoom cannot reach",
    ),
    ("cmd+shift+-", "ui.scale.down", ""),
    ("cmd+shift+0", "ui.scale.reset", ""),
    // iTerm2's maximise-pane chord. Escape is deliberately NOT bound to
    // restore: a terminal needs it.
    (
        "cmd+shift+enter",
        "card.maximize.toggle",
        "iTerm2's maximise pane; Escape stays the shell's",
    ),
    // H for hide. Cmd+H and Cmd+Alt+H are macOS's (hide the app, hide the
    // others) and unbindable; this one is free.
    (
        "cmd+shift+h",
        "card.mask",
        "a decoy over the card while somebody reads your screen",
    ),
    // The address bar chord every browser uses, and free here: a shell
    // knows Ctrl+L as clear, never Cmd+L.
    (
        "cmd+l",
        "card.omnibox",
        "the address bar chord every browser uses",
    ),
    ("cmd+t", "card.new.terminal", ""),
    // iTerm2's split keys with iTerm2's meaning: beside, then below. The
    // card is halved, not a pane tree grown.
    ("cmd+d", "card.split.right", "iTerm2's split: beside"),
    ("cmd+shift+d", "card.split.down", "iTerm2's split: below"),
    // iTerm2's clear. Reserved for it from the start, which is why the
    // palette is on Cmd+Shift+P.
    (
        "cmd+k",
        "card.clear",
        "iTerm2's clear; why the palette is not here",
    ),
    // The same key with Alt for the variant: a new card that joins NO group,
    // otherwise unreachable from inside one because Cmd+T always inherits.
    (
        "cmd+alt+t",
        "card.new.ungrouped",
        "Cmd+T always inherits the group; this is the way out",
    ),
    ("cmd+w", "card.close", ""),
    (
        "cmd+ctrl+w",
        "card.close.leave",
        "the split partner keeps its size; the space stays free",
    ),
    (
        "cmd+ctrl+enter",
        "card.size.reset",
        "grows into the free space beside and below, up to the default size",
    ),
    (
        "cmd+z",
        "layout.undo",
        "moves, swaps, drops, resizes, closes, new cards",
    ),
    ("cmd+shift+z", "layout.redo", ""),
    (
        "cmd+shift+l",
        "card.protect",
        "the card cannot be closed until unlocked; the label wears a lock",
    ),
    (
        "cmd+alt+s",
        "card.size",
        "a size from the default's fractions: full, halves, quarter, doubles",
    ),
    // The undo for Cmd+W. Cmd+Shift+T is the browser's chord for this and
    // is already the placement menu here, so the close chord takes Ctrl.
    (
        "cmd+ctrl+t",
        "card.reopen",
        "Cmd+Shift+T is already the placement menu",
    ),
    // The editor's save. A terminal has nothing to save, so the key is inert
    // there rather than reaching the shell, the same as every Cmd key.
    (
        "cmd+s",
        "card.save",
        "inert on a terminal, like every Cmd key",
    ),
    // An empty editor, the way Cmd+N is a new document everywhere on a Mac.
    (
        "cmd+n",
        "card.new.editor",
        "a new document, as everywhere on a Mac",
    ),
    // Who last touched each line, in a diff card's gutter.
    ("cmd+b", "editor.blame", ""),
    // The agent's session as turns, beside its card. Cmd+I is "info" in
    // iTerm2 and most Mac apps, and free of any shell meaning.
    (
        "cmd+i",
        "card.transcript",
        "\"info\" in iTerm2 and most Mac apps",
    ),
    // Out of a browser card's page, which owns every other key while focused.
    (
        "cmd+escape",
        "browser.leave",
        "the page owns every other key while focused",
    ),
    // The sidebar above the content or beside it, Gmail's reading-pane
    // toggle; Cmd+\ is what the split editors use to flip an arrangement.
    (
        "cmd+\\",
        "card.sidebar.flip",
        "what split editors use to flip an arrangement",
    ),
    // Reloads the interface, killing the shells first. Inert outside a
    // development build: it is the browser reflex key.
    (
        "cmd+r",
        "app.reload",
        "development builds only; the browser reflex key would cost every shell",
    ),
    ("cmd+shift+r", "card.rename", ""),
    // The macOS convention for preferences.
    (
        "cmd+,",
        "app.settings",
        "the macOS convention for preferences",
    ),
    ("cmd+shift+,", "app.keybindings", ""),
    // The command palette. Cmd+Shift+P is what editors use; Cmd+K is
    // deliberately NOT taken, because in a terminal that means clear.
    (
        "cmd+shift+p",
        "app.palette",
        "what editors use; Cmd+K is clear in a terminal",
    ),
    // Cmd+/ only. Cmd+Shift+/ is Cmd+? on a US layout, which macOS reserves
    // for the Help menu's search field.
    // macOS's own chord for the panel, taken over so it has exactly one
    // owner here; see app.emoji.
    (
        "cmd+ctrl+space",
        "app.emoji",
        "the system's chord, owned by us",
    ),
    (
        "cmd+/",
        "app.shortcuts",
        "Cmd+? is the Help menu's search field",
    ),
    // Three modifiers, on purpose. Quitting and reopening ends every process
    // in every card, and the daemon backend only makes that survivable, not
    // free. Nothing else in this table is this hard to press by accident.
    // Was Cmd+Ctrl+Alt+R; Ctrl and Alt together are a stretch on Ekin's
    // hand, Ctrl and Shift are not.
    (
        "cmd+ctrl+shift+r",
        "app.restart",
        "a restart nobody asked for is expensive",
    ),
    // Cmd+T with Shift: a new terminal, but WHERE is the question. A short
    // menu whose first entry letters every empty slot around the cards.
    ("cmd+shift+t", "card.place", "a new terminal, but WHERE"),
    // A letter on every card, one keystroke to any of them. "Find", loosely.
    // Cmd+H was the first choice and is Hide on macOS.
    (
        "cmd+f",
        "focus.hint",
        "\"find\", loosely: how you find a card. Cmd+H is Hide",
    ),
    // Groups. Cmd+G and Cmd+Shift+G are group / ungroup nearly everywhere
    // that has the concept.
    (
        "cmd+g",
        "group.new",
        "group / ungroup, as nearly everywhere with the concept",
    ),
    ("cmd+shift+g", "group.dissolve", ""),
    ("cmd+alt+r", "group.rename", ""),
    // Cmd+[ and Cmd+] step through every group AND the loose cards as one
    // ring, in reading order. group.focus.* stays in the palette.
    (
        "cmd+[",
        "focus.prev",
        "one ring over groups and loose cards, in reading order",
    ),
    ("cmd+]", "focus.next", ""),
    // Alt moves the card rather than the view, with "no group" as a real
    // stop, so the same pair takes a card out of a group as puts it in.
    (
        "cmd+alt+[",
        "group.card.prev",
        "moves the card, not the view; \"no group\" is a real stop",
    ),
    ("cmd+alt+]", "group.card.next", ""),
    // Third in the zoom-target series after fitCard and fitAll.
    (
        "cmd+3",
        "canvas.zoom.fitGroup",
        "third in the zoom-target series",
    ),
];

pub fn default_keymap() -> Keymap {
    DEFAULT_KEYMAP
        .iter()
        .map(|(chord, id, _)| (chord.to_string(), id.to_string()))
        .collect()
}

pub fn lookup<'a>(keymap: &'a Keymap, chord: &str) -> Option<&'a str> {
    keymap
        .iter()
        .find(|(c, _)| c == chord)
        .map(|(_, id)| id.as_str())
}

/// What the ui crate hands over per key press. `code` is the physical key
/// when the platform reports one (`KeyJ`, `Digit0`, `Equal`, `ArrowLeft`,
/// the DOM names); `key` is the character or key name it produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyPress<'a> {
    pub key: &'a str,
    pub code: Option<&'a str>,
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// The unshifted symbol each punctuation key carries, by physical key code.
/// Names are the UNSHIFTED symbol so a binding is written the way the key
/// is labelled: `cmd+shift+=`, never `cmd+shift++`.
const PUNCTUATION: [(&str, &str); 11] = [
    ("Minus", "-"),
    ("Equal", "="),
    ("BracketLeft", "["),
    ("BracketRight", "]"),
    ("Backslash", "\\"),
    ("Semicolon", ";"),
    ("Quote", "'"),
    ("Backquote", "`"),
    ("Comma", ","),
    ("Period", "."),
    ("Slash", "/"),
];

/// What Shift makes of each US-layout symbol and digit, for events that
/// arrive as the shifted character with no physical code (gpui's shape).
const SHIFTED: [(&str, &str); 21] = [
    ("_", "-"),
    ("+", "="),
    ("{", "["),
    ("}", "]"),
    ("|", "\\"),
    (":", ";"),
    ("\"", "'"),
    ("~", "`"),
    ("<", ","),
    (">", "."),
    ("?", "/"),
    (")", "0"),
    ("!", "1"),
    ("@", "2"),
    ("#", "3"),
    ("$", "4"),
    ("%", "5"),
    ("^", "6"),
    ("&", "7"),
    ("*", "8"),
    ("(", "9"),
];

/// gpui names for keys the reference spells the DOM way.
const KEY_NAMES: [(&str, &str); 4] = [
    ("left", "arrowleft"),
    ("right", "arrowright"),
    ("up", "arrowup"),
    ("down", "arrowdown"),
];

/// The physical key, independent of what the layout produced. Letters,
/// digits and punctuation are normalised; everything else keeps its name
/// (`arrowleft`, `enter`, `escape`), which is already stable.
pub fn key_name(e: &KeyPress) -> String {
    if let Some(code) = e.code {
        if let Some(letter) = code
            .strip_prefix("Key")
            .filter(|l| l.len() == 1 && l.chars().all(|c| c.is_ascii_uppercase()))
        {
            return letter.to_ascii_lowercase();
        }
        if let Some(digit) = code
            .strip_prefix("Digit")
            .filter(|d| d.len() == 1 && d.chars().all(|c| c.is_ascii_digit()))
        {
            return digit.to_string();
        }
        if let Some((_, symbol)) = PUNCTUATION.iter().find(|(c, _)| *c == code) {
            return symbol.to_string();
        }
    }
    let key = e.key.to_lowercase();
    KEY_NAMES
        .iter()
        .find(|(g, _)| *g == key)
        .map_or(key, |(_, dom)| dom.to_string())
}

/// Whether Shift was held, as far as the chord is concerned: the platform's
/// flag, or a shifted symbol arriving with the flag already cleared.
fn shifted(e: &KeyPress) -> (bool, Option<&'static str>) {
    if e.code.is_some() {
        return (e.shift, None);
    }
    match SHIFTED.iter().find(|(s, _)| *s == e.key) {
        Some((_, base)) => (true, Some(base)),
        None => (e.shift, None),
    }
}

/// Modifier order is fixed so a chord string has exactly one spelling.
pub fn chord_for(e: &KeyPress) -> String {
    let mut parts: Vec<String> = vec![];
    if e.cmd {
        parts.push("cmd".into());
    }
    if e.ctrl {
        parts.push("ctrl".into());
    }
    if e.alt {
        parts.push("alt".into());
    }
    let (shift, base) = shifted(e);
    if shift {
        parts.push("shift".into());
    }
    parts.push(base.map_or_else(|| key_name(e), String::from));
    parts.join("+")
}

/// Whether a chord may be bound at all: Cmd anything, or Ctrl plus a single
/// digit. The one place that decides.
pub fn is_allowed_chord(chord: &str) -> bool {
    let lower = chord.to_lowercase();
    let parts: Vec<&str> = lower.split('+').collect();
    if parts.contains(&"cmd") {
        return true;
    }
    parts.len() == 2
        && parts[0] == "ctrl"
        && parts[1].len() == 1
        && parts[1].chars().all(|c| c.is_ascii_digit())
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParsedKeymap {
    /// `None` is how a binding is REMOVED: a file of overrides needs a way
    /// to give a chord back to the terminal, not only to point it elsewhere.
    pub bindings: Vec<(String, Option<String>)>,
    pub errors: Vec<String>,
}

/// Reads a keybindings file into overrides.
pub fn parse_keymap(json: &Value) -> ParsedKeymap {
    let mut out = ParsedKeymap::default();
    let Some(map) = json.as_object() else {
        out.errors
            .push("keybindings.json must be a JSON object of \"chord\": \"command.id\"".into());
        return out;
    };
    for (chord, id) in map {
        let id = match id {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            _ => {
                out.errors.push(format!(
                    "\"{chord}\": value must be a command id, or null to unbind"
                ));
                continue;
            }
        };
        if !is_allowed_chord(chord) {
            out.errors.push(format!(
                "\"{chord}\": must include cmd, or be ctrl plus a digit \u{2014} other chords belong to the focused terminal"
            ));
            continue;
        }
        out.bindings.push((chord.to_lowercase(), id));
    }
    out
}

/// The defaults with a user's overrides applied. A chord mapped to `None`
/// is deleted, so the result is a plain keymap and nothing downstream has
/// to know that unbinding exists.
pub fn merge_keymap(defaults: &Keymap, overrides: &[(String, Option<String>)]) -> Keymap {
    let mut out = defaults.clone();
    for (chord, id) in overrides {
        out.retain(|(c, _)| c != chord);
        if let Some(id) = id {
            out.push((chord.clone(), id.clone()));
        }
    }
    out
}

const KEYMAP_HEADER: &[&str] = &[
    "Every key infiniterm binds, and what it runs.",
    "",
    "THIS FILE IS REWRITTEN EVERY LAUNCH. Editing it does nothing.",
    "Put your changes in keybindings.json beside it. Copy a line across to rebind a",
    "chord, or set one to null to give it back to the terminal:",
    "",
    "    { \"cmd+k\": null }",
    "",
    "Every chord must include cmd. A focused terminal consumes ctrl, alt and bare",
    "keys and has to keep consuming them, or TUI applications break.",
    "",
    "Comments and trailing commas are allowed in both files.",
];

/// The full text of `keybindings.default.json`. Each binding is commented
/// with its command's own label, so the file explains itself without a
/// second list of descriptions to keep in step with the registry.
pub fn render_keybindings_default(
    bindings: &Keymap,
    label_for: impl Fn(&str) -> Option<String>,
) -> String {
    let header: Vec<String> = KEYMAP_HEADER
        .iter()
        .map(|line| {
            if line.is_empty() {
                "//".to_string()
            } else {
                format!("// {line}")
            }
        })
        .collect();
    let body: Vec<String> = bindings
        .iter()
        .enumerate()
        .map(|(i, (chord, id))| {
            let label = label_for(id);
            let comma = if i < bindings.len() - 1 { "," } else { "" };
            let spacer = if i > 0 && label.is_some() { "\n" } else { "" };
            let comment = label.map(|l| format!("  // {l}\n")).unwrap_or_default();
            format!(
                "{spacer}{comment}  {}: {}{comma}",
                Value::String(chord.clone()),
                Value::String(id.clone())
            )
        })
        .collect();
    format!("{}\n{{\n{}\n}}\n", header.join("\n"), body.join("\n"))
}

/// Binding ids that no command answers to. A keymap entry pointing at a
/// command that was never registered fails silently (the run logs and the
/// key does nothing), which reads as "the shortcut is broken" rather than
/// "the wiring is missing". Sorted, each once.
pub fn unregistered_bindings(bindings: &Keymap, registered_ids: &[&str]) -> Vec<String> {
    let mut missing: Vec<String> = bindings
        .iter()
        .map(|(_, id)| id.clone())
        .filter(|id| !registered_ids.contains(&id.as_str()))
        .collect();
    missing.sort();
    missing.dedup();
    missing
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonc::parse_jsonc;
    use serde_json::json;

    fn key<'a>(
        key: &'a str,
        code: Option<&'a str>,
        cmd: bool,
        ctrl: bool,
        alt: bool,
        shift: bool,
    ) -> KeyPress<'a> {
        KeyPress {
            key,
            code,
            cmd,
            ctrl,
            alt,
            shift,
        }
    }

    fn km(pairs: &[(&str, &str)]) -> Keymap {
        pairs
            .iter()
            .map(|(c, i)| (c.to_string(), i.to_string()))
            .collect()
    }

    fn bindings(parsed: &ParsedKeymap) -> Vec<(&str, Option<&str>)> {
        parsed
            .bindings
            .iter()
            .map(|(c, i)| (c.as_str(), i.as_deref()))
            .collect()
    }

    #[test]
    fn builds_a_canonical_chord_string() {
        assert_eq!(
            chord_for(&key("t", None, true, false, false, false)),
            "cmd+t"
        );
        assert_eq!(
            chord_for(&key("T", None, true, false, false, true)),
            "cmd+shift+t"
        );
        assert_eq!(
            chord_for(&key("ArrowLeft", None, true, false, false, false)),
            "cmd+arrowleft"
        );
    }

    #[test]
    fn modifier_order_is_fixed_so_a_chord_has_one_spelling() {
        assert_eq!(
            chord_for(&key("x", None, true, false, true, true)),
            "cmd+alt+shift+x"
        );
    }

    #[test]
    fn accepts_cmd_chords() {
        let r = parse_keymap(&json!({"cmd+t": "card.new.terminal"}));
        assert_eq!(bindings(&r), [("cmd+t", Some("card.new.terminal"))]);
        assert!(r.errors.is_empty());
    }

    #[test]
    fn rejects_a_chord_without_cmd_because_terminals_must_keep_those_keys() {
        let r = parse_keymap(&json!({"ctrl+t": "card.new.terminal"}));
        assert!(r.bindings.is_empty());
        assert!(r.errors[0].contains("must include cmd"));
    }

    #[test]
    fn a_bad_entry_does_not_discard_the_good_ones() {
        let r = parse_keymap(&json!({"ctrl+t": "card.new.terminal", "cmd+w": "card.close"}));
        assert_eq!(bindings(&r), [("cmd+w", Some("card.close"))]);
        assert_eq!(r.errors.len(), 1);
    }

    #[test]
    fn non_string_values_are_reported_not_fatal() {
        let r = parse_keymap(&json!({"cmd+t": 42}));
        assert!(r.bindings.is_empty());
        assert!(r.errors[0].contains("cmd+t"));
    }

    #[test]
    fn a_non_object_keymap_yields_an_error_and_no_bindings() {
        assert_eq!(parse_keymap(&json!(["cmd+t"])).errors.len(), 1);
        assert!(parse_keymap(&Value::Null).bindings.is_empty());
    }

    #[test]
    fn every_default_binding_obeys_the_cmd_rule() {
        let defaults: serde_json::Map<String, Value> = DEFAULT_KEYMAP
            .iter()
            .map(|(c, i, _)| (c.to_string(), json!(i)))
            .collect();
        let r = parse_keymap(&Value::Object(defaults));
        assert!(r.errors.is_empty());
        assert_eq!(r.bindings.len(), DEFAULT_KEYMAP.len());
    }

    #[test]
    fn spots_a_binding_whose_command_was_never_registered() {
        assert_eq!(
            unregistered_bindings(
                &km(&[("cmd+a", "does.exist"), ("cmd+b", "does.not")]),
                &["does.exist"]
            ),
            ["does.not"]
        );
    }

    #[test]
    fn a_fully_wired_keymap_reports_nothing() {
        assert!(unregistered_bindings(&km(&[("cmd+a", "x")]), &["x", "y"]).is_empty());
    }

    #[test]
    fn reports_each_missing_id_once_sorted() {
        assert_eq!(
            unregistered_bindings(&km(&[("cmd+a", "b"), ("cmd+b", "a"), ("cmd+c", "b")]), &[]),
            ["a", "b"]
        );
    }

    // macOS Option+J reports `∆`; building a chord from the character means
    // an alt binding can never match.
    #[test]
    fn alt_chords_use_the_physical_key_not_what_the_layout_produced() {
        assert_eq!(
            chord_for(&key("∆", Some("KeyJ"), true, false, true, false)),
            "cmd+alt+j"
        );
        assert_eq!(
            chord_for(&key("¬", Some("KeyL"), true, false, true, false)),
            "cmd+alt+l"
        );
    }

    #[test]
    fn digits_come_from_the_physical_key_too() {
        assert_eq!(
            chord_for(&key("0", Some("Digit0"), true, false, false, false)),
            "cmd+0"
        );
    }

    #[test]
    fn punctuation_keeps_its_symbol_so_cmd_equals_does_not_become_cmd_equal() {
        assert_eq!(
            chord_for(&key("=", Some("Equal"), true, false, false, false)),
            "cmd+="
        );
        assert_eq!(
            chord_for(&key("-", Some("Minus"), true, false, false, false)),
            "cmd+-"
        );
    }

    #[test]
    fn named_keys_keep_their_name() {
        assert_eq!(
            chord_for(&key(
                "ArrowLeft",
                Some("ArrowLeft"),
                true,
                false,
                false,
                false
            )),
            "cmd+arrowleft"
        );
        assert_eq!(
            chord_for(&key("Enter", Some("Enter"), true, false, false, true)),
            "cmd+shift+enter"
        );
    }

    fn punct<'a>(code: &'a str, ch: &'a str) -> KeyPress<'a> {
        KeyPress {
            key: ch,
            code: Some(code),
            ..Default::default()
        }
    }

    #[test]
    fn names_a_punctuation_key_by_its_unshifted_symbol() {
        assert_eq!(key_name(&punct("BracketLeft", "[")), "[");
        assert_eq!(key_name(&punct("Equal", "=")), "=");
    }

    // Shift rewrites the character, so Cmd+Shift+= arrived as `cmd+shift++`
    // and the binding written `cmd+shift+=` never fired.
    #[test]
    fn key_name_ignores_what_shift_did_to_the_character() {
        assert_eq!(key_name(&punct("Equal", "+")), "=");
        assert_eq!(key_name(&punct("BracketRight", "}")), "]");
    }

    #[test]
    fn key_name_ignores_what_option_did_to_the_character() {
        assert_eq!(key_name(&punct("BracketLeft", "\u{201c}")), "[");
    }

    #[test]
    fn key_name_leaves_named_keys_alone() {
        assert_eq!(key_name(&punct("ArrowLeft", "ArrowLeft")), "arrowleft");
        assert_eq!(key_name(&punct("Enter", "Enter")), "enter");
    }

    // A file of overrides needs a way to REMOVE a binding, not only repoint one.
    #[test]
    fn null_unbinds_a_chord() {
        let r = parse_keymap(&json!({"cmd+k": null}));
        assert!(r.errors.is_empty());
        assert_eq!(bindings(&r), [("cmd+k", None)]);
        assert!(merge_keymap(&km(&[("cmd+k", "card.clear")]), &r.bindings).is_empty());
    }

    #[test]
    fn merge_keymap_layers_overrides_on_defaults() {
        let merged = merge_keymap(
            &km(&[("cmd+t", "card.new.terminal"), ("cmd+w", "card.close")]),
            &[
                ("cmd+t".into(), Some("app.palette".into())),
                ("cmd+j".into(), Some("card.clear".into())),
            ],
        );
        assert_eq!(lookup(&merged, "cmd+t"), Some("app.palette"));
        assert_eq!(lookup(&merged, "cmd+w"), Some("card.close"));
        assert_eq!(lookup(&merged, "cmd+j"), Some("card.clear"));
        assert_eq!(merged.len(), 3);
    }

    #[test]
    fn merge_keymap_leaves_the_defaults_alone() {
        let defaults = km(&[("cmd+t", "card.new.terminal")]);
        merge_keymap(&defaults, &[("cmd+t".into(), None)]);
        assert_eq!(defaults, km(&[("cmd+t", "card.new.terminal")]));
    }

    #[test]
    fn a_value_that_is_neither_a_command_id_nor_null_is_rejected() {
        assert!(parse_keymap(&json!({"cmd+k": 42})).errors[0].contains("null to unbind"));
    }

    #[test]
    fn the_generated_keymap_file_parses_back_to_the_defaults() {
        let text = render_keybindings_default(&default_keymap(), |_| None);
        let parsed = parse_jsonc(&text).unwrap();
        let back: Keymap = parsed
            .as_object()
            .unwrap()
            .iter()
            .map(|(c, i)| (c.clone(), i.as_str().unwrap().to_string()))
            .collect();
        assert_eq!(back, default_keymap());
        assert!(text.contains("REWRITTEN EVERY LAUNCH"));
        assert!(text.contains("must include cmd"));
    }

    #[test]
    fn the_generated_keymap_file_explains_each_binding_with_its_command_label() {
        let text = render_keybindings_default(&km(&[("cmd+t", "card.new.terminal")]), |_| {
            Some("New terminal card".into())
        });
        assert!(text.contains("// New terminal card"));
    }

    // The one exception to cmd-only.
    #[test]
    fn ctrl_plus_a_digit_is_allowed_ctrl_plus_anything_else_is_not() {
        assert!(is_allowed_chord("ctrl+1"));
        assert!(is_allowed_chord("ctrl+9"));
        assert!(!is_allowed_chord("ctrl+w"));
        assert!(!is_allowed_chord("ctrl+shift+1"));
        assert!(!is_allowed_chord("alt+1"));
    }

    #[test]
    fn cmd_is_allowed_with_anything() {
        assert!(is_allowed_chord("cmd+t"));
        assert!(is_allowed_chord("cmd+ctrl+alt+shift+k"));
    }

    #[test]
    fn a_keybindings_file_may_use_ctrl_plus_a_digit() {
        let r = parse_keymap(&json!({"ctrl+5": "workspace.show.5"}));
        assert!(r.errors.is_empty());
        assert_eq!(bindings(&r), [("ctrl+5", Some("workspace.show.5"))]);
    }

    #[test]
    fn a_keybindings_file_still_may_not_steal_ctrl_c() {
        assert!(
            parse_keymap(&json!({"ctrl+c": "card.close"})).errors[0].contains("ctrl plus a digit")
        );
    }

    // Every default must satisfy the rule it enforces on the user's file.
    #[test]
    fn every_default_binding_is_an_allowed_chord() {
        for (chord, _, _) in DEFAULT_KEYMAP {
            assert!(is_allowed_chord(chord), "{chord}");
        }
    }

    // Native checks for gpui's shape: no key code, letters unshifted with
    // the flag kept, punctuation and digits SHIFTED with the flag cleared,
    // arrows named `left`/`right`.
    #[test]
    fn a_gpui_keystroke_with_shifted_punctuation_names_the_unshifted_key() {
        assert_eq!(
            chord_for(&key("+", None, true, false, false, false)),
            "cmd+shift+="
        );
        assert_eq!(
            chord_for(&key("{", None, true, false, false, false)),
            "cmd+shift+["
        );
        assert_eq!(
            chord_for(&key(")", None, true, false, false, false)),
            "cmd+shift+0"
        );
        assert_eq!(
            chord_for(&key("=", None, true, false, false, false)),
            "cmd+="
        );
        assert_eq!(
            chord_for(&key("left", None, true, false, true, false)),
            "cmd+alt+arrowleft"
        );
    }

    #[test]
    fn every_default_chord_is_reachable_from_a_gpui_keystroke() {
        // Each default chord, re-derived from what gpui would deliver for it.
        for (chord, _, _) in DEFAULT_KEYMAP {
            let parts: Vec<&str> = chord.split('+').collect();
            let (mods, k) = parts.split_at(parts.len() - 1);
            let shift = mods.contains(&"shift");
            let k = k[0];
            let dom_to_gpui = [
                ("arrowleft", "left"),
                ("arrowright", "right"),
                ("arrowup", "up"),
                ("arrowdown", "down"),
            ];
            let gpui_key = dom_to_gpui
                .iter()
                .find(|(d, _)| *d == k)
                .map(|(_, g)| *g)
                .unwrap_or(k);
            let shifted_symbol = SHIFTED.iter().find(|(_, base)| *base == k).map(|(s, _)| *s);
            let (delivered, flag) = match (shift, shifted_symbol) {
                (true, Some(s)) => (s, false),
                _ => (gpui_key, shift),
            };
            let e = key(
                delivered,
                None,
                mods.contains(&"cmd"),
                mods.contains(&"ctrl"),
                mods.contains(&"alt"),
                flag,
            );
            assert_eq!(chord_for(&e), *chord);
        }
    }
}
