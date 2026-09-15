//! Pointer and keyboard resize geometry in world units.
//! Port of resize.ts and cardActions.ts; callers convert screen deltas by scale.
//! Derive moving edges from fixed edges so minimum clamps never move the card.
use crate::grid::{Rect, GRID_SIZE};
pub const MIN_CARD_W: f64 = GRID_SIZE * 8.;
pub const MIN_CARD_H: f64 = GRID_SIZE * 4.;
pub const MOVE_STEP: f64 = GRID_SIZE;
pub const RESIZE_STEP: f64 = GRID_SIZE;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    N,
    S,
    E,
    W,
    Ne,
    Nw,
    Se,
    Sw,
}
pub fn apply_resize(start: Rect, edge: Edge, dx: f64, dy: f64) -> Rect {
    let mut r = start;
    if matches!(edge, Edge::E | Edge::Ne | Edge::Se) {
        r.w = (start.w + dx).max(MIN_CARD_W);
    }
    if matches!(edge, Edge::W | Edge::Nw | Edge::Sw) {
        r.w = (start.w - dx).max(MIN_CARD_W);
        r.x = start.x + start.w - r.w;
    }
    if matches!(edge, Edge::S | Edge::Se | Edge::Sw) {
        r.h = (start.h + dy).max(MIN_CARD_H);
    }
    if matches!(edge, Edge::N | Edge::Ne | Edge::Nw) {
        r.h = (start.h - dy).max(MIN_CARD_H);
        r.y = start.y + start.h - r.h;
    }
    r
}
pub fn moved_by(rect: Rect, dx: f64, dy: f64) -> Rect {
    Rect {
        x: rect.x + dx,
        y: rect.y + dy,
        ..rect
    }
}
pub fn resized_by(rect: Rect, dw: f64, dh: f64) -> Rect {
    apply_resize(rect, Edge::Se, dw, dh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::{snap_center, HALF_CELL};
    const BASE: Rect = Rect {
        x: 100.,
        y: 100.,
        w: 700.,
        h: 450.,
    };
    const CARD: Rect = Rect {
        x: HALF_CELL,
        y: HALF_CELL,
        w: 700.,
        h: 450.,
    };
    #[test]
    fn east_only_right_edge() {
        assert_eq!(
            apply_resize(BASE, Edge::E, 50., 999.),
            Rect { w: 750., ..BASE }
        );
    }
    #[test]
    fn south_only_bottom_edge() {
        assert_eq!(
            apply_resize(BASE, Edge::S, 999., 50.),
            Rect { h: 500., ..BASE }
        );
    }
    #[test]
    fn west_keeps_right_fixed() {
        let r = apply_resize(BASE, Edge::W, 50., 0.);
        assert_eq!(
            r,
            Rect {
                x: 150.,
                w: 650.,
                ..BASE
            }
        );
        assert_eq!(r.x + r.w, BASE.x + BASE.w);
    }
    #[test]
    fn north_keeps_bottom_fixed() {
        let r = apply_resize(BASE, Edge::Nw, 0., 50.);
        assert_eq!(r.y, 150.);
        assert_eq!(r.y + r.h, BASE.y + BASE.h);
    }
    #[test]
    fn corners_resize_both_axes() {
        assert_eq!(
            apply_resize(BASE, Edge::Se, 30., 40.),
            Rect {
                w: 730.,
                h: 490.,
                ..BASE
            }
        );
        assert_eq!(
            apply_resize(BASE, Edge::Nw, -30., -40.),
            Rect {
                x: 70.,
                y: 60.,
                w: 730.,
                h: 490.
            }
        );
    }
    #[test]
    fn shrinking_stops_at_minimum() {
        assert_eq!(apply_resize(BASE, Edge::E, -9999., 0.).w, MIN_CARD_W);
        assert_eq!(apply_resize(BASE, Edge::S, 0., -9999.).h, MIN_CARD_H);
    }
    #[test]
    fn west_clamp_does_not_walk() {
        let r = apply_resize(BASE, Edge::W, 9999., 0.);
        assert_eq!(r.w, MIN_CARD_W);
        assert_eq!(r.x, BASE.x + BASE.w - MIN_CARD_W);
        assert_eq!(r.x + r.w, BASE.x + BASE.w);
    }
    #[test]
    fn north_clamp_does_not_walk() {
        let r = apply_resize(BASE, Edge::Ne, 0., 9999.);
        assert_eq!(r.h, MIN_CARD_H);
        assert_eq!(r.y, BASE.y + BASE.h - MIN_CARD_H);
        assert_eq!(r.y + r.h, BASE.y + BASE.h);
    }
    #[test]
    fn minimums_whole_cells() {
        assert_eq!(MIN_CARD_W % 25., 0.);
        assert_eq!(MIN_CARD_H % 25., 0.);
    }
    #[test]
    fn snapped_deltas_keep_grid() {
        let start = Rect {
            x: snap_center(100.),
            y: snap_center(100.),
            ..BASE
        };
        for edge in [
            Edge::E,
            Edge::W,
            Edge::S,
            Edge::Ne,
            Edge::Nw,
            Edge::Se,
            Edge::Sw,
        ] {
            let r = apply_resize(start, edge, 50., -75.);
            assert_eq!((r.x - HALF_CELL).rem_euclid(25.), 0.);
            assert_eq!((r.y - HALF_CELL).rem_euclid(25.), 0.);
            assert_eq!(r.w % 25., 0.);
            assert_eq!(r.h % 25., 0.);
        }
    }
    #[test]
    fn minimum_is_reachable_on_grid() {
        let r = apply_resize(CARD, Edge::W, 9999., 0.);
        assert_eq!((r.x - HALF_CELL).rem_euclid(25.), 0.);
        assert_eq!(r.w % 25., 0.);
    }
    #[test]
    fn keyboard_steps_one_cell() {
        assert_eq!(MOVE_STEP, 25.);
        assert_eq!(RESIZE_STEP, 25.);
    }
    #[test]
    fn move_preserves_size() {
        assert_eq!(moved_by(CARD, MOVE_STEP, 0.), Rect { x: 37.5, ..CARD });
        assert_eq!(moved_by(CARD, 0., -MOVE_STEP), Rect { y: -12.5, ..CARD });
    }
    #[test]
    fn repeated_moves_keep_centers() {
        let mut r = CARD;
        for _ in 0..7 {
            r = moved_by(r, MOVE_STEP, -MOVE_STEP);
        }
        assert_eq!((r.x - HALF_CELL).rem_euclid(25.), 0.);
        assert_eq!((r.y - HALF_CELL).rem_euclid(25.), 0.);
    }
    #[test]
    fn keyboard_resize_from_bottom_right() {
        assert_eq!(
            resized_by(CARD, RESIZE_STEP, RESIZE_STEP),
            Rect {
                w: 725.,
                h: 475.,
                ..CARD
            }
        );
    }
    #[test]
    fn keyboard_resize_respects_minimum() {
        let mut r = CARD;
        for _ in 0..200 {
            r = resized_by(r, -RESIZE_STEP, -RESIZE_STEP);
        }
        assert_eq!(
            r,
            Rect {
                w: MIN_CARD_W,
                h: MIN_CARD_H,
                ..CARD
            }
        );
    }
}
