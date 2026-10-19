//! The shared pan-start predicate for canvas and card input routing.
//! Port of panMode.ts and its tests. Button numbers match the reference input API.
//! Plain left-drag belongs to card content; middle or command-left can pan.
pub const MIDDLE_BUTTON: u8 = 1;
pub fn starts_pan(button: u8, cmd_held: bool) -> bool {
    button == MIDDLE_BUTTON || (button == 0 && cmd_held)
}

/// Whether a press on a card's frame or label begins a move or a resize. Only
/// the left button does. A right press opens the native menu, which takes the
/// mouse-up with it, so a gesture begun by it never ended and the card
/// followed the pointer after the menu closed (#412).
pub fn starts_card_gesture(button: u8) -> bool {
    button == 0
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

/// What a scroll gesture over the canvas is doing: panning it, or scrolling
/// one card's body. Fixed at the first event of a trackpad gesture, so the
/// momentum that follows the lift goes where the fingers began even when the
/// pointer drifts over another card (#451).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WheelTarget {
    Pan,
    Body(String),
}

/// What one scroll event does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WheelRoute {
    /// Pan the canvas.
    Pan,
    /// The gesture already belongs to this body: deliver, and never pan
    /// (a card scrolled to its end does not hand the same gesture to the
    /// canvas halfway through).
    Body(String),
    /// A new gesture over this body: offer it the event; if the body does not
    /// use it (nothing to scroll that way), the canvas pans instead.
    TryBody(String),
}

/// The target a scroll event inherits. A trackpad gesture (`started` is the
/// first event of the fingers' touch) carries nothing over from the last
/// one; momentum events after the lift (`Moved`, no new `started`) keep it.
/// A mouse wheel (`precise` false: lines, no phases) decides every notch on
/// its own, since nothing tells its gestures apart.
pub fn carried_target(
    started: bool,
    precise: bool,
    last: Option<&WheelTarget>,
) -> Option<WheelTarget> {
    if started || !precise {
        None
    } else {
        last.cloned()
    }
}

/// Where a scroll event goes, given the target it carries and the card body
/// under the pointer (`None` over bare canvas, a frame, a label or a gap).
pub fn route_wheel(carried: Option<&WheelTarget>, body_under: Option<&str>) -> WheelRoute {
    match (carried, body_under) {
        (Some(WheelTarget::Pan), _) => WheelRoute::Pan,
        (Some(WheelTarget::Body(id)), _) => WheelRoute::Body(id.clone()),
        (None, Some(id)) => WheelRoute::TryBody(id.to_string()),
        (None, None) => WheelRoute::Pan,
    }
}

/// The viewport origin after two fingers moved by `(dx, dy)` screen pixels:
/// the canvas follows the fingers (natural scrolling), exactly as a Cmd+drag
/// pan does. gpui delivers the fingers' own motion, so a downward swipe has a
/// positive `dy` and the content moves down, which is the origin moving up.
pub fn pan_origin(x: f64, y: f64, scale: f64, dx: f64, dy: f64) -> (f64, f64) {
    (x - dx / scale, y - dy / scale)
}

/// Whether a list scrolled to `pos` (0 to `max`, the last position it can
/// reach) can move the way a scroll delta asks: fingers down (`delta` > 0)
/// reveal what is above (`pos` falls), fingers up reveal what is below
/// (`pos` rises). A body asks this before taking a scroll so that a card
/// with nothing to show that way leaves the gesture to the canvas.
pub fn can_scroll(pos: usize, max: usize, delta: f64) -> bool {
    (delta > 0. && pos > 0) || (delta < 0. && pos < max)
}

#[cfg(test)]
mod tests {
    #[test]
    fn two_fingers_moving_down_move_the_canvas_down() {
        // Fingers down: dy > 0. The content follows, so the origin moves up.
        assert_eq!(pan_origin(100., 100., 1., 0., 30.), (100., 70.));
        // Fingers up: dy < 0, the content moves up, the origin down.
        assert_eq!(pan_origin(100., 100., 1., 0., -30.), (100., 130.));
        // Fingers right: dx > 0, the content moves right, the origin left.
        assert_eq!(pan_origin(100., 100., 1., 20., 0.), (80., 100.));
    }

    #[test]
    fn a_pan_tracks_the_fingers_at_any_zoom() {
        // At 50% a world unit is half a pixel, so 30 pixels is 60 units.
        assert_eq!(pan_origin(0., 0., 0.5, 0., 30.), (0., -60.));
        assert_eq!(pan_origin(0., 0., 2., 0., 30.), (0., -15.));
    }

    #[test]
    fn a_list_scrolls_only_where_it_has_room() {
        // At the top: fingers down would reveal what is above: nothing.
        assert!(!can_scroll(0, 10, 5.));
        assert!(can_scroll(0, 10, -5.));
        // At the end: the other way round.
        assert!(can_scroll(10, 10, 5.));
        assert!(!can_scroll(10, 10, -5.));
        // In the middle: both ways. No movement asks for nothing.
        assert!(can_scroll(4, 10, 5.) && can_scroll(4, 10, -5.));
        assert!(!can_scroll(4, 10, 0.));
        // A list that fits (max 0) never scrolls.
        assert!(!can_scroll(0, 0, 5.) && !can_scroll(0, 0, -5.));
    }

    #[test]
    fn a_new_gesture_forgets_the_last_target_and_momentum_keeps_it() {
        let body = WheelTarget::Body("c1".into());
        // The fingers touch down: the old target is gone.
        assert_eq!(carried_target(true, true, Some(&body)), None);
        // Moves and the momentum after the lift keep it.
        assert_eq!(carried_target(false, true, Some(&body)), Some(body.clone()));
        assert_eq!(
            carried_target(false, true, Some(&WheelTarget::Pan)),
            Some(WheelTarget::Pan)
        );
        // A mouse wheel has no phases: every notch decides again.
        assert_eq!(carried_target(false, false, Some(&body)), None);
    }

    #[test]
    fn a_scroll_pans_over_bare_canvas_and_offers_a_body_the_first_chance() {
        // Nothing under the pointer, nothing carried: pan.
        assert_eq!(route_wheel(None, None), WheelRoute::Pan);
        // A body under a new gesture is asked first.
        assert_eq!(
            route_wheel(None, Some("c1")),
            WheelRoute::TryBody("c1".into())
        );
        // A pan in progress stays a pan over a card (momentum drifting over
        // one must not start scrolling it)...
        assert_eq!(
            route_wheel(Some(&WheelTarget::Pan), Some("c2")),
            WheelRoute::Pan
        );
        // ...and a body's gesture stays that body's over the canvas.
        assert_eq!(
            route_wheel(Some(&WheelTarget::Body("c1".into())), None),
            WheelRoute::Body("c1".into())
        );
        assert_eq!(
            route_wheel(Some(&WheelTarget::Body("c1".into())), Some("c2")),
            WheelRoute::Body("c1".into())
        );
    }

    #[test]
    fn only_a_left_press_starts_a_card_gesture() {
        assert!(starts_card_gesture(0));
        assert!(!starts_card_gesture(1), "middle pans the canvas instead");
        assert!(!starts_card_gesture(2), "right opens the menu (#412)");
    }

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
