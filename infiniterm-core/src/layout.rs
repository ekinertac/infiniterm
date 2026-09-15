//! Card placement, using real occupied rectangles; callers supply one
//! workspace's rects. Persistence is saved_layout.rs.
//!
//! Two placements live here. `nearest_free_slot` is the one new cards use
//! since 2026-09-16: the free slot closest to the card they open from,
//! ties toward keeping the workspace's bounding box the window's shape,
//! so rows form on their own and a card never lands four columns from its
//! source at a row's end. `first_free_slot` is the reference's reading
//! order on a fixed column count (`best_cols`), kept for the group blocks.
use crate::grid::{round, Point, Rect, Size};
pub fn best_cols(
    count: usize,
    card_w: f64,
    card_h: f64,
    gutter: f64,
    viewport_w: f64,
    viewport_h: f64,
) -> usize {
    if count <= 1 || viewport_w <= 0. || viewport_h <= 0. || card_w <= 0. || card_h <= 0. {
        return 1;
    }
    let target = viewport_w / viewport_h;
    let mut best = 1;
    let mut best_err = f64::INFINITY;
    for cols in 1..=count {
        let rows = count.div_ceil(cols);
        let w = cols as f64 * card_w + (cols - 1) as f64 * gutter;
        let h = rows as f64 * card_h + (rows - 1) as f64 * gutter;
        let err = (w / h / target).ln().abs();
        if err < best_err {
            best_err = err;
            best = cols;
        }
    }
    best
}
pub fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}
/// cols is positive, as returned by best_cols.
pub fn slot_rect(index: usize, size: Size, origin: Point, gutter: f64, cols: usize) -> Rect {
    Rect {
        x: origin.x + (index % cols) as f64 * (size.w + gutter),
        y: origin.y + (index / cols) as f64 * (size.h + gutter),
        w: size.w,
        h: size.h,
    }
}
pub fn slot_index(rect: Point, size: Size, origin: Point, gutter: f64, cols: usize) -> usize {
    let col = round((rect.x - origin.x) / (size.w + gutter))
        .min((cols - 1) as f64)
        .max(0.) as usize;
    let row = round((rect.y - origin.y) / (size.h + gutter)).max(0.) as usize;
    row * cols + col
}
/// Reference defaults: limit=200 and from=0. Callers pass them explicitly.
pub fn first_free_slot(
    taken: &[Rect],
    size: Size,
    origin: Point,
    gutter: f64,
    cols: usize,
    limit: usize,
    from: usize,
) -> Rect {
    for i in from..from + limit {
        let candidate = slot_rect(i, size, origin, gutter, cols);
        if !taken.iter().any(|&t| rects_overlap(candidate, t)) {
            return candidate;
        }
    }
    slot_rect(from + taken.len(), size, origin, gutter, cols)
}

fn bounding(rects: &[Rect]) -> Option<Rect> {
    let first = rects.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x + first.w, first.y + first.h);
    for r in rects {
        x0 = x0.min(r.x);
        y0 = y0.min(r.y);
        x1 = x1.max(r.x + r.w);
        y1 = y1.max(r.y + r.h);
    }
    Some(Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    })
}

/// How far a box's shape is from `aspect`, as a log ratio (0 is exact).
fn aspect_error(b: Rect, aspect: f64) -> f64 {
    if b.w <= 0. || b.h <= 0. || aspect <= 0. {
        return 0.;
    }
    ((b.w / b.h) / aspect).ln().abs()
}

