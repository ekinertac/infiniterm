//! Who gets an input event while the pointer is over a webview browser card:
//! the page, or the canvas (gpui). Pure rules with tests; `infiniterm-ui/src/
//! webview_input.rs` applies them from an NSEvent local monitor, which sees
//! every event before AppKit hit-tests it (#761, `browser.engine` = `webview`).
//!
//! Why a monitor and not a subclass of the web view: wry owns the view's
//! class, and one rule table is easier to read and test than overrides
//! spread over scrollWheel:, mouseDown:, magnifyWithEvent: and hitTest:. The
//! spike (PR #812) proved the arrangement with real input.
//!
//! The product rules this table respects (CLAUDE.md): Cmd owns every app
//! binding, scrolling is for cards only (a plain wheel over a page scrolls
//! the page, over a card drawn in front it scrolls that card), the first
//! click on an unfocused card only focuses it, panning is Cmd+drag or
//! middle-drag, a pinch zooms the canvas.
//!
//! Where it fits: `webview_policy::holes_of` and this file take the same
//! cover list, so the mask holes (what the eye sees) and the click routing
//! (what the hand reaches) cannot disagree. Coordinates are window pixels,
//! top-left origin, the space gpui paints in.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Scroll,
    Magnify,
    LeftDown,
    LeftDragged,
    LeftUp,
    OtherDown,
    OtherDragged,
    OtherUp,
    RightDown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub cmd: bool,
    pub ctrl: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Leave the event to AppKit: the page.
    Page,
    /// Hand it to gpui's own view, which runs the app's own mouse path.
    Gpui,
    /// A pinch: gpui has no magnify handler, so the app turns it into the
    /// same zoom a Ctrl+wheel makes.
    Canvas,
    /// A right-click on a page: WebKit's own menu is not shown, the app's
    /// slim menu is (the page is asked what is under the point).
    Menu,
}

/// One page as the monitor sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct PageRoute {
    pub rect: [f64; 4],
    /// Rects gpui draws over the page: the cards in front, an open overlay,
    /// the drag ghost. The SAME list the mask holes are cut from.
    pub covers: Vec<[f64; 4]>,
    /// The card is the focused one. A click on an unfocused card only
    /// focuses it.
    pub focused: bool,
}

pub fn inside(r: &[f64; 4], p: (f64, f64)) -> bool {
    p.0 >= r[0] && p.0 < r[0] + r[2] && p.1 >= r[1] && p.1 < r[1] + r[3]
}

/// The page under a point that is not hidden by one of its own covers: the
/// page the pointer is really over. `None` over a cover, so a card drawn in
/// front of a page owns the pointer there. Pages are in paint order; the
/// last one that qualifies is the one on top.
pub fn page_at(pages: &[PageRoute], p: (f64, f64)) -> Option<&PageRoute> {
    pages
        .iter()
        .rev()
        .find(|pg| inside(&pg.rect, p) && !pg.covers.iter().any(|c| inside(c, p)))
}

/// Whether a point is over some page's rect at all, covered or not.
pub fn over_a_page(pages: &[PageRoute], p: (f64, f64)) -> bool {
    pages.iter().any(|pg| inside(&pg.rect, p))
}

/// Where an event goes. `gpui_owns_drag`: a press that began on gpui stays
/// gpui's until the button goes up, because AppKit sends drag events to the
/// view that took the press, and the monitor took that one away from AppKit.
pub fn route(
    kind: Kind,
    mods: Mods,
    p: (f64, f64),
    pages: &[PageRoute],
    gpui_owns_drag: bool,
) -> Target {
    if gpui_owns_drag
        && matches!(
            kind,
            Kind::LeftDragged | Kind::LeftUp | Kind::OtherDragged | Kind::OtherUp
        )
    {
        return Target::Gpui;
    }
    // Off every page, AppKit and gpui do what they always did.
    if !over_a_page(pages, p) {
        return Target::Page;
    }
    let page = page_at(pages, p);
    match kind {
        Kind::Magnify => Target::Canvas,
        // A Cmd or Ctrl wheel is the canvas zoom (a pinch arrives as Ctrl);
        // over a card in front a plain wheel is that card's.
        Kind::Scroll if mods.cmd || mods.ctrl || page.is_none() => Target::Gpui,
        Kind::OtherDown => Target::Gpui,
        // Over an uncovered page the app's menu; over a card in front, that
        // card's own right-click (gpui).
        Kind::RightDown if page.is_some() => Target::Menu,
        Kind::RightDown => Target::Gpui,
        // The first click on an unfocused card only focuses it (the scrim):
        // gpui's own mouse path does that, so the click goes there.
        Kind::LeftDown if mods.cmd || page.is_none_or(|pg| !pg.focused) => Target::Gpui,
        _ => Target::Page,
    }
}

