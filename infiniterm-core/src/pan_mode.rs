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

/// The longest pause inside one scroll gesture. The OS sends a trackpad's
/// events (and its momentum after the lift) every display refresh, 8 to 16 ms
/// apart, so a pause longer than this is a NEW gesture. It matters for a
/// device that sends pixel scrolls with no touch phase (a smooth-scrolling
/// mouse with a vendor driver): nothing ever says `started` for it, so
/// without this the target one burst chose (pan, over empty canvas) was kept
/// for every later burst, over any card, and a terminal could not be scrolled
/// with that mouse (#485). Long enough that a pause mid-gesture while the
/// fingers rest does not re-decide on the same card, short enough that two
/// flicks of a hand never share a target.
pub const WHEEL_GESTURE_GAP_MS: f64 = 150.;

/// The target a scroll event inherits. A trackpad gesture (`started` is the
/// first event of the fingers' touch) carries nothing over from the last
/// one; momentum events after the lift (`Moved`, no new `started`) keep it,
/// as long as they follow without a pause (`gap_ms`, the time since the
/// previous scroll event, at most `WHEEL_GESTURE_GAP_MS`). A mouse wheel
/// (`precise` false: lines, no phases) decides every notch on its own, since
/// nothing tells its gestures apart.
pub fn carried_target(
    started: bool,
    precise: bool,
    gap_ms: f64,
    last: Option<&WheelTarget>,
) -> Option<WheelTarget> {
    if started || !precise || gap_ms > WHEEL_GESTURE_GAP_MS {
        None
    } else {
        last.cloned()
    }
}

/// Whether the scroll gesture an event belongs to comes from a trackpad.
/// Only a trackpad opens a gesture with a `Started` touch phase; a mouse
/// wheel, notched or smooth, never does (Ekin's mouse sends pixel deltas with
/// no phase at all, #485). A mouse must not pan the canvas: it has no way to
/// mean it, and an accidental pan is worse than none (Ekin, 2026-10-19, after
/// 0.6.0). A gesture that begins without a `Started` event is therefore not a
/// trackpad's; one that carries on keeps what its first event said.
pub fn gesture_is_trackpad(started: bool, begins_here: bool, was: bool) -> bool {
    if started {
        true
    } else if begins_here {
        false
    } else {
        was
    }
}

/// Whether a gesture's first event is too small to say what the fingers mean
/// to do. A real trackpad often opens a gesture with an event that has no
/// movement at all, or a sliver of sideways drift with no vertical part; a
/// terminal reads that as "nothing to scroll that way" and the whole gesture
/// would pan the canvas (the bug of build 998: a terminal with history could
/// not be scrolled). Such an event decides nothing: the first event with real
/// movement chooses, with the same rule as before.
pub fn defers_target(dx: f64, dy: f64) -> bool {
    dy.abs() < 1. && dx.abs() < 3.
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

/// Where a two-finger pan is in its life. A trackpad gesture is a SESSION, the
/// way a Cmd+drag is: it starts when the fingers touch, follows them, and
/// ends when they lift, with the app's own fling (`momentum::velocity_from`
/// and the animator) instead of the OS's. Why not the OS's momentum: macOS
/// sends it as more scroll events, one per display refresh, each handled
/// alone with nothing asking for a frame (it was laggy on the MacBook, while
/// the drag, whose session keeps `needs_frame` true, was not), and a second
/// fling of ours on top would stack with it.
///
/// What gpui reports (gpui 0.2 `platform/mac/events.rs`): the event's `phase`
/// maps Began and MayBegin to Started, Ended to Ended, and everything else to
/// Moved. The OS's momentum events have no finger phase (their own
/// `momentumPhase` is not exposed), so they arrive as `Moved` events AFTER an
/// `Ended`, with no new `Started`. That is how they are told apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanSession {
    /// No two-finger pan going.
    #[default]
    Idle,
    /// The fingers are down and the canvas follows them.
    Active,
    /// The fingers lifted: what still arrives is the OS's momentum, ignored.
    Momentum,
}

/// What one scroll event does to a pan session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanStep {
    /// Start a session at this event, and move by its delta.
    Begin,
    /// Move by the delta.
    Advance,
    /// The fingers lifted: fling and end the session. The delta (an Ended
    /// event carries none, or a last one) is applied first.
    Finish,
    /// Nothing: the OS's momentum after the lift, or an Ended with nothing
    /// going.
    Ignore,
    /// A mouse wheel, or a device with no phases: move by the delta alone,
    /// with no session.
    Single,
}