/// The free slot nearest to `from` (a card's rect; the origin slot when
/// none), on the grid of default-sized slots anchored at `origin`. Slots
/// are searched ring by ring; within a ring the closest centre wins, then
/// the one that keeps the bounding box of `taken` closest to `aspect`
/// (the window's width over height), then right, below, left, above.
/// `taken` holds every rect that is in the way, group frames included.
pub fn nearest_free_slot(
    taken: &[Rect],
    size: Size,
    origin: Point,
    gutter: f64,
    from: Option<Rect>,
    aspect: f64,
) -> Rect {
    let step_x = size.w + gutter;
    let step_y = size.h + gutter;
    let slot_at = |c: i64, r: i64| Rect {
        x: origin.x + c as f64 * step_x,
        y: origin.y + r as f64 * step_y,
        w: size.w,
        h: size.h,
    };
    // The anchor in slot coordinates and its centre in world units.
    let (ac, ar, centre) = match from {
        Some(f) => (
            round((f.x - origin.x) / step_x).max(0.) as i64,
            round((f.y - origin.y) / step_y).max(0.) as i64,
            Point {
                x: f.x + f.w / 2.,
                y: f.y + f.h / 2.,
            },
        ),
        None => (
            0,
            0,
            Point {
                x: origin.x + size.w / 2.,
                y: origin.y + size.h / 2.,
            },
        ),
    };
    let free = |r: Rect| !taken.iter().any(|&t| rects_overlap(r, t));
    // Bounded, and never left of or above the origin: the grid starts there.
    for ring in 0..200i64 {
        let mut best: Option<(Rect, (f64, f64, u8))> = None;
        for dr in -ring..=ring {
            for dc in -ring..=ring {
                if dr.abs() != ring && dc.abs() != ring {
                    continue;
                }
                let (c, r) = (ac + dc, ar + dr);
                if c < 0 || r < 0 {
                    continue;
                }
                let cand = slot_at(c, r);
                if !free(cand) {
                    continue;
                }
                let cx = cand.x + cand.w / 2. - centre.x;
                let cy = cand.y + cand.h / 2. - centre.y;
                let dist = (cx * cx + cy * cy).sqrt();
                let mut all: Vec<Rect> = taken.to_vec();
                all.push(cand);
                let shape = bounding(&all).map_or(0., |b| aspect_error(b, aspect));
                // right, below, left, above, then the rest.
                let side = match (dc.signum(), dr.signum()) {
                    (1, 0) => 0,
                    (0, 1) => 1,
                    (-1, 0) => 2,
                    (0, -1) => 3,
                    _ => 4,
                };
                let key = (round(dist), round(shape * 1000.), side);
                if best.as_ref().is_none_or(|(_, k)| key < *k) {
                    best = Some((cand, key));
                }
            }
        }
        if let Some((rect, _)) = best {
            return rect;
        }
    }
    slot_at(0, taken.len() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    const SIZE: Size = Size { w: 100., h: 100. };
    const ORIGIN: Point = Point { x: 0., y: 0. };
    fn slot(i: usize) -> Rect {
        slot_rect(i, SIZE, ORIGIN, 10., 3)
    }
    fn free(t: &[Rect]) -> Rect {
        first_free_slot(t, SIZE, ORIGIN, 10., 3, 200, 0)
    }
    #[test]
    fn overlap_excludes_edges() {
        let a = slot(0);
        assert!(!rects_overlap(a, Rect { x: 100., ..a }));
        assert!(rects_overlap(a, Rect { x: 99., ..a }));
        assert!(!rects_overlap(a, Rect { y: 100., ..a }));
    }
    #[test]
    fn slots_wrap_in_reading_order() {
        assert_eq!(
            slot(0),
            Rect {
                x: 0.,
                y: 0.,
                w: 100.,
                h: 100.
            }
        );
        assert_eq!(slot(1).x, 110.);
        assert_eq!(slot(1).y, 0.);
        assert_eq!(slot(3).x, 0.);
        assert_eq!(slot(3).y, 110.);
    }
    #[test]
    fn first_card_takes_first_slot() {
        assert_eq!(free(&[]), slot(0));
    }
    #[test]
    fn new_card_fills_hole() {
        assert_eq!(free(&[slot(1), slot(2)]), slot(0));
    }
    #[test]
    fn dragged_card_blocks_real_space() {
        assert_eq!(
            free(&[Rect {
                x: 20.,
                y: 20.,
                w: 50.,
                h: 50.
            }]),
            slot(1)
        );
    }
    #[test]
    fn full_row_pushes_to_next() {
        assert_eq!(free(&[slot(0), slot(1), slot(2)]), slot(3));
    }
    #[test]
    fn search_is_bounded() {
        let wall = Rect {
            x: -1e6,
            y: -1e6,
            w: 2e6,
            h: 2e6,
        };
        assert_eq!(
            first_free_slot(&[wall], SIZE, ORIGIN, 10., 3, 20, 0),
            slot(1)
        );
    }
    fn cols(n: usize, w: f64, h: f64) -> usize {
        best_cols(n, 950., 1050., 25., w, h)
    }
    #[test]
    fn two_portrait_cards_side_by_side() {
        assert_eq!(cols(2, 2000., 1100.), 2);
    }
    #[test]
    fn ten_cards_not_skyscraper() {
        let c = cols(10, 2000., 1100.);
        assert!(c > 2);
        assert!(10_usize.div_ceil(c) < 5);
    }
    #[test]
    fn chosen_block_closest_to_window_shape() {
        let error = |c: usize| {
            let r = 9_usize.div_ceil(c);
            let w = c as f64 * 950. + (c - 1) as f64 * 25.;
            let h = r as f64 * 1050. + (r - 1) as f64 * 25.;
            (w / h / (2000. / 1100.)).ln().abs()
        };
        let chosen = cols(9, 2000., 1100.);
        for c in 1..=9 {
            assert!(error(chosen) <= error(c));
        }
    }
    #[test]
    fn single_card_single_column() {
        assert_eq!(cols(1, 2000., 1100.), 1);
        assert_eq!(cols(0, 2000., 1100.), 1);
    }
    #[test]
    fn tall_window_prefers_fewer_columns() {
        assert!(cols(6, 3000., 1000.) > cols(6, 1000., 3000.));
    }
    #[test]
    fn unmeasured_viewport() {
        assert_eq!(cols(6, 0., 0.), 1);
    }
    #[test]
    fn slot_index_reads_nearest_slot() {
        let r = slot(4);
        assert_eq!(
            slot_index(Point { x: r.x, y: r.y }, SIZE, ORIGIN, 10., 3),
            4
        );
        assert_eq!(
            slot_index(
                Point {
                    x: r.x + 30.,
                    y: r.y - 20.
                },
                SIZE,
                ORIGIN,
                10.,
                3
            ),
            4
        );
        assert_eq!(
            slot_index(
                Point {
                    x: 10000.,
                    y: -500.
                },
                SIZE,
                ORIGIN,
                10.,
                3
            ),
            2
        );
    }
    #[test]
    fn scan_from_skips_earlier_holes() {
        assert_eq!(
            first_free_slot(&[slot(2)], SIZE, ORIGIN, 10., 3, 200, 3),
            slot(3)
        );
        assert_eq!(
            first_free_slot(
                &[slot(0), slot(1), slot(2), slot(3)],
                SIZE,
                ORIGIN,
                10.,
                3,
                200,
                3
            ),
            slot(4)
        );
    }

    // A card at the end of a row opens the next one below or beside it,
    // never at the start of the next row.
    #[test]
    fn nearest_slot_stays_beside_the_source_at_a_row_end() {
        let row: Vec<Rect> = (0..5).map(|i| slot_rect(i, SIZE, ORIGIN, 10., 5)).collect();
        let last = row[4];
        let got = nearest_free_slot(&row, SIZE, ORIGIN, 10., Some(last), 1.8);
        // Right would make the box 6 wide by 1; below keeps it nearer 1.8.
        assert_eq!(
            got,
            Rect {
                x: 440.,
                y: 110.,
                w: 100.,
                h: 100.
            }
        );
        // With only the last card left, left and right are equally near and
        // make the same shape; right wins the tie, as a tab would.
        let got = nearest_free_slot(&[last], SIZE, ORIGIN, 10., Some(last), 1.8);
        assert_eq!(
            got,
            Rect {
                x: 550.,
                y: 0.,
                w: 100.,
                h: 100.
            }
        );
    }

    #[test]
    fn rows_form_from_the_window_shape() {
        let mut taken: Vec<Rect> = vec![];
        let mut last: Option<Rect> = None;
        for _ in 0..7 {
            let r = nearest_free_slot(&taken, SIZE, ORIGIN, 10., last, 1.8);
            taken.push(r);
            last = Some(r);
        }
        let cols = taken.iter().map(|r| r.x).fold(0., f64::max) / 110. + 1.;
        let rows = taken.iter().map(|r| r.y).fold(0., f64::max) / 110. + 1.;
        // Square cards on a 1.8 screen: a second row starts by the fourth
        // card, the box stays near the screen's shape, and every card is
        // adjacent to the one before it (nearness outranks shape).
        assert_eq!(rows, 2.);
        assert!(cols <= 5., "{cols} columns");
        for pair in taken.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert!(
                (a.x - b.x).abs() <= 110. && (a.y - b.y).abs() <= 110.,
                "{a:?} -> {b:?}"
            );
        }
    }

    #[test]
    fn nothing_is_placed_left_of_or_above_the_origin() {
        let first = slot_rect(0, SIZE, ORIGIN, 10., 3);
        let got = nearest_free_slot(&[first], SIZE, ORIGIN, 10., Some(first), 1.8);
        assert!(got.x >= 0. && got.y >= 0.);
        assert_eq!(
            got,
            Rect {
                x: 110.,
                y: 0.,
                w: 100.,
                h: 100.
            }
        );
        // No source: the origin slot itself when free.
        assert_eq!(nearest_free_slot(&[], SIZE, ORIGIN, 10., None, 1.8), first);
    }
}
