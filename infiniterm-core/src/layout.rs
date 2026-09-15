//! Card placement in reading order, using real occupied rectangles.
//! Port of src/lib/layout.ts and its tests; callers supply one workspace's rects.
//! The bounded scan starts after the active card. Persistence is saved_layout.rs.
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
}
