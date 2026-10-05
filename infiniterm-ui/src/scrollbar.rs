//! The scrollbar thumb every scrolling body draws: a thin rounded bar at the
//! right edge of the area it scrolls, only when there is more than shows.
//!
//! Called by `page_body.rs` and `editor_body.rs`. The thumb's position is
//! core's `scrollbar::thumb` (pure, tested); this file picks the size and
//! colour and paints the one rectangle. Sizes are SCREEN pixels, like every
//! affordance, so the bar keeps its width at any zoom while the text scales.
//! It is a mark, not a control: no hover, no drag. Painted by the caller
//! before an inactive dim, so it dims with its card.

use gpui::{fill, point, px, size, Bounds, Hsla, Pixels, Window};

const WIDTH_PX: f32 = 6.;
const INSET_PX: f32 = 4.;
/// A page of thousands of lines would draw a sliver nobody can see.
const MIN_LEN_PX: f64 = 28.;
const ALPHA: f32 = 0.6;

/// `total`, `visible` and `offset` in the body's own rows (lines); `color`
/// is its faint text colour, which the bar wears at `ALPHA`.
pub fn paint(
    window: &mut Window,
    area: Bounds<Pixels>,
    total: usize,
    visible: usize,
    offset: usize,
    color: Hsla,
) {
    let inset = px(INSET_PX);
    let track = f32::from(area.size.height - inset * 2.) as f64;
    let Some((start, len)) =
        infiniterm_core::scrollbar::thumb(total, visible, offset, track, MIN_LEN_PX)
    else {
        return;
    };
    let thumb = Bounds::new(
        point(
            area.origin.x + area.size.width - inset - px(WIDTH_PX),
            area.origin.y + inset + px(start as f32),
        ),
        size(px(WIDTH_PX), px(len as f32)),
    );
    window.paint_quad(
        fill(thumb, crate::chrome::with_alpha(color, color.a * ALPHA))
            .corner_radii(px(WIDTH_PX / 2.)),
    );
}
