// Who gets an input event while the pointer is over a webview card: the page,
// or gpui (the canvas). Pure rules, with tests; mac.rs applies them from an
// NSEvent local monitor, which sees every event before AppKit hit-tests it.
//
// Why a monitor and not a WKWebView subclass: wry owns the WryWebView class,
// and one rule table here is easier to read and test than overrides spread
// over scrollWheel:, mouseDown:, magnifyWithEvent: and hitTest:.
// Related: mac.rs (installs the monitor, forwards to gpui's view),
// docs/agent/webview-spike.md ("Follow-up: gestures and input").
// Constraint: coordinates are window points, top-left origin, the same as the
// canvas; the monitor converts AppKit's bottom-left location first.

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
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub cmd: bool,
    pub ctrl: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Leave the event to AppKit: the page (or whatever is under the pointer).
    Page,
    /// Hand the event to gpui's own view.
    Gpui,
    /// A pinch: gpui has no magnify handler, so the app turns it into a zoom.
    Canvas,
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    /// The pages' rects.
    pub web: Vec<[f32; 4]>,
    /// Rects drawn by gpui over the pages (cards in front, the ghost, a panel).
    pub cover: Vec<[f32; 4]>,
}

pub fn inside(r: &[f32; 4], p: (f32, f32)) -> bool {
    p.0 >= r[0] && p.0 < r[0] + r[2] && p.1 >= r[1] && p.1 < r[1] + r[3]
}

/// The holes of a mask as rects that do not overlap. An even-odd path with
/// two overlapping holes fills their overlap again, so the page showed
/// through where two covers met (found in the follow-up run). Coordinate
/// compression: cut the plane along every edge, keep the cells inside any
/// hole. Cells are a few; merging neighbours is not worth it here.
pub fn disjoint(holes: &[[f32; 4]]) -> Vec<[f32; 4]> {
    let mut xs: Vec<f32> = holes.iter().flat_map(|h| [h[0], h[0] + h[2]]).collect();
    let mut ys: Vec<f32> = holes.iter().flat_map(|h| [h[1], h[1] + h[3]]).collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    xs.dedup();
    ys.dedup();
    let mut out = Vec::new();
    for xw in xs.windows(2) {
        for yw in ys.windows(2) {
            let c = (xw[0] + 0.01, yw[0] + 0.01);
            if holes.iter().any(|h| inside(h, c)) {
                out.push([xw[0], yw[0], xw[1] - xw[0], yw[1] - yw[0]]);
            }
        }
    }
    out
}

/// `gpui_owns_drag`: a drag that began on gpui stays gpui's until the button
/// goes up, because AppKit sends drag events to the view that took the
/// mouse-down, and we took that one away from AppKit.
pub fn route(kind: Kind, mods: Mods, p: (f32, f32), layout: &Layout, gpui_owns_drag: bool) -> Target {
    match kind {
        Kind::LeftDragged | Kind::LeftUp | Kind::OtherDragged | Kind::OtherUp if gpui_owns_drag => {
            return Target::Gpui
        }
        _ => {}
    }
    if !layout.web.iter().any(|r| inside(r, p)) {
        return Target::Page;
    }
    let covered = layout.cover.iter().any(|r| inside(r, p));
    match kind {
        Kind::Magnify => Target::Canvas,
        // A Cmd or Ctrl wheel is the canvas zoom, and a pinch arrives as Ctrl
        // (docs/agent/ui-chrome.md); a plain wheel scrolls the page unless
        // another card is drawn over that spot.
        Kind::Scroll if mods.cmd || mods.ctrl || covered => Target::Gpui,
        Kind::OtherDown => Target::Gpui,
        Kind::LeftDown if mods.cmd || covered => Target::Gpui,
        _ => Target::Page,
    }
}

