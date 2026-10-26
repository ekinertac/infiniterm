//! "Resize to window" geometry (SPIKE, #585): give cards the window's aspect
//! ratio without letting them overlap, in four variants Ekin compares in the
//! build. Pure: sizes, rects and group ids in, rects out; no model.
//!
//! Two questions, answered separately so the variants can mix them:
//! 1. What size? `window_size`: the new size for ONE card, in the window's
//!    aspect, by a `SizeRule` (keep its area, keep its width, or the window-
//!    shaped default card).
//! 2. Where, with several cards? `sweep` (each card keeps its top-left, what
//!    would overlap is pushed right or down in reading order) or `scale_block`
//!    (the selection's positions are scaled about its top-left corner as one
//!    block; `sweep` then only moves what an outsider still blocks).
//!
//! Called from `model/window_fit_cmd.rs` (the commands `card.size.window.*`
//! and the rows of the `card.size` picker). Related: `resize.rs` (the
//! fractions picker), `slide.rs` (the same "never overlap" rule for one
//! card), `groups.rs` (the frame is derived from the members, so a moved
//! member can never leave its frame).
//!
//! Constraints: a group's members keep `gap / 2` of air towards other
//! groups' cards (the frame pad), so frames never touch. This is checked per
//! card, not per frame: an unselected card standing inside the bounding box
//! of an L-shaped group is not seen as in the way. shortcut: O(n^2) per card
//! pushed, fine for the few dozen cards of a selection.
use crate::grid::{snap, Rect, Size, GRID_SIZE};
use crate::resize::{MIN_CARD_H, MIN_CARD_W};

/// What size a card takes when it is shaped like the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeRule {
    /// The same area as before, so a portrait 50x80 becomes a 80x50.
    KeepArea,
    /// The same width, the height follows the aspect.
    KeepWidth,
    /// The window-shaped default card (`cards::auto_size(view, None)`): as
    /// large as a new card would be, whatever the card was.
    Fill,
}

/// How the selection is placed once it has its new sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Reflow (`reflow`): each card sits a gap right of its left neighbour
    /// and a gap below the card above it, so it is pushed OR pulled.
    Reflow,
    /// The whole selection is scaled about its top-left corner.
    Block,
}

/// One of the four commands: a size rule and a placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Variant {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub size: SizeRule,
    pub placement: Placement,
}

pub const VARIANTS: &[Variant] = &[
    Variant {
        id: "window-area",
        label: "Resize to window: same area, reflow with its neighbours",
        hint: "(a) default: pushed or pulled",
        size: SizeRule::KeepArea,
        placement: Placement::Reflow,
    },
    Variant {
        id: "window-width",
        label: "Resize to window: same width, reflow with its neighbours",
        hint: "(a) pushed or pulled",
        size: SizeRule::KeepWidth,
        placement: Placement::Reflow,
    },
    Variant {
        id: "window-fill",
        label: "Resize to window: as large as a new card, reflow",
        hint: "(a) pushed or pulled",
        size: SizeRule::Fill,
        placement: Placement::Reflow,
    },
    Variant {
        id: "window-block",
        label: "Resize to window: same area, scale the selection as one block",
        hint: "(b) layout keeps its shape",
        size: SizeRule::KeepArea,
        placement: Placement::Block,
    },
];

/// A card (or an obstacle) for the geometry: rect and group, nothing else.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    pub id: String,
    pub rect: Rect,
    pub group: Option<String>,
}

/// The new size of one card in the window's `aspect` (width over height).
/// On the grid, never under the minimum card; `fill` is the size a `Fill`
/// card takes (the caller knows the window), `aspect` must be positive.
pub fn window_size(old: Size, aspect: f64, rule: SizeRule, fill: Size) -> Size {
    let (mut w, h) = match rule {
        SizeRule::KeepArea => {
            let w = (old.w * old.h * aspect).sqrt();
            (w, w / aspect)
        }
        SizeRule::KeepWidth => (old.w, old.w / aspect),
        SizeRule::Fill => (fill.w, fill.w / aspect),
    };
    // Too small on either side: grow both, aspect kept, until it is not.
    let up = (MIN_CARD_W / w).max(MIN_CARD_H / h).max(1.);
    w *= up;
    let w = snap(w).max(MIN_CARD_W);
    // Height from the snapped width, so a row of cards agree on their shape.
    let h = snap(w / aspect).max(MIN_CARD_H);
    Size { w, h }
}

