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

/// The layout size (CSS pixels) a painted frame was REALLY laid out at, found
/// from its own pixels (#479, the race behind "sometimes it works").
///
/// A frame used to be labelled with the last size CEF had asked for in
/// `view_rect`. That is not the size of the frame that arrives next: after a
/// quick burst of resizes CEF asks for the newest size, then still delivers a
/// frame it had already composited at an older one. Labelled with the new
/// size, such a frame looked current: the ui stretched its pixels to fill the
/// card, and CEF, believing it had painted the newest size, sent nothing
/// after it, so the page stayed stretched. The pixels do not lie: the frame is
/// laid out at the (size, device scale) pair, among those CEF asked for
/// recently, whose product is its pixel size. `asked` and `scales` are what
/// CEF asked (newest last); when no pair fits, the pixels over `fallback_scale`.
pub fn frame_layout_size(
    px_w: u32,
    px_h: u32,
    asked: &[(i32, i32)],
    scales: &[f32],
    fallback_scale: f32,
) -> (i32, i32) {
    let fits = |css: i32, scale: f32, px: u32| (css as f64 * scale as f64 - px as f64).abs() <= 1.;
    // The newest scale first: the same pixels can be 1600 css at 1x or 800 css
    // at 2x, and after a zoom the newest scale is the likelier one.
    for &scale in scales.iter().rev() {
        for &(w, h) in asked.iter().rev() {
            if fits(w, scale, px_w) && fits(h, scale, px_h) {
                return (w, h);
            }
        }
    }
    let scale = if fallback_scale > 0. { fallback_scale as f64 } else { 1. };
    (
        (px_w as f64 / scale).round() as i32,
        (px_h as f64 / scale).round() as i32,
    )
}

/// How long the card must have been quiet (no new frame, no new size wanted)
/// before it asks CEF to paint again at the size it wants. A drag sends sizes
/// every frame and CEF answers within a few frames; this is well past that.
pub const KICK_AFTER_MS: f64 = 150.;
/// The least time between two such requests for the same size.
pub const KICK_EVERY_MS: f64 = 400.;
/// Requests for one wanted size before giving up (a page that cannot be laid
/// out at the size asked would otherwise be asked for ever).
pub const MAX_KICKS: u32 = 5;

/// Whether the card should tell CEF to paint again now (#479). The newest
/// frame was laid out at `frame_css` and the card wants `wanted_css`: while they
/// differ the page is drawn at an old size, and CEF will not send another frame
/// on its own if it believes it already painted the newest size. `quiet_ms` is
/// the time since a frame last arrived or the wanted size last changed,
/// `since_kick_ms` the time since the last request, `kicks` how many were made
/// for this wanted size. One CSS pixel of difference is rounding, not a stale
/// frame.
pub fn needs_kick(
    frame_css: (i32, i32),
    wanted_css: (i32, i32),
    quiet_ms: f64,
    since_kick_ms: f64,
    kicks: u32,
) -> bool {
    is_stale(frame_css, wanted_css)
        && kicks < MAX_KICKS
        && quiet_ms >= KICK_AFTER_MS
        && since_kick_ms >= KICK_EVERY_MS
}

/// The frame is not the size the card wants (more than rounding apart).
pub fn is_stale(frame_css: (i32, i32), wanted_css: (i32, i32)) -> bool {
    (frame_css.0 - wanted_css.0).abs() > 1 || (frame_css.1 - wanted_css.1).abs() > 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_laid_out_at_the_size_its_pixels_prove() {
        // The race seen on the MacBook: CEF had asked for 1625 wide, the last
        // ones before it were 1575 and 1550, and the frame that arrived was
        // 1575 pixels wide. It must be labelled 1575, not the newest ask.
        let asked = [(1550, 800), (1575, 800), (1625, 800)];
        assert_eq!(frame_layout_size(1575, 800, &asked, &[1.], 1.), (1575, 800));
        // The newest ask is the answer when the pixels say so.
        assert_eq!(frame_layout_size(1625, 800, &asked, &[1.], 1.), (1625, 800));
    }

    #[test]
    fn the_device_scale_is_part_of_the_proof() {
        // 800 css at 2.0 is 1600 pixels; the same pixels are not 1600 css.
        let asked = [(800, 600), (1600, 1200)];
        assert_eq!(frame_layout_size(1600, 1200, &asked, &[2.], 2.), (800, 600));
        // The scale changed in between: the older scale still explains an
        // older frame.
        assert_eq!(frame_layout_size(800, 600, &asked, &[1., 2.], 2.), (800, 600));
        assert_eq!(frame_layout_size(1600, 1200, &asked, &[1., 2.], 2.), (800, 600));
    }

    #[test]
    fn a_frame_no_ask_explains_falls_back_to_the_pixels_over_the_scale() {
        assert_eq!(frame_layout_size(1000, 500, &[], &[], 2.), (500, 250));
        assert_eq!(frame_layout_size(1000, 500, &[(333, 333)], &[1.], 0.), (1000, 500));
    }

    #[test]
    fn rounding_of_one_pixel_still_matches() {
        // 1475 css at 1.5 is 2212.5 pixels: CEF paints 2212 or 2213.
        assert_eq!(frame_layout_size(2212, 1200, &[(1475, 800)], &[1.5], 1.5), (1475, 800));
        assert_eq!(frame_layout_size(2213, 1200, &[(1475, 800)], &[1.5], 1.5), (1475, 800));
    }

    #[test]
    fn a_stale_frame_asks_for_a_repaint_only_after_a_quiet_moment() {
        let (frame, wanted) = ((1575, 800), (1625, 800));
        // Still being dragged or a frame just came: wait.
        assert!(!needs_kick(frame, wanted, 40., 1e9, 0));
        // Quiet for long enough: ask.
        assert!(needs_kick(frame, wanted, KICK_AFTER_MS, 1e9, 0));
        // Just asked: do not ask again at once.
        assert!(!needs_kick(frame, wanted, 1000., 100., 1));
        assert!(needs_kick(frame, wanted, 1000., KICK_EVERY_MS, 1));
        // Gave up after MAX_KICKS for this size.
        assert!(!needs_kick(frame, wanted, 5000., 5000., MAX_KICKS));
    }

    #[test]
    fn a_frame_at_the_wanted_size_never_asks() {
        assert!(!needs_kick((1625, 800), (1625, 800), 5000., 5000., 0));
        // One pixel of rounding is not stale.
        assert!(!needs_kick((1624, 801), (1625, 800), 5000., 5000., 0));
        assert!(is_stale((1623, 800), (1625, 800)));
    }

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
