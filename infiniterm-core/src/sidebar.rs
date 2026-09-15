//! Per-card sidebar extent, clamping, and drag conversion.
//! Port of sidebar.ts and its tests. Editor, diff, and transcript views share this.
//! Stored values use card pixels; the content retains its own minimum extent.
use crate::grid::{round, Size};
pub const DEFAULT_SIDEBAR: f64 = 280.;
pub const MIN_SIDEBAR: f64 = 120.;
pub const MIN_CONTENT: f64 = 160.;
pub fn clamp_sidebar(size: f64, extent: f64) -> f64 {
    let max = (extent - MIN_CONTENT).max(MIN_SIDEBAR);
    round(size.clamp(MIN_SIDEBAR, max))
}
pub fn sidebar_width(stored: Option<f64>, extent: f64) -> f64 {
    clamp_sidebar(stored.unwrap_or(DEFAULT_SIDEBAR), extent)
}
pub fn sidebar_extent(rect: Size, top: bool) -> f64 {
    if top {
        rect.h
    } else {
        rect.w
    }
}
pub fn dragged_sidebar(start: f64, d_screen: f64, scale: f64, extent: f64) -> f64 {
    let scale = if scale == 0. || scale.is_nan() {
        1.
    } else {
        scale
    };
    clamp_sidebar(start + d_screen / scale, extent)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_without_stored_width() {
        assert_eq!(sidebar_width(None, 1000.), DEFAULT_SIDEBAR);
    }
    #[test]
    fn minimum_and_content_space() {
        assert_eq!(clamp_sidebar(10., 1000.), MIN_SIDEBAR);
        assert_eq!(clamp_sidebar(950., 1000.), 1000. - MIN_CONTENT);
    }
    #[test]
    fn minimum_when_card_too_narrow() {
        assert_eq!(clamp_sidebar(500., 200.), MIN_SIDEBAR);
    }
    #[test]
    fn extent_depends_on_orientation() {
        let s = Size { w: 800., h: 600. };
        assert_eq!(sidebar_extent(s, false), 800.);
        assert_eq!(sidebar_extent(s, true), 600.);
    }
    #[test]
    fn drag_delta_divided_by_zoom() {
        assert_eq!(dragged_sidebar(280., 100., 1., 1000.), 380.);
        assert_eq!(dragged_sidebar(280., 100., 0.5, 1000.), 480.);
        assert_eq!(dragged_sidebar(280., -50., 2., 1000.), 255.);
    }
    #[test]
    fn zero_scale_survives() {
        assert_eq!(dragged_sidebar(280., 20., 0., 1000.), 300.);
    }
}
