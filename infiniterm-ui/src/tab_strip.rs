//! The tab strip an editor card wears: the band at the top listing its
//! tabs, the `×` on each, the `+` past the last, the card's number at the
//! right, and the hit test that says which of those a click landed on.
//!
//! A second copy of what `browser_body.rs` draws for a browser card, on
//! purpose: that file is the browser session's, and the two strips must
//! look and click the same, so the constants are the same numbers. When a
//! third card kind wants tabs the browser's strip should come here too.
//! Sized in screen pixels, then `ui_scale`, then the zoom, like every
//! other piece of chrome.
//!
//! A tab dragged along the strip (#682) is `TabDrag`: the body keeps one
//! from the press to the release, `paint_strip` draws the others making
//! room, and the drop is handed to the model as `Model::move_tab`.
//!
//! Called by `editor_tabs.rs`. `body::TabClick` is the shared answer.
use crate::body::TabClick;
use gpui::{fill, point, px, size, App, Bounds, Font, Hsla, Pixels, Window};
use infiniterm_core::grid::Point;
use infiniterm_core::model::tabs_cmd::index_after_move;

/// The strip is sized from `ui.fontSize`, in screen pixels times `ui_scale`.
/// Everything else is a ratio of that, so the strip keeps its proportions
/// at any size.
pub const TAB_STRIP_HEIGHT_RATIO: f64 = 1.9;
/// A tab's width in font sizes. Fixed rather than proportional to the
/// card: a card with many tabs would otherwise shrink every tab to a
/// sliver.
pub const TAB_STRIP_TAB_WIDTH_RATIO: f64 = 9.;
pub const TAB_STRIP_LABEL_PAD_RATIO: f64 = 0.6;
/// The close button's band at a tab's right edge.
pub const TAB_STRIP_CLOSE_WIDTH_RATIO: f64 = 1.5;
pub const TAB_STRIP_BORDER_PX: f64 = 1.;
/// The "+" button's slot after the last tab, in font sizes: wide enough to
/// click, narrow enough that the glyph sits beside the tabs. It took a whole
/// tab's width (9) and floated about four sizes away from them (#375).
pub const TAB_STRIP_PLUS_WIDTH_RATIO: f64 = 2.5;

/// How far a press on a tab travels, in font sizes, before it is a drag:
/// under that it is the click that switched to the tab, and a hand that
/// shakes a little does not reorder anything.
pub const TAB_DRAG_SLOP_RATIO: f64 = 0.5;

/// A tab held by the pointer (#682), in world units like `strip_hit`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabDrag {
    pub from: usize,
    start_x: f64,
    /// How far the pointer has gone since the press.
    pub dx: f64,
    /// Past the slop: the strip draws the drag and the release drops it.
    pub moving: bool,
}

impl TabDrag {
    pub fn new(from: usize, x: f64) -> TabDrag {
        TabDrag {
            from,
            start_x: x,
            dx: 0.,
            moving: false,
        }
    }

    /// The pointer at `x`; true when the strip has to be painted again.
    pub fn moved(&mut self, x: f64, font_px: f64, ui_scale: f32) -> bool {
        self.dx = x - self.start_x;
        let slop = TAB_DRAG_SLOP_RATIO * font_px * ui_scale as f64;
        if !self.moving && self.dx.abs() >= slop {
            self.moving = true;
        }
        self.moving
    }

    /// Where the tab would land if dropped now.
    pub fn to(&self, font_px: f64, ui_scale: f32, tab_count: usize) -> usize {
        let tab_w = TAB_STRIP_TAB_WIDTH_RATIO * font_px * ui_scale as f64;
        drop_index(self.from, self.dx, tab_w, tab_count)
    }

    /// The release: the move to make, if the tab went anywhere.
    pub fn drop(&self, font_px: f64, ui_scale: f32, tab_count: usize) -> Option<(usize, usize)> {
        let to = self.to(font_px, ui_scale, tab_count);
        (self.moving && to != self.from).then_some((self.from, to))
    }
}

/// `dx` held to the strip: the dragged tab stops at the first and last
/// places rather than sliding off over the "+" or the folder header.
fn clamped_dx(from: usize, dx: f64, tab_w: f64, tab_count: usize) -> f64 {
    let last = tab_count.saturating_sub(1) as f64;
    dx.clamp(-(from as f64) * tab_w, (last - from as f64) * tab_w)
}

/// The place whose slot holds the dragged tab's middle: it takes that
/// place, the way a dragged workspace tab takes the one it is over.
pub fn drop_index(from: usize, dx: f64, tab_w: f64, tab_count: usize) -> usize {
    if tab_count == 0 || tab_w <= 0. {
        return from;
    }
    let mid = (from as f64 + 0.5) * tab_w + clamped_dx(from, dx, tab_w, tab_count);
    ((mid / tab_w).floor().max(0.) as usize).min(tab_count - 1)
}

