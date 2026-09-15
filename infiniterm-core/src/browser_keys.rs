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
const OVERRIDES: [(&str, &str); 3] = [
    ("cmd+=", "browser.zoom.in"),
    ("cmd+-", "browser.zoom.out"),
    ("cmd+0", "browser.zoom.reset"),
];

pub fn browser_override(chord: &str) -> Option<&'static str> {
    OVERRIDES
        .iter()
        .find(|(c, _)| *c == chord)
        .map(|(_, id)| *id)
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
}
