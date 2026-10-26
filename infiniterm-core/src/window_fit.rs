//! Reflow geometry for the size commands that act on a whole selection:
//! "Resize to window" (`card.size.window`, #585: each card keeps its width and
//! takes the height of the window's aspect) and "Default size"
//! (`card.size.default`: each card takes the default card's size), then the
//! selection is reflowed so nothing overlaps. `Target` says which. Pure: rects
//! and group ids in, rects out; no model.
//!
//! Where a card goes is decided by its neighbours, not by its old corner:
//! one gap right of the nearest card on its left, one gap below the nearest
//! card above it, so a row stays a row and a column stays a column, cards are
//! pushed away by a neighbour that grew and pulled in after one that shrank.
//! What still collides (an unselected card right of or below the selection,
//! a diagonal clash) is swept right or down. The first card of a row or
//! column keeps its coordinate.
//!
//! Called from `model/window_fit_cmd.rs` (the command and the `card.size`
//! picker row). Related: `resize.rs` (the fractions picker), `slide.rs` (the
//! same "never overlap" rule for one card), `groups.rs` (the frame is derived
//! from the members, so a moved member can never leave its frame).
//!
//! Constraints: a group's members keep `gap / 2` of air towards other
//! groups' cards (the frame pad), checked per card, not per frame: an
//! unselected card standing inside the bounding box of an L-shaped group is
//! not seen as in the way. A selection with unselected cards between its own
//! can send a card across them: the sweep only looks for a free spot, it does
//! not keep the selection together. shortcut: O(n^2) per card pushed, fine
//! for the few dozen cards of a selection.
use crate::grid::{snap, Rect, Size, GRID_SIZE};
use crate::resize::MIN_CARD_H;

/// A card (or an obstacle) for the geometry: rect and group, nothing else.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    pub id: String,
    pub rect: Rect,
    pub group: Option<String>,
}

/// The height that gives `width` the window's `aspect` (width over height):
/// on the grid, never under the minimum card height. `aspect` must be positive.
pub fn window_height(width: f64, aspect: f64) -> f64 {
    snap(width / aspect).max(MIN_CARD_H)
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

/// The selection (`old`) resized to the window's `aspect` at the same width
/// and reflowed among the `fixed` bodies, in reading order of the OLD
/// positions. A card's neighbours are read from the old geometry: a body
/// wholly left of it whose rows overlap its rows is a left neighbour, one
/// wholly above with overlapping columns an upper one. Its new x is a gap
/// right of its left neighbours' NEW right edges (the farthest wins), its new
/// y a gap below the upper ones' NEW bottoms; with no such neighbour that
/// coordinate stays. Left neighbours placed later in the order are not seen;
/// the closing `sweep` moves whatever still collides. Result is in `old`'s
/// order.
pub fn reflow(old: &[Body], fixed: &[Body], aspect: f64, gap: f64) -> Vec<Body> {
    reflow_to(old, fixed, Target::Window(aspect), gap)
}

/// What size the reflowed cards take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// The same width, the height of the window's aspect (width over height):
    /// "Resize to window".
    Window(f64),
    /// Exactly this size, the default card: "Default size".
    Size(Size),
}

impl Target {
    fn size_of(self, old: Rect) -> Size {
        match self {
            Target::Window(aspect) => Size {
                w: old.w,
                h: window_height(old.w, aspect),
            },
            Target::Size(s) => s,
        }
    }
}

