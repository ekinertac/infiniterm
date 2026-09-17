//! Screen-sized card borders, focus rings, labels, and UI scale limits.
//! Port of chrome.ts and its tests. UI rendering consumes these pure measurements.
//! Only labels use UI scale; terminal metrics and card geometry remain separate.
use crate::grid::round;
pub const UI_SCALE_MIN: f64 = 0.6;
pub const UI_SCALE_MAX: f64 = 2.5;
pub const UI_SCALE_STEP: f64 = 0.1;
/// The focus ring, in screen pixels, stepped with the zoom. It was 2 px at
/// 45% alpha up close, a half-transparent hairline that read as "faintly
/// there" on a resting card and as nothing at all beside a 4 px agent
/// border in yellow or green. The ring is the one thing that says where
/// you are, so it is wide enough to see and nearly opaque at every zoom.
pub const RING_NEAR: f64 = 3.;
pub const RING_MID: f64 = 4.;
pub const RING_FAR: f64 = 5.;
pub const RING_ALPHA_NEAR: f64 = 0.9;
pub const RING_ALPHA_MID: f64 = 0.95;
pub const RING_ALPHA_FAR: f64 = 1.;
/// Canvas between the card's border and the ring, as a multiple of the
/// ring's width. Touching a solid agent border the white ring merged into
/// the colour; floated off it on a strip of canvas it reads as its own
/// shape whatever the border's colour is.
pub const RING_GAP_RATIO: f64 = 1.;
pub const CARD_BORDER_SCREEN_PX: f64 = 2.;
/// A card with an agent in it wears a heavier border than a resting one.
/// Hue cannot carry a signal through two screen pixels: at a glance across
/// a canvas it is the AREA of colour that is read, not the colour.
pub const STATE_BORDER_NEAR: f64 = 4.;
pub const STATE_BORDER_MID: f64 = 6.;
/// Zoomed out the card is a rectangle and the border is the only thing left
/// that says anything, so it takes the largest share it can without eating
/// the card. The steps are the focus ring's, for the same reason.
pub const STATE_BORDER_FAR: f64 = 9.;
/// The corner label's multiplier on `ui.cardLabelSize`, stepped with the
/// zoom.
///
/// A card label is sized in SCREEN pixels, so it never shrank as you zoomed
/// out; what shrank was the card around it, until a name sat in a rectangle
/// barely taller than the text. A second, larger label used to be drawn in
/// the middle of the card to cover that, which meant two pieces of chrome
/// saying the same word. The corner label grows instead, on the focus
/// ring's steps and for the focus ring's reason: the further out you are,
/// the more of the card has to be label for the name to register.
pub const CORNER_LABEL_NEAR: f64 = 1.2;
pub const CORNER_LABEL_MID: f64 = 1.8;
pub const CORNER_LABEL_FAR: f64 = 2.4;
pub fn corner_label_scale(scale: f64) -> f64 {
    if scale >= 0.6 {
        CORNER_LABEL_NEAR
    } else if scale >= 0.25 {
        CORNER_LABEL_MID
    } else {
        CORNER_LABEL_FAR
    }
}
pub fn clamp_ui_scale(value: f64) -> f64 {
    if !value.is_finite() {
        return 1.;
    }
    let stepped = round(value / UI_SCALE_STEP) * UI_SCALE_STEP;
    // Clamp before decimal cleanup to avoid overflow from corrupt finite saves.
    (round(stepped.clamp(UI_SCALE_MIN, UI_SCALE_MAX) * 100.) / 100.)
        .clamp(UI_SCALE_MIN, UI_SCALE_MAX)
}
pub fn focus_ring_screen_px(scale: f64) -> f64 {
    if scale >= 0.6 {
        RING_NEAR
    } else if scale >= 0.25 {
        RING_MID
    } else {
        RING_FAR
    }
}
pub fn focus_ring_alpha(scale: f64) -> f64 {
    if scale >= 0.6 {
        RING_ALPHA_NEAR
    } else if scale >= 0.25 {
        RING_ALPHA_MID
    } else {
        RING_ALPHA_FAR
    }
}
pub fn focus_ring_world_px(scale: f64) -> f64 {
    focus_ring_screen_px(scale) / scale
}
/// The border a card with agent state wears, in SCREEN pixels. Stepped with
/// the zoom like the focus ring: the further out you are, the more of the
/// card has to be border for the state to register at all.
pub fn state_border_screen_px(scale: f64) -> f64 {
    if scale >= 0.6 {
        STATE_BORDER_NEAR
    } else if scale >= 0.25 {
        STATE_BORDER_MID
    } else {
        STATE_BORDER_FAR
    }
}

