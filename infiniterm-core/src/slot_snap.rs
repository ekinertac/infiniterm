//! Where a dragged card's ghost snaps: the SLOTS of the app's grid for a
//! card of its size, and what the drag draws to show them.
//!
//! Since 2026-09-27 the grid decides, not the neighbours. The ghost used to
//! snap to the nearest edge a card nearby offered, so a canvas arranged by
//! hand drifted off the grid one drag at a time, and every card Cmd+T made
//! afterwards landed out of line with them (Ekin's Streaming workspace).
//! Now each axis snaps to the nearest slot start, and the drag draws the
//! slots around the ghost (`slots_near`), so where a card belongs is
//! visible before the drop.
//!
//! Slots come in paper sizes (Ekin: "a card like an A4 paper, half A5,
//! then A6"): a card smaller than the full slot gets the full slot divided
//! the way a split divides it (`pieces`), so halves are 850 + 25 + 850 of
//! 1725 wide and 1000 + 25 + 975 of 2000 tall, quarters divide again, and
//! every piece lines up with the full grid.
//!
//! Called from `Model::snap_ghost` and `Model::ghost_slots`; drawn by
//! `paint.rs`; the drop itself is `Model::drop_card`. Related:
//! `layout::fill_slot` (the same grid, anchored at the canvas's half cell).
use crate::grid::{Point, Rect, Size, GRID_SIZE};

/// How many pieces of `size` a full slot of `full` holds along one axis,
/// and where each starts inside it: the split's rule, the first pieces
/// rounded up to the grid and the last taking what is left.
pub fn pieces(full: f64, size: f64, gutter: f64) -> Vec<f64> {
    let n = ((full + gutter) / (size + gutter)).round().max(1.) as usize;
    if n == 1 {
        return vec![0.];
    }
    let piece = ((full - (n - 1) as f64 * gutter) / n as f64 / GRID_SIZE).ceil() * GRID_SIZE;
    (0..n).map(|i| i as f64 * (piece + gutter)).collect()
}

/// Slot starts along one axis for a card of `size`, over the full slots
/// from `from` to `to` (full-slot indices from `origin`).
fn starts(from: i64, to: i64, full: f64, size: f64, origin: f64, gutter: f64) -> Vec<f64> {
    let step = full + gutter;
    let offsets = pieces(full, size, gutter);
    (from..=to)
        .flat_map(|k| offsets.iter().map(move |o| origin + k as f64 * step + o))
        .collect()
}

/// How close (as a share of the ghost's own extent on that axis) a slot
/// start must be before the ghost jumps to it. Past it the ghost follows
/// the pointer on the 25 px grid: always snapping pulled a full card up to
/// half a slot, 875 px, and nothing could sit between slots (Ekin,
/// 2026-09-27: "too strong").
pub const SNAP_RANGE_RATIO: f64 = 0.1;

fn nearest(at: f64, full: f64, size: f64, origin: f64, gutter: f64) -> f64 {
    let k = ((at - origin) / (full + gutter)).floor() as i64;
    starts(k - 1, k + 1, full, size, origin, gutter)
        .into_iter()
        .filter(|s| (s - at).abs() <= size * SNAP_RANGE_RATIO)
        .min_by(|a, b| (a - at).abs().partial_cmp(&(b - at).abs()).unwrap())
        .unwrap_or_else(|| {
            let off = origin.rem_euclid(GRID_SIZE);
            ((at - off) / GRID_SIZE).round() * GRID_SIZE + off
        })
}

/// The ghost at `free` (the pointer's unsnapped rect) snapped to the
/// nearest slot of its own size. `default` is the full slot's card size,
/// `origin` the grid's anchor.
pub fn snap_ghost(free: Rect, default: Size, origin: Point, gutter: f64) -> Rect {
    Rect {
        x: nearest(free.x, default.w, free.w, origin.x, gutter),
        y: nearest(free.y, default.h, free.h, origin.y, gutter),
        w: free.w,
        h: free.h,
    }
}

/// The slots of `ghost`'s size within `reach` full slots of it, for the
/// drag to draw.
pub fn slots_near(ghost: Rect, default: Size, origin: Point, gutter: f64, reach: i64) -> Vec<Rect> {
    let kx = ((ghost.x - origin.x) / (default.w + gutter)).floor() as i64;
    let ky = ((ghost.y - origin.y) / (default.h + gutter)).floor() as i64;
    let xs = starts(kx - reach, kx + reach, default.w, ghost.w, origin.x, gutter);
    let ys = starts(ky - reach, ky + reach, default.h, ghost.h, origin.y, gutter);
    ys.iter()
        .flat_map(|&y| {
            xs.iter().map(move |&x| Rect {
                x,
                y,
                w: ghost.w,
                h: ghost.h,
            })
        })
        .collect()
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

    // Paper sizes: a half is 850 + 25 + 850 of the width and 1000 + 25 +
    // 975 of the height, as a split makes them; a full card has one slot.
    #[test]
    fn a_full_slot_divides_the_way_a_split_does() {
        assert_eq!(pieces(1725., 1725., G), [0.]);
        assert_eq!(pieces(1725., 850., G), [0., 875.]);
        assert_eq!(pieces(2000., 975., G), [0., 1025.]);
        assert_eq!(pieces(2000., 1000., G), [0., 1025.]);
        assert_eq!(pieces(1725., 400., G).len(), 4, "quarters");
    }

    // A quarter dragged near the gap a split left lands exactly in it.
    #[test]
    fn a_quarter_snaps_into_the_gap_beside_a_split_half() {
        let ghost = r(40., 1060., 850., 975.);
        assert_eq!(snap_ghost(ghost, D, O, G), r(12.5, 1037.5, 850., 975.));
        let ghost = r(900., 30., 850., 1000.);
        assert_eq!(snap_ghost(ghost, D, O, G), r(887.5, 12.5, 850., 1000.));
    }

    // Near a slot a card snaps to it; between slots it goes where the
    // pointer puts it.
    #[test]
    fn a_card_snaps_near_a_slot_and_moves_freely_between() {
        let ghost = r(1800., 2100., 1725., 2000.);
        assert_eq!(
            snap_ghost(ghost, D, O, G),
            r(12.5 + 1750., 12.5 + 2025., 1725., 2000.)
        );
        let ghost = r(1600., 150., 1725., 2000.);
        assert_eq!(snap_ghost(ghost, D, O, G), r(1762.5, 12.5, 1725., 2000.));
        // Between slots: where the pointer puts it, on the 25 px grid.
        let ghost = r(1100., 900., 1725., 2000.);
        assert_eq!(snap_ghost(ghost, D, O, G), r(1112.5, 912.5, 1725., 2000.));
    }

    // What the drag draws: slots of the ghost's own size around it, halves
    // for a half, all on the grid.
    #[test]
    fn the_drag_shows_slots_of_the_cards_own_size() {
        let half = r(900., 30., 850., 2000.);
        let slots = slots_near(half, D, O, G, 1);
        assert!(slots.iter().all(|s| s.w == 850. && s.h == 2000.));
        assert!(slots.contains(&r(887.5, 12.5, 850., 2000.)));
        assert!(
            slots.contains(&r(1762.5, 12.5, 850., 2000.)),
            "the next full slot's first half"
        );
        assert_eq!(
            slots.len(),
            6 * 3,
            "three full slots across, two halves each, three rows"
        );
    }
}
