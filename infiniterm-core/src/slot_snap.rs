//! Where a dragged card's ghost snaps: the slots a card of its size can
//! occupy, not the 25 px grid. Ekin's ask, 2026-09-19: a quarter card
//! dragged over empty space should land in quarter slots.
//!
//! There is no fixed lattice of slots: a split makes halves of 1000 and
//! 975 from 2000, so a quarter's real slots are wherever the cards around
//! it leave room. The ghost snaps each axis to the nearest EDGE the cards
//! on the canvas offer (a card's left or top edge, or its right or bottom
//! edge plus the gutter, or the same minus the ghost's own size, so it
//! sits flush on either side), within `SNAP_RANGE` of the pointer's
//! position; past that range, on empty canvas, to the lattice the
//! placement uses (the default size plus the gutter from the origin). Each
//! axis snaps on its own, so a card can share a row with one neighbour
//! and a column with another.
//!
//! Called from `Model::snap_ghost`; drawn by `paint.rs`; the drop itself is
//! `Model::drop_card`. Related: `layout::nearest_free_slot` (the same
//! lattice), `alignment.rs` (the guides, which want exact matches and so
//! benefit from this).
use crate::grid::{Point, Rect, Size};

/// How far (in world px) a ghost is pulled to an edge. A quarter of the
/// ghost's own extent on that axis: far enough that a slot is easy to
/// hit, near enough that the pull is never surprising.
pub const SNAP_RANGE_RATIO: f64 = 0.25;

/// The ghost at `free` (the pointer's unsnapped rect) snapped to the slots
/// `others` and the lattice offer. `default` is the default card size,
/// `origin` the lattice's origin.
pub fn snap_ghost(free: Rect, others: &[Rect], default: Size, origin: Point, gutter: f64) -> Rect {
    let xs: Vec<f64> = others
        .iter()
        .flat_map(|o| {
            [
                o.x,
                o.x + o.w + gutter,
                o.x - gutter - free.w,
                o.x + o.w - free.w,
            ]
        })
        .collect();
    let ys: Vec<f64> = others
        .iter()
        .flat_map(|o| {
            [
                o.y,
                o.y + o.h + gutter,
                o.y - gutter - free.h,
                o.y + o.h - free.h,
            ]
        })
        .collect();
    Rect {
        x: snap_axis(free.x, &xs, free.w, default.w, origin.x, gutter),
        y: snap_axis(free.y, &ys, free.h, default.h, origin.y, gutter),
        w: free.w,
        h: free.h,
    }
}

fn snap_axis(at: f64, edges: &[f64], extent: f64, pitch: f64, origin: f64, gutter: f64) -> f64 {
    let range = extent * SNAP_RANGE_RATIO;
    let nearest = edges
        .iter()
        .copied()
        .filter(|e| (e - at).abs() <= range)
        .min_by(|a, b| (a - at).abs().partial_cmp(&(b - at).abs()).unwrap());
    if let Some(e) = nearest {
        return e;
    }
    // Empty canvas: the placement's own lattice.
    let step = pitch + gutter;
    origin + ((at - origin) / step).round() * step
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }
    const D: Size = Size { w: 1725., h: 2000. };
    const O: Point = Point { x: 12.5, y: 12.5 };
    const G: f64 = 25.;

    // A quarter dragged near the gap a split left lands exactly in it,
    // whichever of the two uneven halves the gap is.
    #[test]
    fn a_quarter_snaps_into_the_gap_beside_a_split_half() {
        // The kept half (1000 tall) at the top; the freed space below it
        // starts at y = 12.5 + 1000 + 25 = 1037.5, x flush with it.
        let kept = r(12.5, 12.5, 850., 1000.);
        let ghost = r(40., 1060., 850., 975.);
        let s = snap_ghost(ghost, &[kept], D, O, G);
        assert_eq!((s.x, s.y), (12.5, 1037.5));
        // Right of the kept half: x = 12.5 + 850 + 25.
        let ghost = r(900., 30., 850., 1000.);
        let s = snap_ghost(ghost, &[kept], D, O, G);
        assert_eq!((s.x, s.y), (887.5, 12.5));
    }

    // Flush on the far side too: a ghost just left of a card ends where its
    // right edge meets the card's left edge minus the gutter.
    #[test]
    fn a_ghost_left_of_a_card_sits_flush_against_it() {
        let card = r(2000., 12.5, 1725., 2000.);
        let ghost = r(200., 12.5, 1725., 2000.);
        let s = snap_ghost(ghost, &[card], D, O, G);
        assert_eq!(s.x, 2000. - 25. - 1725.);
    }

    // Out on empty canvas nothing pulls, so the lattice does: default
    // pitch from the origin.
    #[test]
    fn empty_canvas_uses_the_placement_lattice() {
        let ghost = r(1800., 2100., 1725., 2000.);
        let s = snap_ghost(ghost, &[], D, O, G);
        assert_eq!((s.x, s.y), (12.5 + 1750., 12.5 + 2025.));
    }

    // Each axis on its own: the row from one neighbour, the column from
    // another.
    #[test]
    fn axes_snap_independently() {
        let row = r(12.5, 3000., 1725., 2000.);
        let col = r(5000., 12.5, 1725., 2000.);
        let ghost = r(5020., 3010., 1725., 2000.);
        let s = snap_ghost(ghost, &[row, col], D, O, G);
        assert_eq!((s.x, s.y), (5000., 3000.));
    }
}
