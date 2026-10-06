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
//! Called by `editor_tabs.rs`. `body::TabClick` is the shared answer.
use crate::body::TabClick;
use gpui::{fill, font, point, px, size, App, Bounds, Hsla, Pixels, Window};
use infiniterm_core::grid::Point;

/// The strip is sized from its FONT, and the font is the terminal's
/// (`terminal.fontSize`, in screen pixels, times `ui_scale`): an 11 px
/// strip under 19 px text was unreadable. Everything else is a ratio of
/// that, so the strip keeps its proportions at any size.
pub const TAB_STRIP_HEIGHT_RATIO: f64 = 1.9;
/// A tab's width in font sizes. Fixed rather than proportional to the
/// card: a card with many tabs would otherwise shrink every tab to a
/// sliver.
pub const TAB_STRIP_TAB_WIDTH_RATIO: f64 = 9.;
pub const TAB_STRIP_LABEL_PAD_RATIO: f64 = 0.6;
/// The close button's band at a tab's right edge.
pub const TAB_STRIP_CLOSE_WIDTH_RATIO: f64 = 1.5;
pub const TAB_STRIP_BORDER_PX: f64 = 1.;

/// The strip's colours, mirrored from the chrome each frame.
#[derive(Clone, Debug, PartialEq)]
pub struct StripStyle {
    pub bg: Hsla,
    pub border: Hsla,
    pub active_bg: Hsla,
    pub text_bright: Hsla,
    pub text_muted: Hsla,
    pub font_family: String,
    /// `terminal.fontSize`: the strip's font, in screen pixels.
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
pub fn strip_hit(local: Point, font_px: f64, ui_scale: f32, tab_count: usize) -> Option<TabClick> {
    let strip_h = strip_world_h(font_px, ui_scale);
    if local.y < 0. || local.y >= strip_h {
        return None;
    }
    let unit = font_px * ui_scale as f64;
    let tab_w = TAB_STRIP_TAB_WIDTH_RATIO * unit;
    let index = (local.x / tab_w).floor().max(0.) as usize;
    if index == tab_count {
        return Some(TabClick::New);
    }
    if index > tab_count {
        return None;
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
#[allow(clippy::too_many_arguments)]
pub fn paint_strip(
    bounds: Bounds<Pixels>,
    scale: f64,
    ui_scale: f32,
    labels: &[String],
    active: usize,
    card_number: u32,
    protected: bool,
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
    let f = font(style.font_family.clone());
    if strip_font >= crate::chrome::legible_font_px(window.scale_factor()) {
        let pad = px((TAB_STRIP_LABEL_PAD_RATIO * unit) as f32);
        for (i, label) in labels.iter().enumerate() {
            let tab_bounds = Bounds::new(
                point(strip.origin.x + tab_w * (i as f32), strip.origin.y),
                size(tab_w, strip_h),
            );
            let color = if i == active {
                window.paint_quad(fill(tab_bounds, style.active_bg));
                style.text_bright
            } else {
                style.text_muted
            };
            let sep = Bounds::new(
                point(tab_bounds.origin.x + tab_w - border, tab_bounds.origin.y),
                size(border, strip_h),
            );
            window.paint_quad(fill(sep, style.border));
            let room = f32::from(close_left) - f32::from(pad) * 2.;
            let shown = crate::text::elide(label, room, |t| {
                f32::from(crate::text::shape(window, t, strip_font, &f, color).width)
            });
            let line = crate::text::shape(window, &shown, strip_font, &f, color);
            crate::text::paint_in(window, cx, &line, tab_bounds, pad);
            let close_line = crate::text::shape(window, "×", strip_font, &f, color);
            let close_bounds = Bounds::new(
                point(
                    tab_bounds.origin.x + close_left + (close_w - close_line.width) / 2.,
                    tab_bounds.origin.y,
                ),
                size(close_line.width, strip_h),
            );
            crate::text::paint_in(window, cx, &close_line, close_bounds, px(0.));
        }
        let plus = crate::text::shape(window, "+", strip_font, &f, style.text_muted);
        let plus_bounds = Bounds::new(
            point(
                strip.origin.x + tab_w * (labels.len() as f32) + tab_w / 2. - plus.width / 2.,
                strip.origin.y,
            ),
            size(plus.width, strip_h),
        );
        crate::text::paint_in(window, cx, &plus, plus_bounds, px(0.));
        if card_number > 0 {
            let number = number_label(card_number, protected);
            let line = crate::text::shape(window, &number, strip_font, &f, style.text_muted);
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
    // band after the last tab is "+".
    #[test]
    fn a_click_lands_on_a_tab_its_close_or_the_plus() {
        let at = |x: f64, y: f64| Point { x, y };
        assert_eq!(
            strip_hit(at(10., 10.), 10., 1., 2),
            Some(TabClick::Switch(0))
        );
        assert_eq!(
            strip_hit(at(80., 10.), 10., 1., 2),
            Some(TabClick::Close(0))
        );
        assert_eq!(
            strip_hit(at(100., 10.), 10., 1., 2),
            Some(TabClick::Switch(1))
        );
        assert_eq!(strip_hit(at(200., 10.), 10., 1., 2), Some(TabClick::New));
        assert_eq!(strip_hit(at(400., 10.), 10., 1., 2), None, "past the plus");
        assert_eq!(strip_hit(at(10., 25.), 10., 1., 2), None, "below the strip");
        // Twice the interface scale: everything twice as big.
        assert_eq!(
            strip_hit(at(100., 25.), 10., 2., 2),
            Some(TabClick::Switch(0))
        );
    }

    // The lock is `card.protect`'s, not this strip's own `locked`: it
    // shows or not by the card's saved flag alone.
    #[test]
    fn a_protected_card_wears_the_lock_glyph_beside_its_number() {
        assert_eq!(number_label(7, false), "#7");
        assert_eq!(number_label(7, true), "\u{1f512} #7");
    }
}
