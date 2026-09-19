//! Chords an editor card keeps for itself. Port of editorKeys.ts and its
//! tests.
//!
//! The keymap follows the text-field reflex, and inside an editor the text
//! field is real: Cmd+Shift+Arrow selects to the boundary there, as in
//! every Mac text field. The app binds the same chord to extending the CARD
//! selection and both cannot have it; when the active card is an editor the
//! editor wins and card extend is left to Shift+click. Find, find next and
//! previous, replace and comment are what every Mac editor makes those
//! chords; hints, groups and the shortcuts panel give them up here. The
//! app consults this before the keymap. Chords are spelled as the keymap
//! normalises them.
/// Undo and redo are the buffer's inside an editor; the layout's undo
/// (`layout.undo`) is for the other cards.
const KEPT: [&str; 11] = [
    "cmd+shift+arrowleft",
    "cmd+shift+arrowright",
    "cmd+shift+arrowup",
    "cmd+shift+arrowdown",
    "cmd+f",
    "cmd+g",
    "cmd+shift+g",
    "cmd+alt+f",
    "cmd+/",
    "cmd+z",
    "cmd+shift+z",
];

pub fn editor_keeps(chord: &str) -> bool {
    KEPT.contains(&chord)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_editor_keeps_the_select_to_boundary_chords() {
        assert!(editor_keeps("cmd+shift+arrowleft"));
        assert!(editor_keeps("cmd+shift+arrowdown"));
        assert!(editor_keeps("cmd+f")); // find, not hints
        assert!(editor_keeps("cmd+/")); // comment, not the shortcuts panel
        assert!(editor_keeps("cmd+z")); // the buffer's undo, not the layout's
    }

    #[test]
    fn and_nothing_the_app_needs_everywhere() {
        assert!(!editor_keeps("cmd+alt+arrowleft")); // focus move
        assert!(!editor_keeps("cmd+w"));
        assert!(!editor_keeps("cmd+s"));
    }
}