/// One scroll event routed to PAN: the session's next state and what to do.
/// `precise` is a pixel delta (a trackpad); a mouse wheel's line deltas have
/// no phases to tell gestures apart, so every notch is a step of its own.
pub fn pan_step(
    state: PanSession,
    started: bool,
    ended: bool,
    precise: bool,
) -> (PanSession, PanStep) {
    if !precise {
        return (PanSession::Idle, PanStep::Single);
    }
    match (state, started, ended) {
        // The fingers touch (MayBegin then Began both say Started: the
        // second continues the session the first began).
        (PanSession::Active, true, _) => (PanSession::Active, PanStep::Advance),
        (_, true, _) => (PanSession::Active, PanStep::Begin),
        (PanSession::Active, false, true) => (PanSession::Momentum, PanStep::Finish),
        // An Ended nothing was going for (the gesture went to a card).
        (_, false, true) => (state, PanStep::Ignore),
        (PanSession::Active, false, false) => (PanSession::Active, PanStep::Advance),
        (PanSession::Momentum, false, false) => (PanSession::Momentum, PanStep::Ignore),
        // Moved with no Started before it and no lift: a device that sends
        // pixels without phases. Step it like a wheel.
        (PanSession::Idle, false, false) => (PanSession::Idle, PanStep::Single),
    }
}

/// How long a session may go without a scroll event before it is ended as if
/// the fingers had lifted: macOS can cancel a gesture (a `Cancelled` phase
/// gpui reports as Moved) and then no Ended ever comes, which would hold the
/// frame loop open for good.
pub const PAN_SESSION_IDLE_MS: f64 = 120.;

