//! World geometry and grid snapping for placement, resize, and canvas drawing.
//! Port of src/lib/grid.ts and its tests in the reference app.
//! Positions use cell centers; lengths use whole cells. No rendering dependencies.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}
pub const GRID_SIZE: f64 = 25.;
pub const GRID_MIN_PX: f64 = 6.;
pub const HALF_CELL: f64 = GRID_SIZE / 2.;
/// JavaScript Math.round: ties toward +infinity, including negative zero.
pub(crate) fn round(value: f64) -> f64 {
    let floor = value.floor();
    let rounded = if value - floor < 0.5 {
        floor
    } else {
        floor + 1.
    };
    if rounded == 0. {
        rounded.copysign(value)
    } else {
        rounded
    }
}
pub fn snap(value: f64) -> f64 {
    round(value / GRID_SIZE) * GRID_SIZE
}
pub fn snap_center(value: f64) -> f64 {
    round((value - HALF_CELL) / GRID_SIZE) * GRID_SIZE + HALF_CELL
}
pub fn snap_rect(rect: Rect) -> Rect {
    Rect {
        x: snap_center(rect.x),
        y: snap_center(rect.y),
        w: snap(rect.w).max(GRID_SIZE),
        h: snap(rect.h).max(GRID_SIZE),
    }
}
pub fn grid_cell_px(scale: f64) -> f64 {
    GRID_SIZE * scale
}
pub fn is_grid_visible(scale: f64) -> bool {
    grid_cell_px(scale) >= GRID_MIN_PX
}
/// Callers provide a finite positive scale and finite viewport coordinates.
pub fn grid_line_offsets(world_min: f64, length_px: f64, scale: f64) -> Vec<f64> {
    if length_px < 0. {
        return vec![];
    }
    let mut offsets = Vec::new();
    let mut k = (world_min / GRID_SIZE).ceil();
    loop {
        let offset = (k * GRID_SIZE - world_min) * scale;
        if offset > length_px {
            break;
        }
        offsets.push(offset);
        k += 1.;
    }
    offsets
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cells_are_25_world_units() {
        assert_eq!(GRID_SIZE, 25.0);
    }
    #[test]
    fn snap_rounds_to_nearest_grid_line() {
        for (v, expected) in [
            (0., 0.),
            (12., 0.),
            (13., 25.),
            (25., 25.),
            (37., 25.),
            (38., 50.),
        ] {
            assert_eq!(snap(v), expected);
        }
    }
    #[test]
    fn snap_works_on_negative_coordinates() {
        for (v, expected) in [(-12., -0.0_f64), (-13., -25.), (-25., -25.), (-38., -50.)] {
            assert_eq!(snap(v).to_bits(), expected.to_bits());
        }
    }
    #[test]
    fn snap_rect_snaps_position_and_size() {
        assert_eq!(
            snap_rect(Rect {
                x: 13.,
                y: 37.,
                w: 687.,
                h: 441.
            }),
            Rect {
                x: 12.5,
                y: 37.5,
                w: 675.,
                h: 450.
            }
        );
    }
    #[test]
    fn snap_rect_never_collapses() {
        assert_eq!(
            snap_rect(Rect {
                x: 0.,
                y: 0.,
                w: 4.,
                h: 4.
            }),
            Rect {
                x: HALF_CELL,
                y: HALF_CELL,
                w: 25.,
                h: 25.
            }
        );
    }
    #[test]
    fn cell_size_scales() {
        for (s, p) in [(1., 25.), (2., 50.), (0.4, 10.)] {
            assert_eq!(grid_cell_px(s), p);
        }
    }
    #[test]
    fn dense_grid_hides() {
        for (s, v) in [
            (1., true),
            (0.3, true),
            (0.24, true),
            (0.2, false),
            (0.05, false),
        ] {
            assert_eq!(is_grid_visible(s), v);
        }
    }
    #[test]
    fn lines_start_inside_viewport() {
        assert_eq!(
            grid_line_offsets(0., 100., 1.),
            vec![0., 25., 50., 75., 100.]
        );
    }
    #[test]
    fn lines_account_for_pan() {
        assert_eq!(grid_line_offsets(10., 100., 1.), vec![15., 40., 65., 90.]);
    }
    #[test]
    fn lines_scale_without_drift() {
        let o = grid_line_offsets(0., 100., 0.5);
        assert_eq!(o, vec![0., 12.5, 25., 37.5, 50., 62.5, 75., 87.5, 100.]);
        assert_eq!(o[8] - o[0], 100.);
    }
    #[test]
    fn lines_handle_negative_origin() {
        assert_eq!(grid_line_offsets(-30., 60., 1.), vec![5., 30., 55.]);
    }
    #[test]
    fn degenerate_viewport() {
        assert_eq!(grid_line_offsets(0., 0., 1.), vec![0.]);
        assert!(grid_line_offsets(0., -5., 1.).is_empty());
    }
    #[test]
    fn snap_center_lands_between_lines() {
        for (v, e) in [
            (0., 12.5),
            (12.5, 12.5),
            (20., 12.5),
            (26., 37.5),
            (37.5, 37.5),
        ] {
            assert_eq!(snap_center(v), e);
        }
    }
    #[test]
    fn snap_center_negative() {
        assert_eq!(snap_center(-12.5), -12.5);
        assert_eq!(snap_center(-30.), -37.5);
    }
    #[test]
    fn delta_keeps_center_offset() {
        let x = snap_center(140.);
        for d in [25., -50., 300., -975.] {
            assert_eq!((x + d - HALF_CELL).rem_euclid(GRID_SIZE), 0.);
        }
    }
    #[test]
    fn even_cell_card_center_is_cell_center() {
        let r = Rect {
            x: snap_center(0.),
            y: snap_center(0.),
            w: GRID_SIZE * 28.,
            h: GRID_SIZE * 18.,
        };
        assert_eq!((r.x + r.w / 2. - HALF_CELL).rem_euclid(GRID_SIZE), 0.);
        assert_eq!((r.y + r.h / 2. - HALF_CELL).rem_euclid(GRID_SIZE), 0.);
    }
    // Rust rounds ties away from zero; JavaScript rounds ties toward +infinity.
    #[test]
    fn negative_half_cell_ties_match_javascript() {
        assert_eq!(snap(-12.5).to_bits(), (-0.0_f64).to_bits());
        assert_eq!(snap(-37.5), -25.);
        assert_eq!(snap_center(0.), 12.5);
    }
}