/// How much clear space two bodies need between them: nothing for cards of
/// one group (they are a gap apart by their own history) or two loose cards,
/// the frame pad between a grouped card and anything else, a whole gap
/// between two groups.
fn clearance(a: &Body, b: &Body, gap: f64) -> f64 {
    if a.group.is_some() && a.group == b.group {
        return 0.;
    }
    let pad = gap / 2.;
    [a, b].iter().filter(|x| x.group.is_some()).count() as f64 * pad
}

fn collides(a: &Body, b: &Body, gap: f64) -> bool {
    let c = clearance(a, b, gap);
    a.rect.x < b.rect.x + b.rect.w + c
        && b.rect.x < a.rect.x + a.rect.w + c
        && a.rect.y < b.rect.y + b.rect.h + c
        && b.rect.y < a.rect.y + a.rect.h + c
}

/// Reading order: rows top to bottom (cards whose tops are within a grid cell
/// share a row), left to right inside a row. Indices into `bodies`.
fn reading_order(bodies: &[Body]) -> Vec<usize> {
    let mut by_y: Vec<usize> = (0..bodies.len()).collect();
    by_y.sort_by(|&a, &b| bodies[a].rect.y.total_cmp(&bodies[b].rect.y));
    let mut order = Vec::with_capacity(bodies.len());
    let mut row: Vec<usize> = Vec::new();
    let mut row_top = f64::NEG_INFINITY;
    for i in by_y {
        if !row.is_empty() && bodies[i].rect.y - row_top > GRID_SIZE {
            row.sort_by(|&a, &b| bodies[a].rect.x.total_cmp(&bodies[b].rect.x));
            order.append(&mut row);
        }
        if row.is_empty() {
            row_top = bodies[i].rect.y;
        }
        row.push(i);
    }
    row.sort_by(|&a, &b| bodies[a].rect.x.total_cmp(&bodies[b].rect.x));
    order.extend(row);
    order
}

/// The `desired` rects with every overlap swept away: cards are taken in
/// reading order, and one that collides with a `fixed` body or a card placed
/// before it is pushed right or down, whichever is the shorter move, to one
/// gap from the body in its way, and again until it is free. `fixed` is
/// everything on the canvas that is not moving. Result is in `desired`'s
/// order. A card that collides with nothing does not move.
pub fn sweep(desired: &[Body], fixed: &[Body], gap: f64) -> Vec<Body> {
    let mut placed: Vec<Body> = Vec::with_capacity(desired.len());
    let mut out: Vec<Option<Body>> = vec![None; desired.len()];
    for i in reading_order(desired) {
        let mut card = desired[i].clone();
        // Every push passes one edge of one body, so this ends; the cap is
        // a belt for degenerate input (NaN).
        for _ in 0..(fixed.len() + placed.len() + 1) * 4 {
            let Some(hit) = fixed
                .iter()
                .chain(placed.iter())
                .find(|o| collides(&card, o, gap))
            else {
                break;
            };
            let clear = clearance(&card, hit, gap).max(gap);
            let right = hit.rect.x + hit.rect.w + clear - card.rect.x;
            let down = hit.rect.y + hit.rect.h + clear - card.rect.y;
            if right <= down {
                card.rect.x += right;
            } else {
                card.rect.y += down;
            }
        }
        placed.push(card.clone());
        out[i] = Some(card);
    }
    out.into_iter().flatten().collect()
}