/// The strip's colours, mirrored from the chrome each frame.
#[derive(Clone, Debug, PartialEq)]
pub struct StripStyle {
    pub bg: Hsla,
    pub border: Hsla,
    pub active_bg: Hsla,
    pub text_bright: Hsla,
    pub text_muted: Hsla,
    pub regular: Font,
    pub bold: Font,
    /// `ui.fontSize`: the strip's font, in screen pixels.
    pub font_px: f64,
}

/// The strip's height in WORLD units: `local`'s space in every mouse
/// handler, the zoom not yet applied.
pub fn strip_world_h(font_px: f64, ui_scale: f32) -> f64 {
    font_px * TAB_STRIP_HEIGHT_RATIO * ui_scale as f64
}

/// Where a tab's close band starts, from the tab's left edge, in font sizes.
fn close_band_left_ratio() -> f64 {
    TAB_STRIP_TAB_WIDTH_RATIO - TAB_STRIP_CLOSE_WIDTH_RATIO
}

/// The strip's own "#N" badge, with `card.protect`'s own lock ahead of it
/// when set — the same glyph and wording as the corner label's
/// (`Model::numbered_label`). A different lock from this strip's `locked`
/// (the keyboard's), so it is spelled out rather than reusing that word.
fn number_label(card_number: u32, protected: bool) -> String {
    if protected {
        format!("\u{1f512} #{card_number}")
    } else {
        format!("#{card_number}")
    }
}

/// Which of the strip's affordances a click at `local` (world units) lands
/// on, or `None` below the strip.
///
/// `left` is the width of the folder header ahead of the tabs (world units,
/// 0 without one): a click on the header is no tab's.
pub fn strip_hit(
    local: Point,
    font_px: f64,
    ui_scale: f32,
    tab_count: usize,
    left: f64,
) -> Option<TabClick> {
    let strip_h = strip_world_h(font_px, ui_scale);
    if local.y < 0. || local.y >= strip_h || local.x < left {
        return None;
    }
    let local = Point {
        x: local.x - left,
        y: local.y,
    };
    let unit = font_px * ui_scale as f64;
    let tab_w = TAB_STRIP_TAB_WIDTH_RATIO * unit;
    let index = (local.x / tab_w).floor().max(0.) as usize;
    if index >= tab_count {
        let from_plus = local.x - tab_count as f64 * tab_w;
        return (from_plus >= 0. && from_plus < TAB_STRIP_PLUS_WIDTH_RATIO * unit)
            .then_some(TabClick::New);
    }
    let x_in_tab = local.x - (index as f64) * tab_w;
    if x_in_tab >= close_band_left_ratio() * unit {
        Some(TabClick::Close(index))
    } else {
        Some(TabClick::Switch(index))
    }
}

