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

/// `tidy` for UNITS: `unit[i]` says which unit card `i` belongs to (a
/// group, a split cluster, or the card alone) and `pad[u]` how much room
/// unit `u`'s frame takes around it (a group's frame, `GROUP_PAD`; 0 for
/// the rest). Each unit keeps its inside arrangement and moves as one
/// block, measured with its frame, in the order you read the units now.
/// Returns every card's new rect, in the input order.
///
/// Card by card, tidy spread a group across the block and its frame, drawn
/// around its members, ballooned over other cards; a split pair came apart
/// (2026-09-26).
/// `grid` is the placement grid's anchor and full slot size. Units are
/// packed a gutter apart, but every column and row starts on a HALF-slot
/// line of that grid (the A5 grid: 850 + 25 + 850 of 1725), so full and
/// half cards sit tight and on the grid new cards use. Neighbours are a
/// gutter apart unless two frames' `pad`s need more (with `GROUP_PAD` at
/// half the gutter they never do). Whole slots first, which left about
/// 875 px beside every half card (2026-09-27).
pub fn tidy_units(
    rects: &[Rect],
    unit: &[usize],
    pad: &[f64],
    gutter: f64,
    grid: (Point, Size),
) -> Vec<Rect> {
    let n = pad.len();
    let inner: Vec<Rect> = (0..n)
        .map(|u| {
            let members: Vec<Rect> = rects
                .iter()
                .zip(unit)
                .filter(|(_, &k)| k == u)
                .map(|(r, _)| *r)
                .collect();
            bounding_rect_of(&members).unwrap_or(Rect {
                x: 0.,
                y: 0.,
                w: 0.,
                h: 0.,
            })
        })
        .collect();
    let padded: Vec<Rect> = (0..n)
        .map(|u| Rect {
            x: inner[u].x - pad[u],
            y: inner[u].y - pad[u],
            w: inner[u].w + pad[u] * 2.,
            h: inner[u].h + pad[u] * 2.,
        })
        .collect();
    let Some(all) = bounding_rect_of(&padded) else {
        return rects.to_vec();
    };
    let (anchor, full) = grid;
    let order = crate::workspaces::reading_order(&padded);
    let cells: Vec<(usize, usize)> = (0..order.len())
        .map(|i| {
            let (c, r) = block_order(i);
            (c as usize, r as usize)
        })
        .collect();
    let cols = cells.iter().map(|&(c, _)| c + 1).max().unwrap_or(0);
    let rows = cells.iter().map(|&(_, r)| r + 1).max().unwrap_or(0);
    let (mut w, mut pw, mut h, mut ph) = (
        vec![0f64; cols],
        vec![0f64; cols],
        vec![0f64; rows],
        vec![0f64; rows],
    );
    for (&u, &(c, r)) in order.iter().zip(&cells) {
        w[c] = w[c].max(inner[u].w);
        pw[c] = pw[c].max(pad[u]);
        h[r] = h[r].max(inner[u].h);
        ph[r] = ph[r].max(pad[u]);
    }
    // The first half-slot line at or after `v`, on one axis.
    let line = |v: f64, a: f64, full: f64| -> f64 {
        let step = full + gutter;
        let half = crate::slot_snap::pieces(full, (full - gutter) / 2., gutter);
        let k = ((v - a) / step).floor() as i64;
        (k..k + 3)
            .flat_map(|k| half.iter().map(move |o| a + k as f64 * step + o))
            .find(|s| *s >= v - 0.5)
            .unwrap_or(v)
    };
    let starts = |len: &[f64], pads: &[f64], lo: f64, a: f64, full: f64| -> Vec<f64> {
        let mut out = Vec::with_capacity(len.len());
        // Neighbours a gutter apart, unless two frames' pads need more.
        let mut at = lo;
        for i in 0..len.len() {
            out.push(at);
            if i + 1 < len.len() {
                at = line(at + len[i] + gutter.max(pads[i] + pads[i + 1]), a, full);
            }
        }
        out
    };
    // Start on the half-slot line nearest the cards themselves (not their
    // frames, which pushed a whole row up and a column left).
    let cards = bounding_rect_of(&inner).unwrap_or(all);
    let nearest_line = |v: f64, a: f64, full: f64| -> f64 {
        let down = line(v - (full + gutter) / 2., a, full);
        let up = line(v, a, full);
        // Past `line`'s half-pixel tolerance, or it returns `down` again.
        let below = line(down + 1., a, full);
        [down, below, up]
            .into_iter()
            .min_by(|x, y| (x - v).abs().partial_cmp(&(y - v).abs()).unwrap())
            .unwrap_or(up)
    };
    let lo_x = nearest_line(cards.x, anchor.x, full.w);
    let lo_y = nearest_line(cards.y, anchor.y, full.h);
    let xs = starts(&w, &pw, lo_x, anchor.x, full.w);
    let ys = starts(&h, &ph, lo_y, anchor.y, full.h);
    let mut shift = vec![(0., 0.); n];
    for (&u, &(c, r)) in order.iter().zip(&cells) {
        shift[u] = (xs[c] - inner[u].x, ys[r] - inner[u].y);
    }
    rects
        .iter()
        .zip(unit)
        .map(|(r, &u)| Rect {
            x: r.x + shift[u].0,
            y: r.y + shift[u].1,
            ..*r
        })
        .collect()
}