/// Whether a press that was routed to gpui starts a drag gpui owns.
pub fn starts_gpui_drag(kind: Kind, target: Target) -> bool {
    target == Target::Gpui && matches!(kind, Kind::LeftDown | Kind::OtherDown)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// Normal AppKit dispatch: the page types.
    Pass,
    /// Offer the chord to gpui first (the app's keymap, the locked card's
    /// tab chords, the page editing chords through `edit_chord`); if gpui
    /// does not take it, the page gets it.
    TryGpui,
}

/// `page_focused`: the first responder is a web view. Only Cmd chords go to
/// gpui; plain keys, Shift, Option and Escape belong to the page (a CJK
/// input method and dead keys work inside it).
pub fn key_route(cmd: bool, page_focused: bool) -> KeyAction {
    if cmd && page_focused {
        KeyAction::TryGpui
    } else {
        KeyAction::Pass
    }
}

/// The ctrl+wheel delta that gives the zoom a pinch of `magnification`
/// should give: `input.rs::wheel` zooms by `exp(dy * 0.002 * sensitivity)`.
pub fn pinch_to_wheel_dy(magnification: f64, sensitivity: f64) -> f64 {
    let factor = (1. + magnification).max(0.01);
    factor.ln() / (0.002 * sensitivity.max(0.01))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(rect: [f64; 4], covers: Vec<[f64; 4]>, focused: bool) -> PageRoute {
        PageRoute {
            rect,
            covers,
            focused,
        }
    }
    const NONE: Mods = Mods {
        cmd: false,
        ctrl: false,
    };
    const CMD: Mods = Mods {
        cmd: true,
        ctrl: false,
    };
    const CTRL: Mods = Mods {
        cmd: false,
        ctrl: true,
    };

    fn one(focused: bool) -> Vec<PageRoute> {
        vec![page(
            [0., 0., 100., 100.],
            vec![[50., 50., 100., 100.]],
            focused,
        )]
    }

    #[test]
    fn a_plain_wheel_over_the_page_scrolls_the_page() {
        assert_eq!(
            route(Kind::Scroll, NONE, (10., 10.), &one(true), false),
            Target::Page
        );
        assert_eq!(
            route(Kind::Scroll, NONE, (10., 10.), &one(false), false),
            Target::Page
        );
    }

    #[test]
    fn a_cmd_or_ctrl_wheel_is_the_canvas() {
        assert_eq!(
            route(Kind::Scroll, CMD, (10., 10.), &one(true), false),
            Target::Gpui
        );
        assert_eq!(
            route(Kind::Scroll, CTRL, (10., 10.), &one(true), false),
            Target::Gpui
        );
    }

    #[test]
    fn a_covered_spot_belongs_to_the_card_in_front() {
        assert_eq!(
            route(Kind::LeftDown, NONE, (60., 60.), &one(true), false),
            Target::Gpui
        );
        assert_eq!(
            route(Kind::Scroll, NONE, (60., 60.), &one(true), false),
            Target::Gpui
        );
        assert_eq!(
            route(Kind::LeftDown, NONE, (10., 10.), &one(true), false),
            Target::Page
        );
    }

    #[test]
    fn the_first_click_on_an_unfocused_card_goes_to_gpui_to_focus_it() {
        assert_eq!(
            route(Kind::LeftDown, NONE, (10., 10.), &one(false), false),
            Target::Gpui
        );
        assert_eq!(
            route(Kind::LeftDown, NONE, (10., 10.), &one(true), false),
            Target::Page
        );
    }

    #[test]
    fn middle_button_and_cmd_click_belong_to_the_canvas() {
        assert_eq!(
            route(Kind::OtherDown, NONE, (10., 10.), &one(true), false),
            Target::Gpui
        );
        assert_eq!(
            route(Kind::LeftDown, CMD, (10., 10.), &one(true), false),
            Target::Gpui
        );
    }

    #[test]
    fn a_gpui_drag_stays_gpuis_even_off_every_page() {
        assert_eq!(
            route(Kind::LeftDragged, NONE, (500., 500.), &one(true), true),
            Target::Gpui
        );
        assert_eq!(
            route(Kind::LeftUp, NONE, (10., 10.), &one(true), true),
            Target::Gpui
        );
        assert_eq!(
            route(Kind::OtherUp, NONE, (10., 10.), &one(true), true),
            Target::Gpui
        );
        assert_eq!(
            route(Kind::LeftDragged, NONE, (10., 10.), &one(true), false),
            Target::Page
        );
    }

    #[test]
    fn events_off_every_page_are_left_alone() {
        assert_eq!(
            route(Kind::Scroll, CMD, (500., 500.), &one(true), false),
            Target::Page
        );
        assert_eq!(
            route(Kind::Magnify, NONE, (500., 500.), &one(true), false),
            Target::Page
        );
    }

    #[test]
    fn a_right_click_on_a_page_is_the_apps_menu_and_on_a_cover_the_covers() {
        assert_eq!(
            route(Kind::RightDown, NONE, (10., 10.), &one(true), false),
            Target::Menu
        );
        assert_eq!(
            route(Kind::RightDown, NONE, (10., 10.), &one(false), false),
            Target::Menu
        );
        assert_eq!(
            route(Kind::RightDown, NONE, (60., 60.), &one(true), false),
            Target::Gpui
        );
        assert_eq!(
            route(Kind::RightDown, NONE, (500., 500.), &one(true), false),
            Target::Page
        );
    }

    #[test]
    fn a_pinch_over_a_page_is_the_canvas() {
        assert_eq!(
            route(Kind::Magnify, NONE, (10., 10.), &one(true), false),
            Target::Canvas
        );
    }

    #[test]
    fn the_topmost_uncovered_page_wins_where_two_pages_overlap() {
        // B is later in paint order, so B is over A where they meet, and A's
        // cover list holds B's rect.
        let pages = vec![
            page([0., 0., 100., 100.], vec![[50., 0., 100., 100.]], true),
            page([50., 0., 100., 100.], vec![], false),
        ];
        let p = page_at(&pages, (60., 10.)).unwrap();
        assert_eq!(p.rect[0], 50.);
        assert_eq!(page_at(&pages, (10., 10.)).unwrap().rect[0], 0.);
        // The unfocused one on top takes the click as a focusing click.
        assert_eq!(
            route(Kind::LeftDown, NONE, (60., 10.), &pages, false),
            Target::Gpui
        );
    }

    #[test]
    fn only_cmd_chords_with_the_page_focused_are_offered_to_gpui() {
        assert_eq!(key_route(true, true), KeyAction::TryGpui);
        assert_eq!(key_route(true, false), KeyAction::Pass);
        assert_eq!(key_route(false, true), KeyAction::Pass);
    }

    #[test]
    fn a_pinch_gives_the_zoom_the_wheel_would() {
        // exp(dy * 0.002) must equal 1 + magnification.
        let dy = pinch_to_wheel_dy(0.25, 1.);
        assert!(((dy * 0.002).exp() - 1.25).abs() < 1e-9);
        assert!(pinch_to_wheel_dy(0., 1.).abs() < 1e-12);
        // A pinch in is positive, as scrolling up is.
        assert!(pinch_to_wheel_dy(0.1, 1.) > 0.);
        assert!(pinch_to_wheel_dy(-0.1, 1.) < 0.);
    }

    #[test]
    fn a_press_that_gpui_took_starts_a_gpui_drag() {
        assert!(starts_gpui_drag(Kind::LeftDown, Target::Gpui));
        assert!(starts_gpui_drag(Kind::OtherDown, Target::Gpui));
        assert!(!starts_gpui_drag(Kind::LeftDown, Target::Page));
        assert!(!starts_gpui_drag(Kind::Scroll, Target::Gpui));
    }
}