/// The selection (`old`, with its new `sizes`) reflowed among the `fixed`
/// bodies, in reading order of the OLD positions. A card's neighbours are
/// read from the old geometry: a body wholly left of it whose rows overlap
/// its rows is a left neighbour, one wholly above with overlapping columns
/// an upper one. Its new x is a gap right of its left neighbours' NEW right
/// edges (the farthest wins), its new y a gap below the upper ones' NEW
/// bottoms; with no such neighbour that coordinate stays. So a row stays a
/// row and a column a column, cards are pushed away by a neighbour that
/// grew and pulled in after one that shrank. Left neighbours placed later in
/// the order are not seen; the closing `sweep` moves whatever still
/// collides (a fixed card to the right or below, a diagonal clash).
/// Result is in `old`'s order.
pub fn reflow(old: &[Body], sizes: &[Size], fixed: &[Body], gap: f64) -> Vec<Body> {
    const EPS: f64 = 0.5;
    // (old rect, new rect) of everything already placed, fixed bodies first.
    let mut done: Vec<(Rect, Rect)> = fixed.iter().map(|b| (b.rect, b.rect)).collect();
    let mut desired: Vec<Option<Body>> = vec![None; old.len()];
    for i in reading_order(old) {
        let (o, s) = (old[i].rect, sizes[i]);
        let mut x = o.x;
        let mut y = o.y;
        let (mut left, mut above) = (None::<f64>, None::<f64>);
        for (was, now) in &done {
            let rows = was.y < o.y + o.h && o.y < was.y + was.h;
            let cols = was.x < o.x + o.w && o.x < was.x + was.w;
            if rows && was.x + was.w <= o.x + EPS {
                let edge = now.x + now.w + gap;
                left = Some(left.map_or(edge, |l| l.max(edge)));
            }
            if cols && was.y + was.h <= o.y + EPS {
                let edge = now.y + now.h + gap;
                above = Some(above.map_or(edge, |a| a.max(edge)));
            }
        }
        if let Some(l) = left {
            x = l;
        }
        if let Some(a) = above {
            y = a;
        }
        let rect = Rect {
            x,
            y,
            w: s.w,
            h: s.h,
        };
        done.push((o, rect));
        desired[i] = Some(Body {
            rect,
            ..old[i].clone()
        });
    }
    let desired: Vec<Body> = desired.into_iter().flatten().collect();
    sweep(&desired, fixed, gap)
}