/// Where a new card goes on a canvas that has cards on it (`next_slot`):
/// the lattice of `size` plus the gutter, still anchored at `anchor` (the
/// canvas's half cell, so slots line up with cards placed before), laid
/// over the WHOLE workspace. First the free slots inside the box the cards
/// already cover, in block order from its top-left corner; only when that
/// box is full does it grow, right and down, never up or left.
///
/// `block_slot` alone counted from the canvas origin, so a canvas whose
/// cards had been moved up and left of it (Ekin's Personal Stuff reached
/// x = -5237, y = -2762, 2026-09-26) never saw the holes there, and Cmd+T
/// kept growing the block down and right past them.
pub fn fill_slot(taken: &[Rect], size: Size, anchor: Point, gutter: f64) -> Rect {
    let Some(b) = bounding_rect_of(taken) else {
        return block_slot(taken, size, anchor, gutter);
    };
    let (sx, sy) = (size.w + gutter, size.h + gutter);
    if let Some(r) = grid_hole(taken, size, anchor, gutter) {
        return r;
    }
    // The lattice point at or before the box's top-left corner.
    let origin = Point {
        x: anchor.x + ((b.x - anchor.x) / sx).floor() * sx,
        y: anchor.y + ((b.y - anchor.y) / sy).floor() * sy,
    };
    let slot = |(c, r): (i64, i64)| Rect {
        x: origin.x + c as f64 * sx,
        y: origin.y + r as f64 * sy,
        w: size.w,
        h: size.h,
    };
    let free = |r: &Rect| !taken.iter().any(|&t| rects_overlap(*r, t));
    let inside = |r: &Rect| {
        r.x >= b.x - 0.5
            && r.y >= b.y - 0.5
            && r.x + r.w <= b.x + b.w + 0.5
            && r.y + r.h <= b.y + b.h + 0.5
    };
    let cols = ((b.x + b.w - origin.x) / sx).ceil() as usize;
    let rows = ((b.y + b.h - origin.y) / sy).ceil() as usize;
    let n = cols.max(rows) + 1;
    if let Some(r) = (0..n * n)
        .map(|i| slot(block_order(i)))
        .find(|r| inside(r) && free(r))
    {
        return r;
    }
    // Full: grow right and down from the box, not into the rows above it
    // or the columns left of it that the lattice origin may reach into.
    (0..200 * 200)
        .map(|i| slot(block_order(i)))
        .find(|r| r.y >= b.y - 0.5 && r.x >= b.x - 0.5 && free(r))
        .unwrap_or_else(|| slot((cols as i64, 0)))
}

/// The GRID's holes: the lattice positions whose row and whose column
/// already hold a card sitting on the lattice, in reading order (top row
/// first, left to right), the first one free. Then, the grid full, the
/// next slot growing it right and down. `None` when no card sits on the
/// lattice, and `fill_slot`'s box rule decides.
///
/// Why rows and columns of lined-up cards, not the whole box: on Ekin's
/// Personal Stuff a cluster of small cards sits above the main grid, off
/// its lattice, and the box rule filled the strip beside it first; the gap
/// he meant was the hole IN the grid, below #18 (2026-09-26).
fn grid_hole(taken: &[Rect], size: Size, anchor: Point, gutter: f64) -> Option<Rect> {
    let (sx, sy) = (size.w + gutter, size.h + gutter);
    let on = |v: f64, a: f64, step: f64| -> Option<i64> {
        let k = ((v - a) / step).round();
        ((v - (a + k * step)).abs() < 0.5).then_some(k as i64)
    };
    let (mut cols, mut rows): (Vec<i64>, Vec<i64>) = (vec![], vec![]);
    for t in taken {
        if let (Some(c), Some(r)) = (on(t.x, anchor.x, sx), on(t.y, anchor.y, sy)) {
            cols.push(c);
            rows.push(r);
        }
    }
    let (c0, c1) = (*cols.iter().min()?, *cols.iter().max()?);
    let (r0, r1) = (*rows.iter().min()?, *rows.iter().max()?);
    let slot = |c: i64, r: i64| Rect {
        x: anchor.x + c as f64 * sx,
        y: anchor.y + r as f64 * sy,
        w: size.w,
        h: size.h,
    };
    let free = |r: &Rect| !taken.iter().any(|&t| rects_overlap(*r, t));
    for r in r0..=r1 {
        for c in c0..=c1 {
            let s = slot(c, r);
            if free(&s) {
                return Some(s);
            }
        }
    }
    // Full: grow the grid in block order from its corner, right and down.
    (0..200 * 200)
        .map(|i| {
            let (c, r) = block_order(i);
            slot(c0 + c, r0 + r)
        })
        .find(free)
}