/// Draws the strip across the top of `bounds` (screen pixels, at `scale`)
/// and returns its height, so the caller paints its content below.
/// `header` is a folder editor's name and the width of its tree (world
/// units): a box over the tree's column, with the tabs starting after it.
/// `drag` is a tab being dragged along the strip, if one is.
#[allow(clippy::too_many_arguments)]
pub fn paint_strip(
    bounds: Bounds<Pixels>,
    scale: f64,
    ui_scale: f32,
    labels: &[String],
    active: usize,
    drag: Option<&TabDrag>,
    card_number: u32,
    protected: bool,
    header: Option<(&str, f64)>,
    style: &StripStyle,
    window: &mut Window,
    cx: &mut App,
) -> Pixels {
    // One unit is the font size on screen; every measure is a ratio of it.
    let unit = style.font_px * ui_scale as f64 * scale;
    let strip_h = px((TAB_STRIP_HEIGHT_RATIO * unit) as f32);
    let strip = Bounds::new(bounds.origin, size(bounds.size.width, strip_h));
    // The strip is the card's top edge: with rounded cards its top corners follow.
    let r = crate::chrome::card_radius(cx);
    window.paint_quad(fill(strip, style.bg).corner_radii(gpui::Corners {
        top_left: r,
        top_right: r,
        bottom_left: px(0.),
        bottom_right: px(0.),
    }));
    let border = px((TAB_STRIP_BORDER_PX * ui_scale as f64 * scale) as f32);
    let tab_w = px((TAB_STRIP_TAB_WIDTH_RATIO * unit) as f32);
    let strip_font = px(unit as f32);
    let close_w = px((TAB_STRIP_CLOSE_WIDTH_RATIO * unit) as f32);
    let close_left = px((close_band_left_ratio() * unit) as f32);
    if strip_font >= crate::chrome::legible_font_px(window.scale_factor()) {
        let pad = px((TAB_STRIP_LABEL_PAD_RATIO * unit) as f32);
        let left = px(header.map_or(0., |(_, w)| w * scale) as f32);
        if let Some((name, _)) = header {
            let head = Bounds::new(strip.origin, size(left, strip_h));
            let room = f32::from(left) - f32::from(pad) * 2.;
            let shown = crate::text::elide(name, room, |t| {
                f32::from(
                    crate::text::shape(window, t, strip_font, &style.bold, style.text_bright).width,
                )
            });
            let line =
                crate::text::shape(window, &shown, strip_font, &style.bold, style.text_bright);
            crate::text::paint_in(window, cx, &line, head, pad);
            window.paint_quad(fill(
                Bounds::new(
                    point(head.origin.x + left - border, head.origin.y),
                    size(border, strip_h),
                ),
                style.border,
            ));
        }
        // A drag (#682): the others sit where the drop would leave them,
        // the held tab follows the pointer and is painted last, over them.
        let world_tab_w = TAB_STRIP_TAB_WIDTH_RATIO * style.font_px * ui_scale as f64;
        let drag = drag.filter(|d| d.moving && d.from < labels.len());
        let to = drag.map(|d| drop_index(d.from, d.dx, world_tab_w, labels.len()));
        let mut order: Vec<usize> = (0..labels.len())
            .filter(|&i| Some(i) != drag.map(|d| d.from))
            .collect();
        order.extend(drag.map(|d| d.from));
        for i in order {
            let label = &labels[i];
            let x = match (drag, to) {
                (Some(d), _) if d.from == i => {
                    let dx = clamped_dx(d.from, d.dx, world_tab_w, labels.len());
                    tab_w * (i as f32) + px((dx * scale) as f32)
                }
                (Some(d), Some(to)) => tab_w * (index_after_move(i, d.from, to) as f32),
                _ => tab_w * (i as f32),
            };
            let tab_bounds = Bounds::new(
                point(strip.origin.x + left + x, strip.origin.y),
                size(tab_w, strip_h),
            );
            let held = drag.is_some_and(|d| d.from == i);
            let (color, font) = if i == active {
                window.paint_quad(fill(tab_bounds, style.active_bg));
                (style.text_bright, &style.bold)
            } else {
                if held {
                    window.paint_quad(fill(tab_bounds, style.bg));
                }
                (style.text_muted, &style.regular)
            };
            if held {
                window.paint_quad(fill(
                    Bounds::new(tab_bounds.origin, size(border, strip_h)),
                    style.border,
                ));
            }
            let sep = Bounds::new(
                point(tab_bounds.origin.x + tab_w - border, tab_bounds.origin.y),
                size(border, strip_h),
            );
            window.paint_quad(fill(sep, style.border));
            let room = f32::from(close_left) - f32::from(pad) * 2.;
            let shown = crate::text::elide(label, room, |t| {
                f32::from(crate::text::shape(window, t, strip_font, font, color).width)
            });
            let line = crate::text::shape(window, &shown, strip_font, font, color);
            crate::text::paint_in(window, cx, &line, tab_bounds, pad);
            let close_line = crate::text::shape(window, "×", strip_font, font, color);
            let close_bounds = Bounds::new(
                point(
                    tab_bounds.origin.x + close_left + (close_w - close_line.width) / 2.,
                    tab_bounds.origin.y,
                ),
                size(close_line.width, strip_h),
            );
            crate::text::paint_in(window, cx, &close_line, close_bounds, px(0.));
        }
        let plus = crate::text::shape(window, "+", strip_font, &style.regular, style.text_muted);
        let plus_w = px((TAB_STRIP_PLUS_WIDTH_RATIO * unit) as f32);
        let plus_bounds = Bounds::new(
            point(
                strip.origin.x + left + tab_w * (labels.len() as f32) + plus_w / 2.
                    - plus.width / 2.,
                strip.origin.y,
            ),
            size(plus.width, strip_h),
        );
        crate::text::paint_in(window, cx, &plus, plus_bounds, px(0.));
        if card_number > 0 {
            let number = number_label(card_number, protected);
            let line = crate::text::shape(
                window,
                &number,
                strip_font,
                &style.regular,
                style.text_muted,
            );
            let number_bounds = Bounds::new(
                point(
                    strip.origin.x + strip.size.width - line.width - pad,
                    strip.origin.y,
                ),
                size(line.width + pad, strip_h),
            );
            crate::text::paint_in(window, cx, &line, number_bounds, px(0.));
        }
    }
    // The hairline under the strip.
    window.paint_quad(fill(
        Bounds::new(
            point(strip.origin.x, strip.origin.y + strip_h - border),
            size(strip.size.width, border),
        ),
        style.border,
    ));
    strip_h
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hit-test geometry in world units at a 10 px font and ui_scale 1: a
    // tab is 90 wide, its last 15 the close band, the strip 19 tall, the
    // 25 after the last tab is "+".
    #[test]
    fn a_click_lands_on_a_tab_its_close_or_the_plus() {
        let at = |x: f64, y: f64| Point { x, y };
        assert_eq!(
            strip_hit(at(10., 10.), 10., 1., 2, 0.),
            Some(TabClick::Switch(0))
        );
        assert_eq!(
            strip_hit(at(80., 10.), 10., 1., 2, 0.),
            Some(TabClick::Close(0))
        );
        assert_eq!(
            strip_hit(at(100., 10.), 10., 1., 2, 0.),
            Some(TabClick::Switch(1))
        );
        assert_eq!(
            strip_hit(at(190., 10.), 10., 1., 2, 0.),
            Some(TabClick::New)
        );
        assert_eq!(
            strip_hit(at(210., 10.), 10., 1., 2, 0.),
            None,
            "past the plus's own slot, 25 wide after the last tab (#375)"
        );
        assert_eq!(
            strip_hit(at(400., 10.), 10., 1., 2, 0.),
            None,
            "past the plus"
        );
        assert_eq!(
            strip_hit(at(10., 25.), 10., 1., 2, 0.),
            None,
            "below the strip"
        );
        // Twice the interface scale: everything twice as big.
        assert_eq!(
            strip_hit(at(100., 25.), 10., 2., 2, 0.),
            Some(TabClick::Switch(0))
        );
    }

    // A folder editor's header takes the tree's width ahead of the tabs:
    // a click on it is nobody's, and the tabs count from its right edge.
    #[test]
    fn the_folder_header_shifts_the_tabs_and_is_no_tab() {
        let at = |x: f64, y: f64| Point { x, y };
        assert_eq!(strip_hit(at(100., 10.), 10., 1., 2, 200.), None);
        assert_eq!(
            strip_hit(at(210., 10.), 10., 1., 2, 200.),
            Some(TabClick::Switch(0))
        );
        assert_eq!(
            strip_hit(at(280., 10.), 10., 1., 2, 200.),
            Some(TabClick::Close(0))
        );
        assert_eq!(
            strip_hit(at(390., 10.), 10., 1., 0, 200.),
            None,
            "no tab and past the plus"
        );
        assert_eq!(
            strip_hit(at(205., 10.), 10., 1., 0, 200.),
            Some(TabClick::New)
        );
    }

    // A dragged tab (#682) takes the place its middle is over, held to the
    // strip's ends. At a 10 px font a tab is 90 wide; three tabs here.
    #[test]
    fn a_dragged_tab_lands_where_its_middle_is() {
        assert_eq!(drop_index(0, 0., 90., 3), 0);
        assert_eq!(
            drop_index(0, 44., 90., 3),
            0,
            "not yet past the next tab's edge"
        );
        assert_eq!(drop_index(0, 46., 90., 3), 1);
        assert_eq!(
            drop_index(0, 500., 90., 3),
            2,
            "the last place, not past it"
        );
        assert_eq!(drop_index(2, -136., 90., 3), 0);
        assert_eq!(drop_index(1, -500., 90., 3), 0);
        assert_eq!(drop_index(0, 10., 90., 0), 0, "no tabs, nothing moves");
    }

    // Under the slop a press is only the click that switched tabs.
    #[test]
    fn a_drag_starts_past_the_slop_and_drops_only_when_it_moved() {
        let mut d = TabDrag::new(0, 20.);
        assert!(!d.moved(23., 10., 1.), "3 is under the 5 slop");
        assert_eq!(d.drop(10., 1., 3), None);
        assert!(d.moved(120., 10., 1.));
        assert_eq!(d.drop(10., 1., 3), Some((0, 1)));
        // Back where it started: a drag, but no move to make.
        d.moved(22., 10., 1.);
        assert!(d.moving);
        assert_eq!(d.drop(10., 1., 3), None);
    }

    // The lock is `card.protect`'s, not this strip's own `locked`: it
    // shows or not by the card's saved flag alone.
    #[test]
    fn a_protected_card_wears_the_lock_glyph_beside_its_number() {
        assert_eq!(number_label(7, false), "#7");
        assert_eq!(number_label(7, true), "\u{1f512} #7");
    }
}
