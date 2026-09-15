//! Screen-sized card borders, focus rings, labels, and UI scale limits.
//! Port of chrome.ts and its tests. UI rendering consumes these pure measurements.
//! Only labels use UI scale; terminal metrics and card geometry remain separate.
use crate::grid::round;
pub const UI_SCALE_MIN: f64 = 0.6;
pub const UI_SCALE_MAX: f64 = 2.5;
pub const UI_SCALE_STEP: f64 = 0.1;
pub const RING_NEAR: f64 = 2.;
pub const RING_MID: f64 = 3.;
pub const RING_FAR: f64 = 4.;
pub const RING_ALPHA_NEAR: f64 = 0.45;
pub const RING_ALPHA_MID: f64 = 0.75;
pub const RING_ALPHA_FAR: f64 = 1.;
pub const CARD_BORDER_SCREEN_PX: f64 = 2.;
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
        assert_eq!(focus_ring_world_px(0.05), 80.);
        assert_eq!(focus_ring_world_px(1.), 2.);
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
}
