//! Cmd+Alt + two fingers (or a scroll wheel) and Cmd+Shift + two fingers
//! sideways as navigation: the pure half.
//!
//! Where it fits: `infiniterm-ui/src/input.rs` (`AppView::wheel`) feeds the
//! scroll events that carry Cmd+Alt (focus) or Cmd+Shift (workspace, clearly
//! horizontal ones only: `claims_workspace`) to `NavState::step`, and runs the
//! commands it answers with through the registry like a chord does:
//! `focus.move.*` (Cmd+Alt+arrows) and `workspace.next` / `workspace.prev`
//! (Ekin, 2026-10-20, after the trackpad panning of 0.6.0 and 0.6.1 was
//! removed: "no panning, only good old regular scroll in cards", and these two
//! instead; Cmd+scroll and the pinch keep zooming, a vertical Cmd+Shift scroll
//! still zooms). Nothing here knows gpui; the tests are tables.
//!
//! Rules that are not obvious:
//! - A swipe's movement is ACCUMULATED and the focus moves ONE card per swipe
//!   (Ekin, 2026-10-20: "Cmd+Alt+two fingers should switch one card per
//!   swipe"): the step comes once `STEP_PX` of movement has gathered, and the
//!   rest of the gesture, a long swipe or a flick, does nothing more. A notched
//!   wheel's notch (`lines`) counts as `STEP_PX`, and a burst of notches less
//!   than `GESTURE_GAP_MS` apart is one swipe.
//! - The OS keeps sending momentum events after the fingers lift (phase
//!   `Moved`, no new `Started`, after an `Ended`). They are IGNORED, or one flick
//!   would keep walking through the canvas. A device with no phases (a mouse)
//!   never says `Ended`, so nothing is ever ignored for it; and the "lifted"
//!   state ends by itself after a pause (`GESTURE_GAP_MS`), so a wheel used
//!   after a trackpad is not mistaken for momentum.
//! - A workspace switch needs a clearly horizontal swipe, happens at most once
//!   per gesture and not within `WORKSPACE_GAP_MS` of the last one, so a swipe
//!   cannot run through every workspace.
//! - Direction (`focus_dir`, `workspace_dir`) follows the fingers: the content
//!   follows them like a page being swiped, so fingers moving left go RIGHT
//!   (next card, next workspace). It is the one place to flip.
//!
//! Related: `docs/agent/canvas-interaction.md`, `shortcuts::GESTURES`,
//! `usage_log::MOUSE_GESTURES` (the gesture ids `mouse.focus.scroll` and
//! `mouse.workspace.scroll`).

/// Pixels of swipe that take a gesture's one focus step: enough that a touch
/// of the fingers does not move the focus, little enough that a short swipe
/// does.
pub const STEP_PX: f64 = 60.;
/// Pixels of clearly horizontal swipe that switch a workspace.
pub const WORKSPACE_PX: f64 = 120.;
/// The least time between two workspace switches.
pub const WORKSPACE_GAP_MS: f64 = 350.;
/// A pause longer than this ends a gesture whatever the device says (also the
/// end of the "fingers lifted" state): momentum arrives every few ms.
pub const GESTURE_GAP_MS: f64 = 150.;
/// A workspace swipe's horizontal movement must be this many times its
/// vertical movement to count.
pub const HORIZONTAL_RATIO: f64 = 2.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The fingers touched down (the first event of a trackpad gesture).
    Started,
    Moved,
    /// The fingers lifted; what follows is momentum.
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusDir {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceDir {
    Next,
    Prev,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavAction {
    Focus(FocusDir),
    Workspace(WorkspaceDir),
}

impl NavAction {
    /// The registered command that does it.
    pub fn command_id(self) -> &'static str {
        match self {
            NavAction::Focus(FocusDir::Left) => "focus.move.left",
            NavAction::Focus(FocusDir::Right) => "focus.move.right",
            NavAction::Focus(FocusDir::Up) => "focus.move.up",
            NavAction::Focus(FocusDir::Down) => "focus.move.down",
            NavAction::Workspace(WorkspaceDir::Next) => "workspace.next",
            NavAction::Workspace(WorkspaceDir::Prev) => "workspace.prev",
        }
    }

    /// The id `usage.log` records the gesture under.
    pub fn gesture_id(self) -> &'static str {
        match self {
            NavAction::Focus(_) => "mouse.focus.scroll",
            NavAction::Workspace(_) => "mouse.workspace.scroll",
        }
    }
}

