//! Percentage formatting for the status bar.
//! Port of formatZoom.ts and its tests; uses the reference rounding rule in grid.rs.
//! Even sub-percent canvas scales must not display as zero percent.
use crate::grid::round;
pub fn format_zoom(scale: f64) -> String {
    let pct = scale * 100.;
    let rounded = if pct < 1. {
        round(pct).max(1.)
    } else {
        round(pct)
    };
    format!("{rounded}%")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn common_zoom_levels() {
        for (s, e) in [(1., "100%"), (0.5, "50%"), (4., "400%"), (0.05, "5%")] {
            assert_eq!(format_zoom(s), e);
        }
    }
    #[test]
    fn whole_percent_rounding() {
        assert_eq!(format_zoom(0.333), "33%");
        assert_eq!(format_zoom(1.006), "101%");
    }
    #[test]
    fn never_zero_percent() {
        assert_eq!(format_zoom(0.001), "1%");
        assert_eq!(format_zoom(0.), "1%");
    }
}