/// Whether a mouse-down that was routed to gpui starts a drag gpui owns.
pub fn starts_gpui_drag(kind: Kind, target: Target) -> bool {
    target == Target::Gpui && matches!(kind, Kind::LeftDown | Kind::OtherDown)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// Not ours to decide: normal AppKit dispatch.
    Pass,
    /// A page editing chord: send this action to the page (`copy:`).
    PageAction(&'static str),
    /// Offer the chord to gpui first; if it does not take it, the page does.
    TryGpui,
}

/// `page_focused`: the first responder is the webview.
pub fn key_route(cmd: bool, ch: &str, page_focused: bool) -> KeyAction {
    if !cmd || !page_focused {
        return KeyAction::Pass;
    }
    match ch {
        "c" => KeyAction::PageAction("copy:"),
        "v" => KeyAction::PageAction("paste:"),
        "x" => KeyAction::PageAction("cut:"),
        "a" => KeyAction::PageAction("selectAll:"),
        "z" => KeyAction::PageAction("undo:"),
        _ => KeyAction::TryGpui,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout { web: vec![[0., 0., 100., 100.]], cover: vec![[50., 50., 100., 100.]] }
    }
    const NONE: Mods = Mods { cmd: false, ctrl: false };
    const CMD: Mods = Mods { cmd: true, ctrl: false };

    #[test]
    fn overlapping_holes_become_disjoint_cells_with_the_same_area() {
        let cells = disjoint(&[[0., 0., 10., 10.], [5., 5., 10., 10.]]);
        let area: f32 = cells.iter().map(|c| c[2] * c[3]).sum();
        assert_eq!(area, 100. + 100. - 25.);
        for (i, a) in cells.iter().enumerate() {
            for b in &cells[i + 1..] {
                let x = a[0].max(b[0]) < (a[0] + a[2]).min(b[0] + b[2]);
                let y = a[1].max(b[1]) < (a[1] + a[3]).min(b[1] + b[3]);
                assert!(!(x && y), "{a:?} overlaps {b:?}");
            }
        }
        assert!(disjoint(&[]).is_empty());
    }

    #[test]
    fn plain_wheel_over_the_page_scrolls_the_page() {
        assert_eq!(route(Kind::Scroll, NONE, (10., 10.), &layout(), false), Target::Page);
    }

    #[test]
    fn cmd_or_ctrl_wheel_is_the_canvas() {
        assert_eq!(route(Kind::Scroll, CMD, (10., 10.), &layout(), false), Target::Gpui);
        let ctrl = Mods { cmd: false, ctrl: true };
        assert_eq!(route(Kind::Scroll, ctrl, (10., 10.), &layout(), false), Target::Gpui);
    }

    #[test]
    fn a_covered_spot_belongs_to_the_card_in_front() {
        assert_eq!(route(Kind::LeftDown, NONE, (60., 60.), &layout(), false), Target::Gpui);
        assert_eq!(route(Kind::Scroll, NONE, (60., 60.), &layout(), false), Target::Gpui);
        assert_eq!(route(Kind::LeftDown, NONE, (10., 10.), &layout(), false), Target::Page);
    }

    #[test]
    fn middle_button_and_cmd_click_pan() {
        assert_eq!(route(Kind::OtherDown, NONE, (10., 10.), &layout(), false), Target::Gpui);
        assert_eq!(route(Kind::LeftDown, CMD, (10., 10.), &layout(), false), Target::Gpui);
    }

    #[test]
    fn a_gpui_drag_stays_gpui_even_off_the_page() {
        assert_eq!(route(Kind::LeftDragged, NONE, (500., 500.), &layout(), true), Target::Gpui);
        assert_eq!(route(Kind::LeftUp, NONE, (10., 10.), &layout(), true), Target::Gpui);
        assert_eq!(route(Kind::LeftDragged, NONE, (10., 10.), &layout(), false), Target::Page);
    }

    #[test]
    fn events_off_every_page_are_left_alone() {
        assert_eq!(route(Kind::Scroll, CMD, (500., 500.), &layout(), false), Target::Page);
        assert_eq!(route(Kind::Magnify, NONE, (500., 500.), &layout(), false), Target::Page);
    }

    #[test]
    fn a_pinch_over_a_page_is_the_canvas() {
        assert_eq!(route(Kind::Magnify, NONE, (10., 10.), &layout(), false), Target::Canvas);
    }

    #[test]
    fn editing_chords_stay_with_the_page_and_the_rest_try_gpui() {
        assert_eq!(key_route(true, "c", true), KeyAction::PageAction("copy:"));
        assert_eq!(key_route(true, "t", true), KeyAction::TryGpui);
        assert_eq!(key_route(true, "t", false), KeyAction::Pass);
        assert_eq!(key_route(false, "t", true), KeyAction::Pass);
    }
}
