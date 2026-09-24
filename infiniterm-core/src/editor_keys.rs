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

/// Chords a LOCKED editor card claims, the browser's lock table with the
/// editor's tab commands behind it (`browser_keys::lock_override` is the
/// model): Cmd+T for a tab instead of a card, Cmd+W for the tab, Cmd+1..9
/// to jump, Cmd+Shift+[ and ] to step, Cmd+Shift+T to reopen. Same chords
/// on both kinds, so locking means one thing.
pub fn lock_override(chord: &str) -> Option<&'static str> {
    Some(match chord {
        "cmd+t" => "editor.tab.new",
        "cmd+w" => "editor.tab.close",
        "cmd+shift+t" => "editor.tab.reopenClosed",
        "cmd+shift+]" => "editor.tab.next",
        "cmd+shift+[" => "editor.tab.prev",
        "cmd+1" => "editor.tab.jump.1",
        "cmd+2" => "editor.tab.jump.2",
        "cmd+3" => "editor.tab.jump.3",
        "cmd+4" => "editor.tab.jump.4",
        "cmd+5" => "editor.tab.jump.5",
        "cmd+6" => "editor.tab.jump.6",
        "cmd+7" => "editor.tab.jump.7",
        "cmd+8" => "editor.tab.jump.8",
        "cmd+9" => "editor.tab.jump.last",
        // Not a tab command either: `editor.goToLine` opens the model's
        // prompt (Batch 1, 2026-09-24), so it must be resolved here like
        // Cmd+S rather than left to fall through to the body, which has no
        // prompt of its own to open.
        "ctrl+g" => "editor.goToLine",
        // Not a tab command, but the one app chord a locked editor cannot
        // do without: an unmatched chord falls through to the body, which
        // has no save of its own, so Cmd+S saved nothing until the lock
        // was dropped. Zed and VS Code save from inside the text; so here.
        "cmd+s" => "card.save",
        _ => return None,
    })
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
        assert_eq!(lock_override("cmd+t"), Some("editor.tab.new"));
        assert_eq!(lock_override("cmd+9"), Some("editor.tab.jump.last"));
        assert_eq!(lock_override("cmd+shift+]"), Some("editor.tab.next"));
        assert_eq!(lock_override("cmd+s"), Some("card.save")); // saves while locked
        assert_eq!(lock_override("ctrl+g"), Some("editor.goToLine"));
        assert_eq!(lock_override("cmd+k"), None);
    }

    #[test]
    fn and_nothing_the_app_needs_everywhere() {
        assert!(!editor_keeps("cmd+alt+arrowleft")); // focus move
        assert!(!editor_keeps("cmd+w"));
        assert!(!editor_keeps("cmd+s"));
    }
}