/// The direction of a focus step for a swipe's movement: the fingers (as the
/// event reports them) moving LEFT go right, UP go down. Flip here.
pub fn focus_dir(horizontal: bool, delta: f64) -> FocusDir {
    match (horizontal, delta < 0.) {
        (true, true) => FocusDir::Right,
        (true, false) => FocusDir::Left,
        (false, true) => FocusDir::Down,
        (false, false) => FocusDir::Up,
    }
}

/// The direction of a workspace switch: fingers moving left go to the next
/// workspace. Flip here.
pub fn workspace_dir(dx: f64) -> WorkspaceDir {
    if dx < 0. {
        WorkspaceDir::Next
    } else {
        WorkspaceDir::Prev
    }
}

/// Whether a Cmd+Shift scroll event belongs to the workspace swipe: a clearly
/// horizontal one, or one with no movement at all (a gesture's first event,
/// which only does the bookkeeping). Anything else, a vertical Cmd+Shift
/// scroll, is NOT claimed: it keeps its old meaning, the zoom of Cmd+scroll
/// (Ekin, 2026-10-20).
pub fn claims_workspace(dx: f64, dy: f64) -> bool {
    (dx == 0. && dy == 0.) || dx.abs() >= dy.abs() * HORIZONTAL_RATIO
}

/// One scroll event, as `AppView::wheel` gets it.
#[derive(Clone, Copy, Debug)]
pub struct ScrollEv {
    /// The delta: pixels, or lines when `lines` (a notched wheel).
    pub dx: f64,
    pub dy: f64,
    pub lines: bool,
    pub phase: Phase,
    /// Shift is held with Cmd: a workspace swipe instead of a focus one.
    pub shift: bool,
    pub now_ms: f64,
}

/// What to run, and whether it is the first thing a gesture did (the usage
/// log counts a gesture once, not its steps).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavOut {
    pub action: NavAction,
    pub first_of_gesture: bool,
}

#[derive(Clone, Debug, Default)]
pub struct NavState {
    ax: f64,
    ay: f64,
    last_ms: f64,
    shift: bool,
    /// The fingers lifted: the events that follow are the OS's momentum.
    lifted: bool,
    /// A workspace switch happened in this gesture.
    switched: bool,
    /// The focus already moved in this gesture: one card per swipe.
    stepped: bool,
    /// When the last workspace switch happened (none yet: a first swipe is
    /// never "too soon").
    last_switch_ms: Option<f64>,
    /// The gesture already did something (and was counted).
    noted: bool,
}

impl NavState {
    /// A gesture begins: nothing accumulated, nothing switched, not lifted.
    fn begin(&mut self) {
        self.ax = 0.;
        self.ay = 0.;
        self.lifted = false;
        self.switched = false;
        self.stepped = false;
        self.noted = false;
    }

    fn first(&mut self) -> bool {
        let first = !self.noted;
        self.noted = true;
        first
    }

    /// Takes one scroll event and answers with what to run (nothing for most
    /// events: a step needs `STEP_PX` of swipe).
    pub fn step(&mut self, ev: ScrollEv) -> Vec<NavOut> {
        let gap = ev.now_ms - self.last_ms;
        self.last_ms = ev.now_ms;
        if gap > GESTURE_GAP_MS || ev.phase == Phase::Started || ev.shift != self.shift {
            self.begin();
        }
        self.shift = ev.shift;
        if self.lifted {
            // Momentum after the fingers lifted.
            return Vec::new();
        }
        let out = if ev.shift {
            self.workspace(ev)
        } else {
            self.focus(ev)
        };
        if ev.phase == Phase::Ended {
            self.lifted = true;
        }
        out
    }

    fn focus(&mut self, ev: ScrollEv) -> Vec<NavOut> {
        let (dx, dy) = if ev.lines {
            (ev.dx * STEP_PX, ev.dy * STEP_PX)
        } else {
            (ev.dx, ev.dy)
        };
        // One card per swipe: once the focus has moved, the rest of the
        // gesture (a long swipe, a flick) does nothing more.
        if self.stepped {
            return Vec::new();
        }
        self.ax += dx;
        self.ay += dy;
        let (x, y) = (self.ax.abs(), self.ay.abs());
        if x.max(y) < STEP_PX {
            return Vec::new();
        }
        // The axis with more movement decides the direction.
        let dir = if x >= y {
            focus_dir(true, self.ax.signum())
        } else {
            focus_dir(false, self.ay.signum())
        };
        self.stepped = true;
        let first = self.first();
        vec![NavOut {
            action: NavAction::Focus(dir),
            first_of_gesture: first,
        }]
    }

