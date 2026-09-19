//! Chords that mean something else inside a focused browser card. Port of
//! browserKeys.ts and its test.
//!
//! The page keeps its own size whatever the canvas zoom is, so the zoom
//! chords, which would otherwise zoom the canvas around a page that does
//! not follow, zoom the PAGE instead: what the same keys do in Safari. The
//! app consults this before the keymap when the focused card is a browser.
//! In the port the page is a CEF texture that does scale with the canvas,
//! but the page zoom is still the thing a person means by Cmd+= over a
//! browser, so the rule stays.
const OVERRIDES: [(&str, &str); 7] = [
    ("cmd+=", "browser.zoom.in"),
    ("cmd+-", "browser.zoom.out"),
    ("cmd+0", "browser.zoom.reset"),
    // Safari and Chrome's back and forward. Outside a browser card these
    // step through groups (`focus.prev` / `focus.next`), which is the right
    // thing everywhere the page is not what you are looking at.
    // Outside a browser card Cmd+R reloads the whole app (development
    // builds only); over a page it is the page that should come back.
    ("cmd+r", "browser.reload"),
    // Outside a browser card Cmd+F letters every card; over a page, find is
    // what the key means everywhere else on the Mac.
    ("cmd+f", "browser.find"),
    ("cmd+[", "browser.back"),
    ("cmd+]", "browser.forward"),
];

pub fn browser_override(chord: &str) -> Option<&'static str> {
    OVERRIDES
        .iter()
        .find(|(c, _)| *c == chord)
        .map(|(_, id)| *id)
}

/// Chords a LOCKED browser card claims beyond `browser_override`'s
/// always-on list: the ones a real Chrome window binds to its own tab
/// strip rather than to a page. `Ctrl+1..9` is deliberately absent —
/// workspace switching, never a Chrome shortcut, is unaffected by lock.
const LOCK_OVERRIDES: [(&str, &str); 12] = [
    ("cmd+t", "browser.tab.new"),
    ("cmd+w", "browser.tab.close"),
    ("cmd+shift+t", "browser.tab.reopenClosed"),
    ("cmd+1", "browser.tab.jump.1"),
    ("cmd+2", "browser.tab.jump.2"),
    ("cmd+3", "browser.tab.jump.3"),
    ("cmd+4", "browser.tab.jump.4"),
    ("cmd+5", "browser.tab.jump.5"),
    ("cmd+6", "browser.tab.jump.6"),
    ("cmd+7", "browser.tab.jump.7"),
    ("cmd+8", "browser.tab.jump.8"),
    ("cmd+9", "browser.tab.jump.last"),
];

pub fn lock_override(chord: &str) -> Option<&'static str> {
    LOCK_OVERRIDES
        .iter()
        .find(|(c, _)| *c == chord)
        .map(|(_, id)| *id)
        .or(match chord {
            "ctrl+tab" => Some("browser.tab.next"),
            "ctrl+shift+tab" => Some("browser.tab.prev"),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zoom_chords_zoom_the_page_inside_a_browser_card_and_nothing_else_changes() {
        assert_eq!(browser_override("cmd+="), Some("browser.zoom.in"));
        assert_eq!(browser_override("cmd+-"), Some("browser.zoom.out"));
        assert_eq!(browser_override("cmd+0"), Some("browser.zoom.reset"));
        assert_eq!(browser_override("cmd+w"), None);
    }

    // The page's history takes the chord it has in every browser; stepping
    // between groups keeps it everywhere else.
    #[test]
    fn the_bracket_chords_are_back_and_forward_inside_a_page() {
        assert_eq!(browser_override("cmd+["), Some("browser.back"));
        assert_eq!(browser_override("cmd+]"), Some("browser.forward"));
        assert_eq!(
            browser_override("cmd+alt+["),
            None,
            "moving a card between groups is not the page's"
        );
    }

    #[test]
    fn the_lock_table_covers_new_close_reopen_jump_and_tab_stepping() {
        assert_eq!(lock_override("cmd+t"), Some("browser.tab.new"));
        assert_eq!(lock_override("cmd+w"), Some("browser.tab.close"));
        assert_eq!(
            lock_override("cmd+shift+t"),
            Some("browser.tab.reopenClosed")
        );
        assert_eq!(lock_override("cmd+1"), Some("browser.tab.jump.1"));
        assert_eq!(lock_override("cmd+8"), Some("browser.tab.jump.8"));
        assert_eq!(lock_override("cmd+9"), Some("browser.tab.jump.last"));
        assert_eq!(lock_override("ctrl+tab"), Some("browser.tab.next"));
        assert_eq!(lock_override("ctrl+shift+tab"), Some("browser.tab.prev"));
    }

    #[test]
    fn ctrl_digit_is_not_in_the_lock_table_workspace_switching_stays_the_apps() {
        assert_eq!(lock_override("ctrl+5"), None);
    }

    #[test]
    fn a_chord_the_lock_table_does_not_know_falls_through_to_the_page_zoom_list() {
        // cmd+= is already browser_override's, unaffected by locking
        assert_eq!(lock_override("cmd+="), None);
        assert_eq!(browser_override("cmd+="), Some("browser.zoom.in"));
    }
}
