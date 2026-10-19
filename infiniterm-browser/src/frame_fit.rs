//! Where a painted CEF frame is drawn inside the card's page area (#479).
//!
//! The browser lays the page out at the card's world size and the ui paints
//! the frame as a texture. A resize reaches CEF at once, but the repaint at
//! the new size arrives a moment later; until it does, the newest frame is the
//! OLD size. Painting it into the new bounds stretched the page (squashed text
//! while a card's edge was dragged). The rule here: a frame is drawn at the
//! size it was laid out at, times the canvas zoom, anchored top-left, so the
//! card shows a page of the old size, clipped or with empty card background
//! beside it, until the new frame replaces it. A canvas ZOOM is not a stretch:
//! the layout size is unchanged, so the natural size follows the zoom and an
//! unresized card always fills its bounds exactly.
//!
//! Pure so it is tested without a window. Called from
//! `infiniterm-ui/src/browser_body.rs` (`paint`); the layout size comes from
//! `Frame::css_width`/`css_height` (surface.rs). No gpui here.

/// How far (screen pixels) a natural size may be from the available one and
/// still count as the same: the card's bounds are fractional and CEF's layout
/// size is rounded to whole CSS pixels, so a settled card is up to about a pixel
/// off. Treating that as equal keeps a settled page flush with its card instead
/// of leaving a one-pixel seam.
pub const SNAP_PX: f64 = 1.5;

/// The size, in screen pixels, to draw a frame at. `css_w` and `css_h` are the
/// layout size the frame was painted at (0 when unknown), `scale` the canvas
/// zoom (screen pixels per CSS pixel), `avail_w` and `avail_h` the page area
/// of the card now.
///
/// Unknown layout size: the available size (the old behaviour; only a frame
/// from before CEF has ever asked for a view rect).
pub fn frame_draw_size(css_w: i32, css_h: i32, scale: f64, avail_w: f64, avail_h: f64) -> (f64, f64) {
    if css_w <= 0 || css_h <= 0 || scale <= 0. {
        return (avail_w, avail_h);
    }
    let snap = |natural: f64, avail: f64| {
        if (natural - avail).abs() <= SNAP_PX {
            avail
        } else {
            natural
        }
    };
    (
        snap(css_w as f64 * scale, avail_w),
        snap(css_h as f64 * scale, avail_h),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settled_card_is_filled_exactly() {
        // Layout 800 x 600 at zoom 1: the page area is 800 x 600.
        assert_eq!(frame_draw_size(800, 600, 1., 800., 600.), (800., 600.));
        // Fractional bounds a hair off the rounded layout size still fill.
        assert_eq!(frame_draw_size(800, 600, 1., 800.4, 599.6), (800.4, 599.6));
    }

    #[test]
    fn a_zoom_is_not_a_stretch() {
        // The card is not resized, the canvas zooms: the layout stays 800 x 600
        // and the page area follows the zoom, so the natural size equals it at
        // every step and the page is scaled with the canvas as before.
        for z in [0.25, 0.5, 1., 1.5, 3.] {
            assert_eq!(
                frame_draw_size(800, 600, z, 800. * z, 600. * z),
                (800. * z, 600. * z),
                "zoom {z}"
            );
        }
    }

    #[test]
    fn a_card_dragged_wider_does_not_stretch_the_old_frame() {
        // The page was laid out at 800 wide; the card is now 1000 wide at zoom
        // 1 and CEF has not repainted: the frame keeps 800, not 1000.
        let (w, h) = frame_draw_size(800, 600, 1., 1000., 600.);
        assert_eq!((w, h), (800., 600.));
    }

    #[test]
    fn a_card_dragged_narrower_keeps_the_old_size_to_be_clipped() {
        // 800 laid out, card now 500: the frame stays 800 wide and the ui clips
        // it to the page area.
        let (w, h) = frame_draw_size(800, 600, 1., 500., 400.);
        assert_eq!((w, h), (800., 600.));
    }

    #[test]
    fn the_natural_size_follows_the_zoom_during_a_resize() {
        // Zoomed to 50% while the card is mid-resize: the old frame is still
        // drawn at its layout size times the zoom, never at the new bounds.
        let (w, h) = frame_draw_size(800, 600, 0.5, 600., 300.);
        assert_eq!((w, h), (400., 300.));
    }

    #[test]
    fn a_frame_with_no_known_layout_fills_the_card() {
        assert_eq!(frame_draw_size(0, 0, 1., 640., 480.), (640., 480.));
        assert_eq!(frame_draw_size(800, 600, 0., 640., 480.), (640., 480.));
    }
}