fn bounding_rect_of(rects: &[Rect]) -> Option<Rect> {
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

#[cfg(test)]
mod tests {
    use super::*;

    const A: Point = Point { x: 12.5, y: 12.5 };
    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    // Cards moved up and left of the canvas origin: the hole among them is
    // filled, on the same lattice as the cards around it.
    // A group of three and a loose card, scattered: the group moves as a
    // block with its inside arrangement kept, its frame's room included,
    // and nothing overlaps; the loose card takes the next place.
    #[test]
    fn tidy_moves_a_group_as_one_block() {
        let rects = [
            r(5000., 5000., 100., 100.),
            r(5125., 5000., 100., 50.),
            r(5125., 5075., 100., 25.),
            r(0., 0., 300., 200.),
        ];
        let unit = [0, 0, 0, 1];
        let pad = [50., 0.];
        let got = tidy_units(
            &rects,
            &unit,
            &pad,
            25.,
            (Point { x: 0., y: 0. }, Size { w: 300., h: 200. }),
        );
        // Inside the group nothing moved relative to its first card.
        for i in 1..3 {
            assert_eq!(got[i].x - got[0].x, rects[i].x - rects[0].x);
            assert_eq!(got[i].y - got[0].y, rects[i].y - rects[0].y);
        }
        // The loose card read first (top-left), so it stays at the corner;
        // the group's frame sits a gutter to its right, not over it.
        assert_eq!((got[3].x, got[3].y), (0., 0.));
        // The group's cards on a half-slot line past the loose card, its
        // frame clear of it.
        assert_eq!(got[0].x, 500.);
        assert!(
            got[0].x - 50. > 300. + 25. - 0.5,
            "the frame clears the loose card"
        );
        for (i, a) in got.iter().enumerate() {
            for b in &got[i + 1..] {
                assert!(!rects_overlap(*a, *b));
            }
        }
    }

    #[test]
    fn a_hole_left_and_above_the_origin_is_filled() {
        let size = Size { w: 1725., h: 2000. };
        let taken = [
            r(-3487.5, 12.5, 1725., 2000.),
            // (-1737.5, 12.5) is the hole
            r(12.5, 12.5, 1725., 2000.),
            r(-3487.5, 2037.5, 1725., 2000.),
            r(-1737.5, 2037.5, 1725., 2000.),
            r(12.5, 2037.5, 1725., 2000.),
        ];
        assert_eq!(
            fill_slot(&taken, size, A, 25.),
            r(-1737.5, 12.5, 1725., 2000.)
        );
    }

    // A full box grows right or down, never into the rows above it.
    // Ekin's Personal Stuff: a cluster of small cards above the main grid,
    // off its lattice, and a hole IN the grid below #18. The hole wins,
    // not the strip beside the cluster.
    #[test]
    fn the_hole_in_the_grid_wins_over_space_beside_an_offgrid_cluster() {
        let size = Size { w: 1725., h: 2000. };
        let taken = [
            r(-5187.5, -2762.5, 850., 2000.),
            r(-4312.5, -2762.5, 850., 1000.),
            r(-3437.5, -2762.5, 850., 1000.),
            r(-4312.5, -1737.5, 850., 975.),
            r(-3437.5, -1737.5, 850., 975.),
            r(-5237.5, 12.5, 1725., 2000.),
            r(-3487.5, 12.5, 1725., 1000.),
            r(-1737.5, 12.5, 1725., 2000.),
            r(12.5, 12.5, 1725., 2000.),
            r(1762.5, 12.5, 1725., 2000.),
            r(3512.5, 12.5, 1725., 2000.),
            r(-3487.5, 1037.5, 1725., 975.),
            r(-5237.5, 2037.5, 1725., 1975.),
            r(-3487.5, 2037.5, 1725., 1975.),
            r(-1737.5, 2037.5, 1725., 975.),
            r(12.5, 2037.5, 1725., 1000.),
            r(1762.5, 2037.5, 1725., 2000.),
            r(3512.5, 2037.5, 1725., 1000.),
            r(-1737.5, 3037.5, 1725., 975.),
            r(12.5, 3062.5, 1725., 975.),
            r(3512.5, 3062.5, 1725., 975.),
            r(12.5, 4062.5, 1725., 2000.),
        ];
        assert_eq!(
            fill_slot(&taken, size, A, 25.),
            r(-5237.5, 4062.5, 1725., 2000.),
            "below #18"
        );
    }

    #[test]
    fn a_full_canvas_grows_right_and_down_not_up() {
        let size = Size { w: 100., h: 100. };
        let taken = [
            r(-212.5, -212.5, 100., 100.),
            r(-87.5, -212.5, 100., 100.),
            r(-212.5, -87.5, 100., 100.),
            r(-87.5, -87.5, 100., 100.),
        ];
        let got = fill_slot(&taken, size, A, 25.);
        assert!(got.y >= -212.5 && got.x >= -212.5, "{got:?}");
        assert!(!taken.iter().any(|t| rects_overlap(*t, got)));
    }

    #[test]
    fn an_empty_canvas_starts_at_the_origin() {
        let size = Size { w: 100., h: 100. };
        assert_eq!(fill_slot(&[], size, A, 25.), r(12.5, 12.5, 100., 100.));
    }

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