    fn workspace(&mut self, ev: ScrollEv) -> Vec<NavOut> {
        let (dx, dy) = if ev.lines {
            (ev.dx * WORKSPACE_PX, ev.dy * WORKSPACE_PX)
        } else {
            (ev.dx, ev.dy)
        };
        // Only a clearly horizontal swipe counts; anything else is ignored
        // (and does not undo what a horizontal one gathered).
        if dx.abs() < dy.abs() * HORIZONTAL_RATIO {
            return Vec::new();
        }
        self.ax += dx;
        if !self.switched
            && self.ax.abs() >= WORKSPACE_PX
            && self
                .last_switch_ms
                .is_none_or(|t| ev.now_ms - t >= WORKSPACE_GAP_MS)
        {
            self.switched = true;
            self.last_switch_ms = Some(ev.now_ms);
            let first = self.first();
            return vec![NavOut {
                action: NavAction::Workspace(workspace_dir(self.ax)),
                first_of_gesture: first,
            }];
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(dx: f64, dy: f64, phase: Phase, now: f64) -> ScrollEv {
        ScrollEv {
            dx,
            dy,
            lines: false,
            phase,
            shift: false,
            now_ms: now,
        }
    }
    fn shift_ev(dx: f64, dy: f64, phase: Phase, now: f64) -> ScrollEv {
        ScrollEv {
            shift: true,
            ..ev(dx, dy, phase, now)
        }
    }
    fn actions(out: &[NavOut]) -> Vec<NavAction> {
        out.iter().map(|o| o.action).collect()
    }
    use FocusDir::*;
    fn f(d: FocusDir) -> NavAction {
        NavAction::Focus(d)
    }

    #[test]
    fn direction_follows_the_fingers() {
        // Fingers moving left go right, up go down; the reverse the other way.
        assert_eq!(focus_dir(true, -10.), Right);
        assert_eq!(focus_dir(true, 10.), Left);
        assert_eq!(focus_dir(false, -10.), Down);
        assert_eq!(focus_dir(false, 10.), Up);
        assert_eq!(workspace_dir(-10.), WorkspaceDir::Next);
        assert_eq!(workspace_dir(10.), WorkspaceDir::Prev);
    }

    #[test]
    fn command_and_gesture_ids() {
        assert_eq!(f(Left).command_id(), "focus.move.left");
        assert_eq!(f(Down).command_id(), "focus.move.down");
        assert_eq!(
            NavAction::Workspace(WorkspaceDir::Next).command_id(),
            "workspace.next"
        );
        assert_eq!(
            NavAction::Workspace(WorkspaceDir::Prev).command_id(),
            "workspace.prev"
        );
        assert_eq!(f(Up).gesture_id(), "mouse.focus.scroll");
        assert_eq!(
            NavAction::Workspace(WorkspaceDir::Prev).gesture_id(),
            "mouse.workspace.scroll"
        );
    }

    #[test]
    fn a_swipe_moves_one_card_however_long_it_is() {
        let mut s = NavState::default();
        assert!(s.step(ev(0., 0., Phase::Started, 1000.)).is_empty());
        let mut steps = Vec::new();
        for i in 0..40 {
            steps.extend(s.step(ev(-10., 0., Phase::Moved, 1010. + i as f64 * 8.)));
        }
        // 400 px to the left: one card to the right, not six.
        assert_eq!(actions(&steps), vec![f(Right)]);
    }

    #[test]
    fn a_flick_and_a_huge_delta_still_move_one_card() {
        let mut s = NavState::default();
        s.step(ev(0., 0., Phase::Started, 0.));
        let out = s.step(ev(0., 200., Phase::Moved, 8.));
        assert_eq!(actions(&out), vec![f(Up)]);
        let mut s = NavState::default();
        s.step(ev(0., 0., Phase::Started, 0.));
        let out = s.step(ev(0., -5000., Phase::Moved, 8.));
        assert_eq!(actions(&out), vec![f(Down)]);
        // The rest of the gesture does nothing, whichever way it goes.
        assert!(s.step(ev(0., -500., Phase::Moved, 16.)).is_empty());
        assert!(s.step(ev(0., 500., Phase::Moved, 24.)).is_empty());
    }

    #[test]
    fn less_than_a_step_moves_nothing_and_the_next_swipe_moves_again() {
        let mut s = NavState::default();
        s.step(ev(0., 0., Phase::Started, 0.));
        assert!(s.step(ev(0., 40., Phase::Moved, 8.)).is_empty());
        s.step(ev(0., 0., Phase::Ended, 16.));
        // A second swipe, a new touch: one more card.
        s.step(ev(0., 0., Phase::Started, 600.));
        assert_eq!(
            actions(&s.step(ev(0., 80., Phase::Moved, 608.))),
            vec![f(Up)]
        );
        s.step(ev(0., 0., Phase::Ended, 700.));
        s.step(ev(0., 0., Phase::Started, 1300.));
        assert_eq!(
            actions(&s.step(ev(0., 80., Phase::Moved, 1308.))),
            vec![f(Up)]
        );
    }

    #[test]
    fn the_dominant_axis_wins() {
        let mut s = NavState::default();
        s.step(ev(0., 0., Phase::Started, 0.));
        let out = s.step(ev(-80., 20., Phase::Moved, 8.));
        assert_eq!(actions(&out), vec![f(Right)]);
    }

    #[test]
    fn momentum_after_the_fingers_lift_is_ignored() {
        let mut s = NavState::default();
        s.step(ev(0., 0., Phase::Started, 0.));
        let out = s.step(ev(0., 70., Phase::Moved, 8.));
        assert_eq!(actions(&out), vec![f(Up)]);
        assert!(s.step(ev(0., 0., Phase::Ended, 16.)).is_empty());
        // The OS's momentum: Moved events, no new Started, every few ms.
        for i in 0..40 {
            let out = s.step(ev(0., 90., Phase::Moved, 24. + i as f64 * 10.));
            assert!(out.is_empty(), "momentum event {i} moved the focus");
        }
    }

    #[test]
    fn a_new_touch_after_momentum_works_and_so_does_a_pause() {
        let mut s = NavState::default();
        s.step(ev(0., 0., Phase::Started, 0.));
        s.step(ev(0., 0., Phase::Ended, 8.));
        assert!(s.step(ev(0., 90., Phase::Moved, 16.)).is_empty());
        // The fingers touch again.
        s.step(ev(0., 0., Phase::Started, 500.));
        assert_eq!(
            actions(&s.step(ev(0., 70., Phase::Moved, 508.))),
            vec![f(Up)]
        );
        // Or, with no touch event at all (a mouse after a trackpad), a pause
        // longer than a gesture ends the "lifted" state.
        s.step(ev(0., 0., Phase::Ended, 520.));
        assert!(s.step(ev(0., 90., Phase::Moved, 530.)).is_empty());
        assert_eq!(
            actions(&s.step(ev(0., 90., Phase::Moved, 900.))),
            vec![f(Up)]
        );
    }

    #[test]
    fn a_pause_forgets_what_was_gathered() {
        let mut s = NavState::default();
        s.step(ev(0., 0., Phase::Started, 0.));
        assert!(s.step(ev(40., 0., Phase::Moved, 8.)).is_empty());
        // 200 ms later the 40 px are gone: 40 more is still not a step.
        assert!(s.step(ev(40., 0., Phase::Moved, 208.)).is_empty());
    }

    #[test]
    fn a_burst_of_wheel_notches_is_one_swipe_and_a_later_notch_another() {
        let mut s = NavState::default();
        let notch = |dy: f64, now: f64| ScrollEv {
            lines: true,
            ..ev(0., dy, Phase::Moved, now)
        };
        // Three notches down, 40 ms apart, no phases: one swipe, one card.
        let mut steps = Vec::new();
        for i in 0..3 {
            steps.extend(s.step(notch(-1., 1000. + i as f64 * 40.)));
        }
        assert_eq!(actions(&steps), vec![f(Down)]);
        // A notch after a long pause is a new swipe and moves one card.
        assert_eq!(actions(&s.step(notch(1., 3000.))), vec![f(Up)]);
    }

    #[test]
    fn a_gesture_is_counted_once() {
        let mut s = NavState::default();
        s.step(ev(0., 0., Phase::Started, 0.));
        let a = s.step(ev(-200., 0., Phase::Moved, 8.));
        assert_eq!(a.len(), 1);
        assert!(a[0].first_of_gesture);
        assert!(s.step(ev(-70., 0., Phase::Moved, 16.)).is_empty());
        // The next gesture is counted again.
        s.step(ev(0., 0., Phase::Started, 600.));
        assert!(s.step(ev(-70., 0., Phase::Moved, 608.))[0].first_of_gesture);
    }

    #[test]
    fn a_sideways_swipe_switches_one_workspace() {
        let mut s = NavState::default();
        s.step(shift_ev(0., 0., Phase::Started, 0.));
        let mut seen = Vec::new();
        for i in 0..30 {
            seen.extend(s.step(shift_ev(-20., 0., Phase::Moved, 8. + i as f64 * 8.)));
        }
        // 600 px of swipe, one switch (to the next workspace) and no more.
        assert_eq!(
            actions(&seen),
            vec![NavAction::Workspace(WorkspaceDir::Next)]
        );
        assert!(seen[0].first_of_gesture);
    }

    #[test]
    fn the_other_way_is_the_previous_workspace() {
        let mut s = NavState::default();
        s.step(shift_ev(0., 0., Phase::Started, 0.));
        let out = s.step(shift_ev(130., 0., Phase::Moved, 8.));
        assert_eq!(
            actions(&out),
            vec![NavAction::Workspace(WorkspaceDir::Prev)]
        );
    }

    #[test]
    fn only_a_horizontal_cmd_shift_swipe_is_claimed() {
        // The first event of a gesture has no movement: claimed (bookkeeping).
        assert!(claims_workspace(0., 0.));
        // Clearly sideways: claimed.
        assert!(claims_workspace(-30., 5.));
        assert!(claims_workspace(8., -4.));
        // Vertical and diagonal: not claimed, they zoom as before.
        assert!(!claims_workspace(0., 10.));
        assert!(!claims_workspace(3., -20.));
        assert!(!claims_workspace(10., 8.));
    }

    #[test]
    fn a_vertical_or_diagonal_swipe_with_shift_does_nothing() {
        let mut s = NavState::default();
        s.step(shift_ev(0., 0., Phase::Started, 0.));
        for i in 0..20 {
            assert!(s
                .step(shift_ev(0., -60., Phase::Moved, 8. + i as f64 * 8.))
                .is_empty());
        }
        let mut s = NavState::default();
        s.step(shift_ev(0., 0., Phase::Started, 0.));
        // 100 sideways against 80 vertical is not clearly horizontal.
        assert!(s.step(shift_ev(-100., 80., Phase::Moved, 8.)).is_empty());
        assert!(s.step(shift_ev(-100., 80., Phase::Moved, 16.)).is_empty());
    }

    #[test]
    fn two_switches_need_350_ms_between_them() {
        let mut s = NavState::default();
        s.step(shift_ev(0., 0., Phase::Started, 1000.));
        assert_eq!(s.step(shift_ev(-130., 0., Phase::Moved, 1008.)).len(), 1);
        s.step(shift_ev(0., 0., Phase::Ended, 1016.));
        // A new swipe 100 ms later (the fingers touched again): too soon.
        s.step(shift_ev(0., 0., Phase::Started, 1100.));
        assert!(s.step(shift_ev(-130., 0., Phase::Moved, 1108.)).is_empty());
        // After 350 ms it works.
        s.step(shift_ev(0., 0., Phase::Started, 1500.));
        assert_eq!(s.step(shift_ev(-130., 0., Phase::Moved, 1508.)).len(), 1);
    }

    #[test]
    fn a_wheel_notch_sideways_with_shift_switches_a_workspace() {
        let mut s = NavState::default();
        let notch = ScrollEv {
            lines: true,
            ..shift_ev(-1., 0., Phase::Moved, 2000.)
        };
        assert_eq!(
            actions(&s.step(notch)),
            vec![NavAction::Workspace(WorkspaceDir::Next)]
        );
    }

    #[test]
    fn changing_the_modifiers_starts_a_new_gesture() {
        let mut s = NavState::default();
        s.step(ev(0., 0., Phase::Started, 0.));
        assert!(s.step(ev(-50., 0., Phase::Moved, 8.)).is_empty());
        // Shift joins: what was gathered for focus does not count for a switch.
        assert!(s.step(shift_ev(-50., 0., Phase::Moved, 16.)).is_empty());
        assert_eq!(s.step(shift_ev(-80., 0., Phase::Moved, 24.)).len(), 1);
    }

    #[test]
    fn momentum_after_a_workspace_swipe_is_ignored_too() {
        let mut s = NavState::default();
        s.step(shift_ev(0., 0., Phase::Started, 0.));
        assert_eq!(s.step(shift_ev(-130., 0., Phase::Moved, 8.)).len(), 1);
        s.step(shift_ev(0., 0., Phase::Ended, 16.));
        for i in 0..40 {
            assert!(s
                .step(shift_ev(-200., 0., Phase::Moved, 24. + i as f64 * 10.))
                .is_empty());
        }
    }
}