/// Whether a session whose last event was at `last_ms` has gone quiet.
pub fn pan_session_stale(state: PanSession, last_ms: f64, now_ms: f64) -> bool {
    state == PanSession::Active && now_ms - last_ms > PAN_SESSION_IDLE_MS
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
    fn only_a_gesture_that_started_with_a_touch_is_a_trackpads() {
        // The fingers touch down: a trackpad.
        assert!(gesture_is_trackpad(true, true, false));
        // A burst that begins with no touch phase: a mouse, even smooth.
        assert!(!gesture_is_trackpad(false, true, true));
        // The rest of a gesture keeps what its first event said, momentum
        // after the lift included.
        assert!(gesture_is_trackpad(false, false, true));
        assert!(!gesture_is_trackpad(false, false, false));
    }

    #[test]
    fn the_first_event_of_a_gesture_decides_nothing_until_it_moves() {
        // A real trackpad opens with an event with no movement, or a sliver
        // of sideways drift: it must not pick pan for the whole gesture.
        assert!(defers_target(0., 0.));
        assert!(defers_target(2., 0.));
        assert!(defers_target(-2.9, 0.5));
        // Vertical movement, or a real sideways swipe, decides.
        assert!(!defers_target(0., 1.));
        assert!(!defers_target(0., -8.));
        assert!(!defers_target(3., 0.));
        assert!(!defers_target(-12., 0.));
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
        assert_eq!(carried_target(true, true, 8., Some(&body)), None);
        // Moves and the momentum after the lift keep it (events 8 to 16 ms
        // apart).
        assert_eq!(
            carried_target(false, true, 8., Some(&body)),
            Some(body.clone())
        );
        assert_eq!(
            carried_target(false, true, 16., Some(&WheelTarget::Pan)),
            Some(WheelTarget::Pan)
        );
        // A mouse wheel has no phases: every notch decides again.
        assert_eq!(carried_target(false, false, 8., Some(&body)), None);
    }

    #[test]
    fn a_pause_longer_than_a_gesture_forgets_the_target() {
        // A smooth mouse with a vendor driver sends pixel scrolls with no
        // phase: nothing says `started`. The burst over empty canvas chose
        // pan; 300 ms later a burst over a terminal must decide afresh
        // (#485: it kept panning and the terminal could not be scrolled).
        assert_eq!(
            carried_target(false, true, 300., Some(&WheelTarget::Pan)),
            None
        );
        assert_eq!(
            carried_target(
                false,
                true,
                WHEEL_GESTURE_GAP_MS + 1.,
                Some(&WheelTarget::Body("c1".into()))
            ),
            None
        );
        // The pause itself is the limit, inclusive: events exactly that far
        // apart still belong to one gesture.
        assert_eq!(
            carried_target(false, true, WHEEL_GESTURE_GAP_MS, Some(&WheelTarget::Pan)),
            Some(WheelTarget::Pan)
        );
        // And the burst after the pause, over a card, is offered to the card.
        let fresh = carried_target(false, true, 300., Some(&WheelTarget::Pan));
        assert_eq!(
            route_wheel(fresh.as_ref(), Some("c1")),
            WheelRoute::TryBody("c1".into())
        );
        // Nothing remembered: nothing carried, however close the events.
        assert_eq!(carried_target(false, true, 8., None), None);
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

    /// A trackpad flick as gpui reports it: Started, Moved..., Ended, then the
    /// OS's momentum as Moved with no new Started.
    fn run(events: &[(bool, bool)]) -> Vec<PanStep> {
        let mut state = PanSession::Idle;
        events
            .iter()
            .map(|&(started, ended)| {
                let (next, step) = pan_step(state, started, ended, true);
                state = next;
                step
            })
            .collect()
    }

    #[test]
    fn a_flick_is_one_session_and_the_os_momentum_after_it_is_ignored() {
        use PanStep::*;
        let steps = run(&[
            (true, false),  // fingers down
            (false, false), // moving
            (false, false),
            (false, true),  // fingers lift
            (false, false), // the OS's momentum: Moved, no Started
            (false, false),
            (false, false),
        ]);
        assert_eq!(
            steps,
            vec![Begin, Advance, Advance, Finish, Ignore, Ignore, Ignore],
            "one fling of ours, never a second from the OS's momentum"
        );
    }

    #[test]
    fn a_new_touch_after_momentum_starts_a_new_session() {
        use PanStep::*;
        let steps = run(&[
            (true, false),
            (false, true),
            (false, false), // momentum
            (true, false),  // fingers touch again
            (false, false),
            (false, true),
        ]);
        assert_eq!(steps, vec![Begin, Finish, Ignore, Begin, Advance, Finish]);
    }

    #[test]
    fn maybegin_then_began_is_one_start() {
        use PanStep::*;
        // gpui reports MayBegin and Began both as Started.
        assert_eq!(
            run(&[(true, false), (true, false), (false, false)]),
            vec![Begin, Advance, Advance]
        );
    }

    #[test]
    fn an_ended_with_nothing_going_does_nothing() {
        // The gesture went to a card, or the lift of an already finished one.
        assert_eq!(run(&[(false, true)]), vec![PanStep::Ignore]);
        let (state, step) = pan_step(PanSession::Momentum, false, true, true);
        assert_eq!((state, step), (PanSession::Momentum, PanStep::Ignore));
    }

    #[test]
    fn a_mouse_wheel_has_no_session_and_never_leaves_idle() {
        for state in [PanSession::Idle, PanSession::Active, PanSession::Momentum] {
            let (next, step) = pan_step(state, false, false, false);
            assert_eq!((next, step), (PanSession::Idle, PanStep::Single));
        }
        // Pixel deltas with no phases at all (a device that sends none): each
        // event is a step of its own.
        let (next, step) = pan_step(PanSession::Idle, false, false, true);
        assert_eq!((next, step), (PanSession::Idle, PanStep::Single));
    }

    #[test]
    fn a_gesture_macos_cancelled_ends_by_going_quiet() {
        assert!(!pan_session_stale(
            PanSession::Active,
            1000.,
            1000. + PAN_SESSION_IDLE_MS
        ));
        assert!(pan_session_stale(
            PanSession::Active,
            1000.,
            1000. + PAN_SESSION_IDLE_MS + 1.
        ));
        // Only an active session can be stale.
        assert!(!pan_session_stale(PanSession::Momentum, 0., 10_000.));
        assert!(!pan_session_stale(PanSession::Idle, 0., 10_000.));
    }

    #[test]
    fn two_fingers_move_the_canvas_the_way_a_drag_does() {
        // drag_pan: viewport -= (pointer delta) / scale. A virtual point that
        // advances by the fingers' delta, fed to the same code, must give the
        // same origin as `pan_origin` (natural direction: fingers down, the
        // origin goes up).
        let (x, y) = pan_origin(100., 100., 2., 10., -20.);
        assert_eq!((x, y), (95., 110.));
        let drag = (100. - 10. / 2., 100. - (-20.) / 2.);
        assert_eq!((x, y), drag);
    }
}
