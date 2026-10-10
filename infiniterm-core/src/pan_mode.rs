//! The shared pan-start predicate for canvas and card input routing.
//! Port of panMode.ts and its tests. Button numbers match the reference input API.
//! Plain left-drag belongs to card content; middle or command-left can pan.
pub const MIDDLE_BUTTON: u8 = 1;
pub fn starts_pan(button: u8, cmd_held: bool) -> bool {
    button == MIDDLE_BUTTON || (button == 0 && cmd_held)
}

/// The right button while the left is held on empty canvas fits everything
/// (Cmd+2 for the mouse, a pinch-out in two fingers). Ekin found it by
/// accident: a left click then a right click on bare canvas counted as a
/// double-click, which fits all, but only when the two were nearly
/// simultaneous. Holding left is the deliberate form. Only a press that
/// began on empty canvas counts: a left press on a card is a selection or a
/// drag, and a right click must not cut into it.
pub fn chord_fits_all(button: u8, left_held_on_canvas: bool) -> bool {
    button == 2 && left_held_on_canvas
}

/// A double-click in a card's body fits the card when the body is drawn as
/// bars (too small to read, or over the frame's glyph budget). Readable, the
/// double-click is the program's (a word selected in a terminal); as bars
/// there is no word to select, and zooming in is what the click is for.
pub fn double_click_fits(click_count: usize, drawn_as_bars: bool) -> bool {
    click_count >= 2 && drawn_as_bars
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_double_click_fits_only_a_card_drawn_as_bars() {
        assert!(double_click_fits(2, true));
        assert!(double_click_fits(3, true));
        assert!(!double_click_fits(1, true), "one click is only a focus");
        assert!(
            !double_click_fits(2, false),
            "readable text keeps its double-click"
        );
    }

    #[test]
    fn middle_with_or_without_cmd() {
        assert!(starts_pan(MIDDLE_BUTTON, false));
        assert!(starts_pan(MIDDLE_BUTTON, true));
    }
    #[test]
    fn left_only_with_cmd() {
        assert!(starts_pan(0, true));
        assert!(!starts_pan(0, false));
    }
    #[test]
    fn plain_left_never_pans() {
        assert!(!starts_pan(0, false));
    }
    #[test]
    fn right_while_left_holds_the_canvas_fits_all() {
        assert!(chord_fits_all(2, true));
        assert!(
            !chord_fits_all(2, false),
            "a plain right click is a right click"
        );
        assert!(
            !chord_fits_all(0, true),
            "the left press itself does nothing"
        );
        assert!(!chord_fits_all(MIDDLE_BUTTON, true));
    }
    #[test]
    fn right_never_pans() {
        assert!(!starts_pan(2, true));
        assert!(!starts_pan(2, false));
    }
}
