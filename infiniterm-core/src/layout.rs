//! Card placement, using real occupied rectangles; callers supply one
//! workspace's rects. Persistence is saved_layout.rs.
//!
//! Two placements live here. `block_slot` is the one new cards use since
//! 2026-09-24: the first free slot of a square block grown from the
//! top-left corner (`block_order`), whatever card is focused. Before it,
//! `nearest_free_slot` (2026-09-16 to 09-24) grew from the focused card,
//! and nobody could say where Cmd+T would land. `first_free_slot` is the
//! reference's reading order on a fixed column count (`best_cols`), kept
//! for the group blocks.
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

/// The (column, row) of the `index`th slot in block order: a square block
/// grown from the top-left corner. Ring `n` adds column `n` top to bottom,
/// stopping above the corner, then row `n` left to right, ending ON the
/// corner, so after n*n cards the block is full:
///
/// ```text
/// 1  2  5  10 17
/// 3  4  6  11 18
/// 7  8  9  12 19
/// 13 14 15 16 20
/// ```
///
/// With cards the screen's shape, a square block of them is the screen's
/// shape too, so fit-all fills the window (Ekin, 2026-09-24).
pub fn block_order(index: usize) -> (i64, i64) {
    let n = (index as f64).sqrt().floor() as usize;
    let n = if (n + 1) * (n + 1) <= index { n + 1 } else { n };
    let k = index - n * n;
    if k < n {
        (n as i64, k as i64)
    } else {
        ((k - n) as i64, n as i64)
    }
}

/// Where a new card goes: the first slot in `block_order` that is free,
/// on a lattice of `size` plus the gutter anchored at `origin`. It does
/// not depend on which card is focused, so Cmd+T lands in the same place
/// whatever you were on, and a hole a closed card left is filled first.
/// Replaced `nearest_free_slot` (2026-09-24), which grew from the focused
/// card and whose result nobody could predict.
pub fn block_slot(taken: &[Rect], size: Size, origin: Point, gutter: f64) -> Rect {
    let slot = |(c, r): (i64, i64)| Rect {
        x: origin.x + c as f64 * (size.w + gutter),
        y: origin.y + r as f64 * (size.h + gutter),
        w: size.w,
        h: size.h,
    };
    // Bounded: 200 rings is 40,000 slots, far past any canvas.
    (0..200 * 200)
        .map(|i| slot(block_order(i)))
        .find(|r| !taken.iter().any(|&t| rects_overlap(*r, t)))
        .unwrap_or_else(|| slot(block_order(0)))
}

/// Where `canvas.tidy` puts cards of these sizes, in this order: the
/// same square block new cards fill (`block_order`), starting at
/// `origin`, each card keeping its size. A column is as wide as its
/// widest card and a row as tall as its tallest, so mixed sizes never
/// overlap; a card smaller than its cell sits in the cell's top-left.
pub fn tidy(sizes: &[Size], origin: Point, gutter: f64) -> Vec<Rect> {
    let cells: Vec<(usize, usize)> = (0..sizes.len())
        .map(|i| {
            let (c, r) = block_order(i);
            (c as usize, r as usize)
        })
        .collect();
    let cols = cells.iter().map(|&(c, _)| c + 1).max().unwrap_or(0);
    let rows = cells.iter().map(|&(_, r)| r + 1).max().unwrap_or(0);
    let mut widths = vec![0f64; cols];
    let mut heights = vec![0f64; rows];
    for (s, &(c, r)) in sizes.iter().zip(&cells) {
        widths[c] = widths[c].max(s.w);
        heights[r] = heights[r].max(s.h);
    }
    let start =
        |lengths: &[f64], i: usize| -> f64 { lengths[..i].iter().map(|l| l + gutter).sum() };
    sizes
        .iter()
        .zip(&cells)
        .map(|(s, &(c, r))| Rect {
            x: origin.x + start(&widths, c),
            y: origin.y + start(&heights, r),
            w: s.w,
            h: s.h,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_packs_mixed_sizes_into_the_block_without_overlap() {
        let s = |w, h| Size { w, h };
        let sizes = [
            s(200., 100.),
            s(100., 100.),
            s(100., 300.),
            s(50., 50.),
            s(80., 80.),
        ];
        let got = tidy(&sizes, Point { x: 25., y: 25. }, 10.);
        // Column 0 is 200 wide (card 1), row 1 is 300 tall (card 3).
        let at: Vec<(f64, f64)> = got.iter().map(|r| (r.x, r.y)).collect();
        assert_eq!(
            at,
            [
                (25., 25.),
                (235., 25.),
                (25., 135.),
                (235., 135.),
                (345., 25.)
            ]
        );
        for (i, a) in got.iter().enumerate() {
            assert_eq!((a.w, a.h), (sizes[i].w, sizes[i].h), "sizes kept");
            for b in &got[i + 1..] {
                assert!(!rects_overlap(*a, *b), "{a:?} {b:?}");
            }
        }
        assert!(tidy(&[], Point { x: 0., y: 0. }, 10.).is_empty());
    }

    #[test]
    fn block_order_grows_a_square_from_the_top_left() {
        let order: Vec<(i64, i64)> = (0..20).map(block_order).collect();
        // Ekin's picture, 1-based there, (column, row) here.
        let want = [
            (0, 0),
            (1, 0),
            (0, 1),
            (1, 1),
            (2, 0),
            (2, 1),
            (0, 2),
            (1, 2),
            (2, 2),
            (3, 0),
            (3, 1),
            (3, 2),
            (0, 3),
            (1, 3),
            (2, 3),
            (3, 3),
            (4, 0),
            (4, 1),
            (4, 2),
            (4, 3),
        ];
        assert_eq!(order, want);
    }

    #[test]
    fn block_slot_fills_holes_first_and_ignores_focus() {
        let size = Size { w: 160., h: 90. };
        let o = Point { x: 0., y: 0. };
        let mut taken = vec![];
        for _ in 0..4 {
            let r = block_slot(&taken, size, o, 10.);
            taken.push(r);
        }
        let xs: Vec<(f64, f64)> = taken.iter().map(|r| (r.x, r.y)).collect();
        assert_eq!(xs, [(0., 0.), (170., 0.), (0., 100.), (170., 100.)]);
        // Close card 2: the next card goes back there, not to slot 5.
        taken.remove(1);
        assert_eq!(
            block_slot(&taken, size, o, 10.),
            Rect {
                x: 170.,
                y: 0.,
                w: 160.,
                h: 90.
            }
        );
    }

    #[test]
    fn block_slot_steps_past_a_card_of_another_size() {
        let size = Size { w: 160., h: 90. };
        let big = Rect {
            x: 0.,
            y: 0.,
            w: 400.,
            h: 300.,
        };
        let r = block_slot(&[big], size, Point { x: 0., y: 0. }, 10.);
        assert!(!rects_overlap(r, big), "{r:?}");
    }
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
}