pub fn card_border_world_px(scale: f64) -> f64 {
    CARD_BORDER_SCREEN_PX / scale
}
pub fn inverse_scale(scale: f64, ui_scale: f64) -> f64 {
    ui_scale / scale
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-10);
    }
    // The same steps as the ring, because it replaced a separate mid-zoom
    // label drawn in the middle of the card: one label, growing, rather
    // than two saying the same thing.
    #[test]
    fn the_corner_label_grows_zooming_out() {
        for (s, e) in [
            (1., CORNER_LABEL_NEAR),
            (0.6, CORNER_LABEL_NEAR),
            (0.59, CORNER_LABEL_MID),
            (0.25, CORNER_LABEL_MID),
            (0.24, CORNER_LABEL_FAR),
            (0.05, CORNER_LABEL_FAR),
        ] {
            assert_eq!(corner_label_scale(s), e, "at {s}");
        }
        // Never smaller as you pull back: the card is shrinking, so a
        // label that shrank with it would be the problem this replaced.
        let zooms = [1., 0.8, 0.6, 0.4, 0.25, 0.1, 0.05];
        for pair in zooms.windows(2) {
            assert!(
                corner_label_scale(pair[1]) >= corner_label_scale(pair[0]),
                "{} to {} made the label smaller",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn ring_thickens_zooming_out() {
        for (s, e) in [
            (1., RING_NEAR),
            (0.6, RING_NEAR),
            (0.59, RING_MID),
            (0.25, RING_MID),
            (0.24, RING_FAR),
            (0.05, RING_FAR),
        ] {
            assert_eq!(focus_ring_screen_px(s), e);
        }
    }
    #[test]
    fn ring_matches_zoom_bands() {
        assert_ne!(focus_ring_screen_px(0.6), focus_ring_screen_px(0.59));
        assert_ne!(focus_ring_screen_px(0.25), focus_ring_screen_px(0.24));
    }
    #[test]
    fn ring_world_converts_to_screen() {
        for s in [1., 0.8, 0.5, 0.3, 0.1, 0.05] {
            close(focus_ring_world_px(s) * s, focus_ring_screen_px(s));
        }
    }
    #[test]
    fn far_zoom_has_wide_world_ring() {
        // 5% zoom: the far ring in world pixels is 20x its screen width.
        assert_eq!(focus_ring_world_px(0.05), RING_FAR / 0.05);
        assert_eq!(focus_ring_world_px(1.), RING_NEAR);
    }
    #[test]
    fn ring_dims_zooming_in() {
        for (s, e) in [
            (1., RING_ALPHA_NEAR),
            (0.6, RING_ALPHA_NEAR),
            (0.59, RING_ALPHA_MID),
            (0.25, RING_ALPHA_MID),
            (0.24, RING_ALPHA_FAR),
        ] {
            assert_eq!(focus_ring_alpha(s), e);
        }
    }
    #[test]
    fn width_and_alpha_strengthen_together() {
        for s in [1., 0.5, 0.1].windows(2) {
            assert!(focus_ring_screen_px(s[1]) >= focus_ring_screen_px(s[0]));
            assert!(focus_ring_alpha(s[1]) >= focus_ring_alpha(s[0]));
        }
    }
    #[test]
    fn border_screen_constant() {
        for s in [4., 1., 0.6, 0.25, 0.1] {
            close(card_border_world_px(s) * s, CARD_BORDER_SCREEN_PX);
        }
    }
    #[test]
    fn border_world_thins_zooming_in() {
        assert_eq!(card_border_world_px(4.), 0.5);
        assert_eq!(card_border_world_px(1.), 2.);
        assert_eq!(card_border_world_px(0.25), 8.);
    }
    #[test]
    fn ring_at_least_border_width() {
        for s in [4., 1., 0.5, 0.2] {
            assert!(focus_ring_screen_px(s) >= CARD_BORDER_SCREEN_PX);
        }
    }
    #[test]
    fn inverse_cancels_canvas_scale() {
        for s in [4., 1., 0.5, 0.1, 0.05] {
            close(inverse_scale(s, 1.) * s, 1.);
        }
    }
    #[test]
    fn world_label_enlarges_zooming_out() {
        assert_eq!(inverse_scale(0.1, 1.), 10.);
        assert_eq!(inverse_scale(1., 1.), 1.);
        assert_eq!(inverse_scale(4., 1.), 0.25);
    }
    #[test]
    fn ui_scale_multiplies_label() {
        assert_eq!(inverse_scale(0.5, 2.), 4.);
        assert_eq!(inverse_scale(0.5, 1.), 2.);
        assert_eq!(inverse_scale(0.25, 1.), 4.);
    }
    #[test]
    fn ui_scale_does_not_reach_borders() {
        assert_eq!(card_border_world_px(2.), 1.);
        assert_eq!(focus_ring_world_px(1.), RING_NEAR);
    }
    #[test]
    fn scale_clamps_and_cleans_steps() {
        assert_eq!(clamp_ui_scale(1.), 1.);
        assert_eq!(clamp_ui_scale(1.0999999999999999), 1.1);
        assert_eq!(clamp_ui_scale(0.01), UI_SCALE_MIN);
        assert_eq!(clamp_ui_scale(99.), UI_SCALE_MAX);
    }
    #[test]
    fn invalid_scale_falls_back() {
        assert_eq!(clamp_ui_scale(f64::NAN), 1.);
        assert_eq!(clamp_ui_scale(f64::INFINITY), 1.);
    }

    // A state border is always heavier than a resting one, and heavier the
    // further out you are: the card shrinks, the signal must not.
    #[test]
    fn a_state_border_thickens_as_the_canvas_shrinks() {
        assert_eq!(state_border_screen_px(1.), STATE_BORDER_NEAR);
        assert_eq!(state_border_screen_px(0.6), STATE_BORDER_NEAR);
        assert_eq!(state_border_screen_px(0.4), STATE_BORDER_MID);
        assert_eq!(state_border_screen_px(0.25), STATE_BORDER_MID);
        assert_eq!(state_border_screen_px(0.1), STATE_BORDER_FAR);
        for scale in [2., 1., 0.6, 0.4, 0.25, 0.1, 0.05] {
            assert!(
                state_border_screen_px(scale) > CARD_BORDER_SCREEN_PX,
                "a state border must outweigh a resting one at {scale}"
            );
        }
    }
}