/// `old` cards with their new `sizes`, positions scaled about the top-left
/// corner of their bounding box by one factor per axis, so the block keeps
/// its arrangement and nothing overlaps (before any outsider is considered).
///
/// The factor on an axis is the least that leaves a gap between every pair
/// that was side by side (or stacked) on it: `(new width + gap) / pitch`. It
/// can be under 1: a portrait grid made landscape closes up its rows instead
/// of leaving holes. Pairs that were apart on both axes (a diagonal) need
/// only one of the two; the cheaper axis grows for them. With nothing side
/// by side on an axis the factor stays 1. Offsets are kept on the grid.
pub fn scale_block(old: &[Body], sizes: &[Size], gap: f64) -> Vec<Body> {
    let Some(first) = old.first() else {
        return vec![];
    };
    let (x0, y0) = old.iter().fold((first.rect.x, first.rect.y), |(x, y), b| {
        (x.min(b.rect.x), y.min(b.rect.y))
    });
    const EPS: f64 = 0.5;
    // Needed factor for a pair apart on an axis: (new extent + gap) / pitch.
    let need = |start_a: f64, start_b: f64, extent_a: f64| -> Option<f64> {
        let pitch = start_b - start_a;
        (pitch > EPS).then(|| (extent_a + gap) / pitch)
    };
    let (mut sx, mut sy) = (None::<f64>, None::<f64>);
    let mut diagonals: Vec<(f64, f64)> = Vec::new();
    for (i, a) in old.iter().enumerate() {
        for (j, b) in old.iter().enumerate() {
            if i == j {
                continue;
            }
            // `b` is right of / below `a` by the OLD geometry only.
            let right = b.rect.x >= a.rect.x + a.rect.w - EPS;
            let below = b.rect.y >= a.rect.y + a.rect.h - EPS;
            let nx = right
                .then(|| need(a.rect.x, b.rect.x, sizes[i].w))
                .flatten();
            let ny = below
                .then(|| need(a.rect.y, b.rect.y, sizes[i].h))
                .flatten();
            match (nx, ny) {
                (Some(x), Some(y)) => diagonals.push((x, y)),
                (Some(x), None) => sx = Some(sx.map_or(x, |s| s.max(x))),
                (None, Some(y)) => sy = Some(sy.map_or(y, |s| s.max(y))),
                (None, None) => {}
            }
        }
    }
    let (mut sx, mut sy) = (sx.unwrap_or(1.), sy.unwrap_or(1.));
    // Diagonal pairs are satisfied by either axis; raise the one that costs
    // less (relative growth) until every pair is. Each step satisfies at
    // least one pair for good, since factors only grow.
    for _ in 0..=diagonals.len() {
        let Some(&(x, y)) = diagonals.iter().find(|(x, y)| sx < *x && sy < *y) else {
            break;
        };
        if x / sx <= y / sy {
            sx = x;
        } else {
            sy = y;
        }
    }
    let build = |sx: f64, sy: f64| -> Vec<Body> {
        old.iter()
            .zip(sizes)
            .map(|(b, s)| Body {
                id: b.id.clone(),
                rect: Rect {
                    x: x0 + ((b.rect.x - x0) * sx / GRID_SIZE).ceil() * GRID_SIZE,
                    y: y0 + ((b.rect.y - y0) * sy / GRID_SIZE).ceil() * GRID_SIZE,
                    w: s.w,
                    h: s.h,
                },
                group: b.group.clone(),
            })
            .collect()
    };
    // The grid rounding can cost a pair its gap by less than a cell; widen
    // the block a little until nothing overlaps. Bounded: it only ever grows.
    let mut out = build(sx, sy);
    for _ in 0..200 {
        let clash =
            (0..out.len()).any(|i| (i + 1..out.len()).any(|j| collides(&out[i], &out[j], 0.)));
        if !clash {
            break;
        }
        sx *= 1.02;
        sy *= 1.02;
        out = build(sx, sy);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAP: f64 = 25.;
    const FILL: Size = Size { w: 1500., h: 900. };

    fn body(id: &str, x: f64, y: f64, w: f64, h: f64, group: Option<&str>) -> Body {
        Body {
            id: id.into(),
            rect: Rect { x, y, w, h },
            group: group.map(str::to_string),
        }
    }
    fn at<'a>(v: &'a [Body], id: &str) -> &'a Rect {
        &v.iter().find(|b| b.id == id).unwrap().rect
    }
    fn any_overlap(v: &[Body]) -> bool {
        (0..v.len()).any(|i| (i + 1..v.len()).any(|j| collides(&v[i], &v[j], 0.)))
    }
    /// 3 columns by 2 rows of portrait 1250x2000 (50x80 cells) cards, a gap apart.
    fn grid_3x2() -> Vec<Body> {
        let mut v = vec![];
        for r in 0..2 {
            for c in 0..3 {
                v.push(body(
                    &format!("c{}", r * 3 + c),
                    12.5 + c as f64 * 1275.,
                    12.5 + r as f64 * 2025.,
                    1250.,
                    2000.,
                    None,
                ));
            }
        }
        v
    }
    fn sizes_of(v: &[Body], aspect: f64, rule: SizeRule) -> Vec<Size> {
        v.iter()
            .map(|b| {
                window_size(
                    Size {
                        w: b.rect.w,
                        h: b.rect.h,
                    },
                    aspect,
                    rule,
                    FILL,
                )
            })
            .collect()
    }

    #[test]
    fn keep_area_turns_portrait_into_landscape_of_the_same_area() {
        let s = window_size(Size { w: 1250., h: 2000. }, 1.6, SizeRule::KeepArea, FILL);
        assert_eq!((s.w, s.h), (2000., 1250.));
    }

    #[test]
    fn keep_width_follows_the_aspect_and_fill_takes_the_default() {
        let s = window_size(Size { w: 1250., h: 2000. }, 1.6, SizeRule::KeepWidth, FILL);
        assert_eq!((s.w, s.h), (1250., 775.));
        let f = window_size(Size { w: 1250., h: 2000. }, 1.6, SizeRule::Fill, FILL);
        assert_eq!((f.w, f.h), (1500., 950.));
    }

    #[test]
    fn sizes_are_on_the_grid_and_never_under_the_minimum() {
        let s = window_size(Size { w: 100., h: 50. }, 3.0, SizeRule::KeepArea, FILL);
        assert!(s.w >= MIN_CARD_W && s.h >= MIN_CARD_H, "{s:?}");
        assert_eq!((s.w % GRID_SIZE, s.h % GRID_SIZE), (0., 0.));
        // A very wide window on a tiny card: grown, shape kept as far as the grid allows.
        assert!((s.w / s.h - 3.0).abs() < 0.3, "{s:?}");
    }

    #[test]
    fn a_single_card_keeps_its_top_left_and_is_not_moved() {
        let want = vec![body("a", 12.5, 12.5, 2000., 1250., None)];
        let out = sweep(&want, &[], GAP);
        assert_eq!(out, want);
    }

    #[test]
    fn sweep_pushes_a_row_apart_and_keeps_the_grid_overlap_free() {
        let old = grid_3x2();
        let sizes = sizes_of(&old, 1.6, SizeRule::KeepArea);
        let desired: Vec<Body> = old
            .iter()
            .zip(&sizes)
            .map(|(b, s)| body(&b.id, b.rect.x, b.rect.y, s.w, s.h, None))
            .collect();
        assert!(any_overlap(&desired), "the test needs overlaps to sweep");
        let out = sweep(&desired, &[], GAP);
        assert!(!any_overlap(&out));
        // The first card does not move; the next is a gap right of it.
        assert_eq!(at(&out, "c0"), at(&desired, "c0"));
        assert_eq!(at(&out, "c1").x, 12.5 + 2000. + GAP);
        // Row two keeps its y (row one got shorter, nothing collides downwards).
        assert_eq!(at(&out, "c3").y, at(&desired, "c3").y);
    }

    #[test]
    fn sweep_goes_around_an_outsider_and_keeps_a_gap() {
        let fixed = vec![body("x", 1300., 12.5, 500., 500., None)];
        let desired = vec![body("a", 12.5, 12.5, 1500., 900., None)];
        let out = sweep(&desired, &fixed, GAP);
        let r = out[0].rect;
        assert!(!rects_hit(r, fixed[0].rect));
        // Down costs 12.5+500+25-12.5 = 525, right costs 1300+500+25-12.5: down wins.
        assert_eq!((r.x, r.y), (12.5, 537.5));
    }
    fn rects_hit(a: Rect, b: Rect) -> bool {
        crate::layout::rects_overlap(a, b)
    }

    #[test]
    fn grouped_cards_keep_the_frame_pad_from_other_cards() {
        let fixed = vec![body("x", 1300., 12.5, 500., 500., None)];
        // Fits beside x by 5 units only: a loose card may stay, a grouped one needs the pad.
        let loose = vec![body("a", 12.5, 12.5, 1280., 400., None)];
        assert_eq!(sweep(&loose, &fixed, GAP)[0].rect, loose[0].rect);
        let grouped = vec![body("a", 12.5, 12.5, 1280., 400., Some("g"))];
        assert_ne!(sweep(&grouped, &fixed, GAP)[0].rect, grouped[0].rect);
    }

    #[test]
    fn cards_of_one_group_do_not_need_more_than_a_gap_between_them() {
        let desired = vec![
            body("a", 0., 0., 500., 500., Some("g")),
            body("b", 525., 0., 500., 500., Some("g")),
        ];
        let out = sweep(&desired, &[], GAP);
        assert_eq!(out, desired);
    }

    #[test]
    fn reading_order_groups_a_row_whose_tops_differ_by_a_cell() {
        let v = vec![
            body("b", 500., 0., 10., 10., None),
            body("c", 0., 100., 10., 10., None),
            body("a", 0., 12.5, 10., 10., None),
        ];
        let order: Vec<&str> = reading_order(&v)
            .iter()
            .map(|&i| v[i].id.as_str())
            .collect();
        assert_eq!(order, ["a", "b", "c"]);
    }

    fn row_of_four() -> Vec<Body> {
        (0..4)
            .map(|c| {
                body(
                    &format!("c{c}"),
                    12.5 + c as f64 * 1275.,
                    12.5,
                    1250.,
                    2000.,
                    None,
                )
            })
            .collect()
    }

    #[test]
    fn reflow_pushes_a_row_of_four_portrait_cards_apart_and_keeps_it_a_row() {
        let old = row_of_four();
        let sizes = sizes_of(&old, 1.6, SizeRule::KeepArea);
        let out = reflow(&old, &sizes, &[], GAP);
        assert!(!any_overlap(&out));
        for (c, b) in out.iter().enumerate() {
            assert_eq!(b.rect.y, 12.5, "same row");
            assert_eq!(b.rect.x, 12.5 + c as f64 * (2000. + GAP), "a gap apart");
        }
    }

    #[test]
    fn reflow_keeps_a_3x2_grid_a_grid_and_closes_the_rows_that_shrank() {
        let old = grid_3x2();
        let sizes = sizes_of(&old, 1.6, SizeRule::KeepArea);
        let out = reflow(&old, &sizes, &[], GAP);
        assert!(!any_overlap(&out));
        for c in 0..3 {
            assert_eq!(at(&out, &format!("c{c}")).y, 12.5);
            assert_eq!(
                at(&out, &format!("c{}", c + 3)).y,
                12.5 + 1250. + GAP,
                "pulled up"
            );
            assert_eq!(
                at(&out, &format!("c{c}")).x,
                at(&out, &format!("c{}", c + 3)).x
            );
        }
        assert_eq!(at(&out, "c1").x, 12.5 + 2000. + GAP);
    }

    #[test]
    fn reflow_pulls_cards_left_when_the_neighbour_shrinks() {
        let old = vec![
            body("a", 12.5, 12.5, 1250., 1000., None),
            body("b", 1287.5, 12.5, 1250., 1000., None),
        ];
        let sizes = vec![Size { w: 500., h: 1000. }, Size { w: 500., h: 1000. }];
        let out = reflow(&old, &sizes, &[], GAP);
        assert_eq!(at(&out, "b").x, 12.5 + 500. + GAP);
    }

    #[test]
    fn reflow_never_moves_the_first_card_and_a_lone_card_stays_put() {
        let old = vec![body("a", 12.5, 12.5, 1250., 2000., None)];
        let out = reflow(&old, &sizes_of(&old, 1.6, SizeRule::KeepArea), &[], GAP);
        assert_eq!((out[0].rect.x, out[0].rect.y), (12.5, 12.5));
    }

    #[test]
    fn reflow_pushes_an_unselected_card_clear_of_nothing_it_keeps_the_card_away() {
        // An unselected card to the right: the grown selected card is pushed
        // down (shorter move) rather than left over it.
        let old = vec![body("a", 12.5, 12.5, 1250., 2000., None)];
        let fixed = vec![body("x", 1287.5, 12.5, 600., 600., None)];
        let out = reflow(&old, &sizes_of(&old, 1.6, SizeRule::KeepArea), &fixed, GAP);
        assert!(!rects_hit(out[0].rect, fixed[0].rect));
        // An unselected card below, same.
        let fixed = vec![body("y", 12.5, 2037.5, 600., 600., None)];
        let out = reflow(&old, &sizes_of(&old, 1.6, SizeRule::KeepArea), &fixed, GAP);
        assert!(!rects_hit(out[0].rect, fixed[0].rect));
    }

    #[test]
    fn reflow_takes_an_unselected_left_or_upper_card_as_the_neighbour() {
        let fixed = vec![body("l", 12.5, 12.5, 500., 500., None)];
        let old = vec![body("a", 1000.5, 12.5, 1250., 2000., None)];
        let out = reflow(&old, &sizes_of(&old, 1.6, SizeRule::KeepArea), &fixed, GAP);
        assert_eq!(out[0].rect.x, 12.5 + 500. + GAP, "pulled to the left card");
    }

    #[test]
    fn reflow_in_a_group_keeps_members_a_gap_apart_and_clear_of_outsiders() {
        let old = vec![
            body("a", 12.5, 12.5, 1250., 2000., Some("g")),
            body("b", 1287.5, 12.5, 1250., 2000., Some("g")),
        ];
        let fixed = vec![body("x", 12.5, 2037.5, 600., 600., None)];
        let out = reflow(&old, &sizes_of(&old, 1.6, SizeRule::KeepArea), &fixed, GAP);
        assert!(!any_overlap(&out));
        assert_eq!(at(&out, "b").x, at(&out, "a").x + 2000. + GAP);
        assert!(out.iter().all(|b| !rects_hit(b.rect, fixed[0].rect)));
    }

    #[test]
    fn block_scales_the_grid_so_relative_positions_stay() {
        let old = grid_3x2();
        let sizes = sizes_of(&old, 1.6, SizeRule::KeepArea);
        let out = scale_block(&old, &sizes, GAP);
        assert!(!any_overlap(&out));
        // Columns stay columns and rows stay rows.
        assert_eq!(at(&out, "c0").x, at(&out, "c3").x);
        assert_eq!(at(&out, "c1").x, at(&out, "c4").x);
        assert_eq!(at(&out, "c0").y, at(&out, "c1").y);
        // The top-left card stays, the neighbours are a gap away: columns spread, rows close.
        assert_eq!(at(&out, "c0").x, 12.5);
        assert_eq!(at(&out, "c1").x, 12.5 + 2000. + GAP);
        assert_eq!(at(&out, "c3").y, 12.5 + 1250. + GAP);
    }

    #[test]
    fn block_of_one_card_does_not_move() {
        let old = vec![body("a", 12.5, 12.5, 1250., 2000., None)];
        let out = scale_block(&old, &sizes_of(&old, 1.6, SizeRule::KeepArea), GAP);
        assert_eq!((out[0].rect.x, out[0].rect.y), (12.5, 12.5));
    }

    #[test]
    fn block_clears_a_diagonal_pair() {
        let old = vec![
            body("a", 0., 0., 500., 500., None),
            body("b", 525., 525., 500., 500., None),
        ];
        let sizes = vec![Size { w: 1000., h: 600. }, Size { w: 1000., h: 600. }];
        let out = scale_block(&old, &sizes, GAP);
        assert!(!any_overlap(&out));
    }

    #[test]
    fn block_keeps_cards_of_different_sizes_apart() {
        let old = vec![
            body("a", 0., 0., 1000., 500., None),
            body("b", 1025., 0., 500., 1500., None),
            body("c", 0., 525., 1000., 975., None),
        ];
        let sizes = sizes_of(&old, 1.6, SizeRule::KeepArea);
        let out = scale_block(&old, &sizes, GAP);
        assert!(!any_overlap(&out));
    }

    #[test]
    fn block_then_sweep_moves_only_what_an_outsider_blocks() {
        let old = grid_3x2();
        let sizes = sizes_of(&old, 1.6, SizeRule::KeepArea);
        let block = scale_block(&old, &sizes, GAP);
        // A card standing where the block's second column lands.
        let fixed = vec![body("x", 2100., 100., 300., 300., None)];
        let out = sweep(&block, &fixed, GAP);
        assert!(!any_overlap(&out));
        assert!(out.iter().all(|b| !rects_hit(b.rect, fixed[0].rect)));
        assert_eq!(at(&out, "c0"), at(&block, "c0"));
    }
}
