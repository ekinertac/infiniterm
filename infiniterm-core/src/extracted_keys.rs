//! The app chords that work in a terminal card taken into a window of its own
//! (#369, after #356).
//!
//! The window has no canvas, no palette and no other cards, so it takes the
//! few commands that mean something for one terminal: the font size, clear
//! and visual mode. A chord is looked up in the LIVE keymap (a rebinding in
//! keybindings.json works there too) and runs only when its command is on
//! this list; every other chord stays the body's, as before. Nothing here is
//! a new binding.
//!
//! Called by `infiniterm-ui/src/own_window.rs`. Related: `browser_keys.rs`
//! (the same idea for a page), `keymap.rs` (`lookup`).
//!
//! Left out on purpose: find (the window has no find bar yet), and everything
//! about the canvas, cards, workspaces, splits and the palette.
use crate::keymap::{lookup, Keymap};

/// Commands the window runs from a chord. `card.clear` and `terminal.visual`
/// act on the focused card, so the caller points them at the window's card.
pub const COMMANDS: [&str; 4] = [
    "terminal.font.bigger",
    "terminal.font.smaller",
    "card.clear",
    "terminal.visual",
];

/// The command a chord runs in the window, or `None` when the chord is the
/// terminal's.
pub fn command_for<'a>(keymap: &'a Keymap, chord: &str) -> Option<&'a str> {
    lookup(keymap, chord).filter(|id| COMMANDS.contains(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::default_keymap;

    #[test]
    fn the_default_chords_for_font_clear_and_visual_run_in_the_window() {
        let k = default_keymap();
        assert_eq!(command_for(&k, "cmd+="), Some("terminal.font.bigger"));
        assert_eq!(command_for(&k, "cmd+-"), Some("terminal.font.smaller"));
        assert_eq!(command_for(&k, "cmd+k"), Some("card.clear"));
        assert_eq!(command_for(&k, "cmd+shift+c"), Some("terminal.visual"));
    }

    #[test]
    fn a_canvas_chord_stays_the_terminals() {
        let k = default_keymap();
        for chord in [
            "cmd+t",
            "cmd+d",
            "cmd+1",
            "cmd+0",
            "cmd+f",
            "cmd+shift+p",
            "cmd+alt+arrowleft",
            "cmd+arrowleft",
            "cmd+c",
            "cmd+v",
        ] {
            assert_eq!(command_for(&k, chord), None, "{chord}");
        }
    }

    #[test]
    fn a_rebinding_moves_the_chord_with_it() {
        let mut k = default_keymap();
        k.retain(|(c, _)| c != "cmd+k");
        k.push(("cmd+shift+k".into(), "card.clear".into()));
        assert_eq!(command_for(&k, "cmd+k"), None);
        assert_eq!(command_for(&k, "cmd+shift+k"), Some("card.clear"));
    }
}