/// `reflow` with the size as an input: the same placement rule for any target.
pub fn reflow_to(old: &[Body], fixed: &[Body], target: Target, gap: f64) -> Vec<Body> {
    const EPS: f64 = 0.5;
    // (old rect, new rect) of everything already placed, fixed bodies first.
    let mut done: Vec<(Rect, Rect)> = fixed.iter().map(|b| (b.rect, b.rect)).collect();
    let mut desired: Vec<Option<Body>> = vec![None; old.len()];
    for i in reading_order(old) {
        let o = old[i].rect;
        let (mut x, mut y) = (o.x, o.y);
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
        let s = target.size_of(o);
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

#[cfg(test)]
mod tests {
    use super::*;

    const GAP: f64 = 25.;
    /// Portrait 1250x2000 (50x80 cells) in a window of aspect 1.6: 1250x775.
    const ASPECT: f64 = 1.6;

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
    fn hit(a: Rect, b: Rect) -> bool {
        crate::layout::rects_overlap(a, b)
    }
    fn any_overlap(v: &[Body]) -> bool {
        (0..v.len()).any(|i| (i + 1..v.len()).any(|j| hit(v[i].rect, v[j].rect)))
    }
    fn grid(cols: usize, rows: usize) -> Vec<Body> {
        let mut v = vec![];
        for r in 0..rows {
            for c in 0..cols {
                v.push(body(
                    &format!("c{}", r * cols + c),
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

    #[test]
    fn the_height_gives_the_aspect_on_the_grid_and_never_under_the_minimum() {
        assert_eq!(window_height(1250., 1.6), 775.);
        assert_eq!(window_height(1250., 1.0), 1250.);
        assert_eq!(window_height(200., 5.), MIN_CARD_H);
        assert_eq!(window_height(1111., 1.6) % GRID_SIZE, 0.);
    }

    #[test]
    fn a_single_card_keeps_its_corner_and_its_width() {
        let old = vec![body("a", 12.5, 12.5, 1250., 2000., None)];
        let out = reflow(&old, &[], ASPECT, GAP);
        assert_eq!(
            out[0].rect,
            Rect {
                x: 12.5,
                y: 12.5,
                w: 1250.,
                h: 775.
            }
        );
    }

    #[test]
    fn a_row_of_four_stays_where_it_is_because_the_widths_do_not_change() {
        let old = grid(4, 1);
        let out = reflow(&old, &[], ASPECT, GAP);
        for (o, n) in old.iter().zip(&out) {
            assert_eq!(
                (n.rect.x, n.rect.y, n.rect.w),
                (o.rect.x, o.rect.y, o.rect.w)
            );
            assert_eq!(n.rect.h, 775.);
        }
    }

    #[test]
    fn a_3x2_grid_closes_its_rows_up_and_keeps_its_columns() {
        let out = reflow(&grid(3, 2), &[], ASPECT, GAP);
        assert!(!any_overlap(&out));
        for c in 0..3 {
            let (top, below) = (at(&out, &format!("c{c}")), at(&out, &format!("c{}", c + 3)));
            assert_eq!(top.y, 12.5);
            assert_eq!(
                below.y,
                12.5 + 775. + GAP,
                "pulled up a gap under the card above"
            );
            assert_eq!(top.x, below.x);
        }
    }

    #[test]
    fn a_wider_neighbour_pushes_and_a_narrower_one_pulls() {
        // Unselected, wider than its old spot: the selected card right of it is pushed.
        let fixed = vec![body("l", 12.5, 12.5, 1500., 775., None)];
        let old = vec![body("a", 1287.5, 12.5, 1250., 2000., None)];
        assert_eq!(
            reflow(&old, &fixed, ASPECT, GAP)[0].rect.x,
            12.5 + 1500. + GAP
        );
        // Narrower: pulled left.
        let fixed = vec![body("l", 12.5, 12.5, 500., 775., None)];
        assert_eq!(
            reflow(&old, &fixed, ASPECT, GAP)[0].rect.x,
            12.5 + 500. + GAP
        );
    }

    #[test]
    fn an_unselected_card_right_or_below_is_not_moved_and_is_cleared() {
        let old = vec![body("a", 12.5, 12.5, 1250., 2000., None)];
        // Right: a card standing where the shorter card does not reach but the old one did.
        let right = vec![body("x", 1287.5, 500., 600., 600., None)];
        let out = reflow(&old, &right, ASPECT, GAP);
        assert!(!hit(out[0].rect, right[0].rect));
        // Below: the old card overlapped it, the new one (775 tall) does not need to move.
        let below = vec![body("y", 12.5, 1000., 600., 600., None)];
        let out = reflow(&old, &below, ASPECT, GAP);
        assert!(!hit(out[0].rect, below[0].rect));
        // Overlapping the NEW rect: swept clear, a gap away.
        let in_way = vec![body("z", 12.5, 500., 600., 600., None)];
        let out = reflow(&old, &in_way, ASPECT, GAP);
        assert!(!hit(out[0].rect, in_way[0].rect));
        // Right costs 625, down 1112: the shorter move, a gap from the card.
        assert_eq!((out[0].rect.x, out[0].rect.y), (12.5 + 600. + GAP, 12.5));
    }

    #[test]
    fn an_unselected_left_or_upper_card_is_a_neighbour_too() {
        let fixed = vec![body("u", 12.5, 12.5, 1250., 500., None)];
        let old = vec![body("a", 12.5, 1000., 1250., 2000., None)];
        let out = reflow(&old, &fixed, ASPECT, GAP);
        assert_eq!(
            out[0].rect.y,
            12.5 + 500. + GAP,
            "pulled up under the card above"
        );
    }

    #[test]
    fn a_group_keeps_its_members_a_gap_apart_and_clear_of_outsiders() {
        let old = vec![
            body("a", 12.5, 12.5, 1250., 2000., Some("g")),
            body("b", 1287.5, 12.5, 1250., 2000., Some("g")),
        ];
        let fixed = vec![body("x", 12.5, 700., 600., 600., None)];
        let out = reflow(&old, &fixed, ASPECT, GAP);
        assert!(!any_overlap(&out));
        assert_eq!(at(&out, "b").x, at(&out, "a").x + 1250. + GAP);
        assert!(out.iter().all(|b| !hit(b.rect, fixed[0].rect)));
        // A loose card may sit closer than a group card may.
        let near = vec![body("n", 1262.5 + 10., 12.5, 400., 400., None)];
        let loose = vec![body("a", 12.5, 12.5, 1250., 2000., None)];
        assert_eq!(reflow(&loose, &near, ASPECT, GAP)[0].rect.y, 12.5);
        let grouped = vec![body("a", 12.5, 12.5, 1250., 2000., Some("g"))];
        assert!(
            reflow(&grouped, &near, ASPECT, GAP)[0].rect.y > 12.5,
            "pushed down"
        );
    }

    // The sample canvas (4x2 portrait cards) with #1, #2 and #6 selected, the
    // rest fixed: Ekin's case.
    #[test]
    fn selecting_one_two_and_six_of_the_sample_canvas_overlaps_nothing() {
        let all = grid(4, 2);
        let pick = |i: usize| all[i].clone();
        let old = vec![pick(0), pick(1), pick(5)];
        let fixed: Vec<Body> = [2, 3, 4, 6, 7].iter().map(|&i| pick(i)).collect();
        let out = reflow(&old, &fixed, ASPECT, GAP);
        let mut every = out.clone();
        every.extend(fixed.clone());
        assert!(!any_overlap(&every));
        assert!(out.iter().all(|b| b.rect.h == 775.));
        assert_eq!(at(&out, "c0").x, 12.5, "the first card does not move");
        // #6 was under #2 and right of #5: it sits under #2's new bottom if free.
        let c5 = at(&out, "c5");
        assert!(c5.y >= 12.5 + 775. + GAP - 0.1);
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

    const DEFAULT: Size = Size { w: 1250., h: 2000. };

    fn to_default(old: &[Body], fixed: &[Body]) -> Vec<Body> {
        reflow_to(old, fixed, Target::Size(DEFAULT), GAP)
    }

    #[test]
    fn a_row_resized_to_the_window_goes_back_to_the_default_in_place() {
        let start = grid(4, 1);
        let shaped = reflow(&start, &[], ASPECT, GAP);
        assert!(shaped.iter().all(|b| b.rect.h == 775.));
        let back = to_default(&shaped, &[]);
        assert_eq!(back, start);
    }

    #[test]
    fn a_3x2_grid_back_to_the_default_reopens_its_rows() {
        let start = grid(3, 2);
        let shaped = reflow(&start, &[], ASPECT, GAP);
        let back = to_default(&shaped, &[]);
        assert_eq!(back, start, "columns and rows return to the lattice");
    }

    #[test]
    fn the_default_size_also_shrinks_a_bigger_card_and_pulls_its_neighbour() {
        let old = vec![
            body("a", 12.5, 12.5, 3000., 3000., None),
            body("b", 3037.5, 12.5, 3000., 3000., None),
        ];
        let out = to_default(&old, &[]);
        assert_eq!(
            *at(&out, "a"),
            Rect {
                x: 12.5,
                y: 12.5,
                w: 1250.,
                h: 2000.
            }
        );
        assert_eq!(at(&out, "b").x, 12.5 + 1250. + GAP);
    }

    #[test]
    fn the_default_size_pushes_clear_of_an_unselected_neighbour() {
        let old = vec![body("a", 12.5, 12.5, 1250., 775., None)];
        // Below the short card, inside the default card's reach.
        let below = vec![body("y", 12.5, 900., 600., 600., None)];
        let out = to_default(&old, &below);
        assert!(!hit(out[0].rect, below[0].rect));
        // To the right, same.
        let right = vec![body("x", 1287.5, 1500., 600., 600., None)];
        let out = to_default(&old, &right);
        assert!(!hit(out[0].rect, right[0].rect));
        assert_eq!(
            right[0].rect.x, 1287.5,
            "the unselected card is never moved"
        );
    }

    #[test]
    fn the_default_size_keeps_a_group_apart_and_clear() {
        let old = vec![
            body("a", 12.5, 12.5, 1250., 775., Some("g")),
            body("b", 1287.5, 12.5, 1250., 775., Some("g")),
        ];
        let fixed = vec![body("x", 12.5, 1000., 600., 600., None)];
        let out = to_default(&old, &fixed);
        assert!(!any_overlap(&out));
        assert!(out.iter().all(|b| !hit(b.rect, fixed[0].rect)));
        assert_eq!(at(&out, "b").x, at(&out, "a").x + 1250. + GAP);
    }

    #[test]
    fn a_card_already_at_the_default_size_does_not_move() {
        let old = grid(2, 2);
        assert_eq!(to_default(&old, &[]), old);
    }

    #[test]
    fn sweep_leaves_cards_that_collide_with_nothing_alone() {
        let desired = vec![
            body("a", 0., 0., 500., 500., Some("g")),
            body("b", 525., 0., 500., 500., Some("g")),
        ];
        assert_eq!(sweep(&desired, &[], GAP), desired);
    }
}
