//! Sliding a card along one axis into the gap beside it: the keyboard's way to
//! close the space a resize left (`card.slide.*`, 2026-10-08).
//!
//! A card made smaller by `card.size` keeps its top-left corner, so it sits a
//! long way from its neighbours, and the only ways back were the mouse and
//! 25 px nudges. Sliding moves the card in a direction until it is one gutter
//! from the next card in its path, in a straight line (a card must overlap its
//! span on the other axis to be in the way). With nothing in the path it goes
//! to the next slot of the grid for a card of its size (`slot_snap`), if that
//! one is free. Never overlaps anything, and `None` says nothing moved.
//!
//! Called from `Model::slide_active` (`model/cards_cmd.rs`); pure and tested
//! here. Related: `swap` (the neighbour exchange), `resize::fill_from_corner`.
use crate::{
    grid::{Point, Rect, Size},
    layout::rects_overlap,
    navigate::Direction,
    slot_snap::slots_near,
};

/// How far a slot may sit from the card across the other axis and still count
/// as "in line" with it, in world px.
const IN_LINE: f64 = 1.;

/// Full slots on each side of the card the search for a slot looks at.
const REACH: i64 = 3;

/// An axis of a rect: its start and its length.
type Axis = fn(&Rect) -> (f64, f64);

/// `rect` slid toward `dir`, or `None` when it cannot move. `taken` are the
/// rects it may not overlap (other cards, other groups' frames).
pub fn slide(
    rect: Rect,
    dir: Direction,
    taken: &[Rect],
    default: Size,
    origin: Point,
    gutter: f64,
) -> Option<Rect> {
    // The same rule on both axes: `forward` is the sign of the move, and the
    // functions read the axis they care about off the rect.
    let (along, across): (Axis, Axis) = match dir {
        Direction::Left | Direction::Right => (|r| (r.x, r.w), |r| (r.y, r.h)),
        Direction::Up | Direction::Down => (|r| (r.y, r.h), |r| (r.x, r.w)),
    };
    let forward = matches!(dir, Direction::Right | Direction::Down);
    let (start, len) = along(&rect);
    let (cross_start, cross_len) = across(&rect);
    let place = |at: f64| match dir {
        Direction::Left | Direction::Right => Rect { x: at, ..rect },
        Direction::Up | Direction::Down => Rect { y: at, ..rect },
    };
    // The nearest card ahead whose span across overlaps this card's.
    let ahead = taken
        .iter()
        .filter(|t| {
            let (ts, tl) = across(t);
            ts < cross_start + cross_len && ts + tl > cross_start
        })
        .filter_map(|t| {
            let (ts, tl) = along(t);
            if forward {
                (ts >= start + len).then_some(ts - gutter - len)
            } else {
                (ts + tl <= start).then_some(ts + tl + gutter)
            }
        })
        .min_by(|a, b| {
            let (da, db) = ((a - start).abs(), (b - start).abs());
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });
    if let Some(target) = ahead {
        let moves = if forward {
            target > start
        } else {
            target < start
        };
        let next = place(target);
        return (moves && !taken.iter().any(|t| rects_overlap(next, *t))).then_some(next);
    }
    // Nothing in the way: the nearest free slot of its own size in line.
    slots_near(rect, default, origin, gutter, REACH)
        .into_iter()
        .filter(|s| {
            let (ss, _) = across(s);
            (ss - cross_start).abs() <= IN_LINE
        })
        .filter(|s| {
            let (a, _) = along(s);
            if forward {
                a > start + IN_LINE
            } else {
                a < start - IN_LINE
            }
        })
        .filter(|s| !taken.iter().any(|t| rects_overlap(*s, *t)))
        .min_by(|a, b| {
            let (da, db) = ((along(a).0 - start).abs(), (along(b).0 - start).abs());
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }
    const DEFAULT: Size = Size { w: 1000., h: 800. };
    const ORIGIN: Point = Point { x: 12.5, y: 12.5 };
    const GAP: f64 = 25.;

    fn go(rect: Rect, dir: Direction, taken: &[Rect]) -> Option<Rect> {
        slide(rect, dir, taken, DEFAULT, ORIGIN, GAP)
    }

    #[test]
    fn a_card_slides_until_it_is_one_gutter_from_the_card_ahead() {
        let a = r(0., 0., 400., 300.);
        let b = r(900., 0., 400., 300.);
        assert_eq!(go(b, Direction::Left, &[a]), Some(r(425., 0., 400., 300.)));
        assert_eq!(go(a, Direction::Right, &[b]), Some(r(475., 0., 400., 300.)));
        let below = r(0., 700., 400., 300.);
        assert_eq!(
            go(below, Direction::Up, &[a]),
            Some(r(0., 325., 400., 300.))
        );
        assert_eq!(
            go(a, Direction::Down, &[below]),
            Some(r(0., 375., 400., 300.))
        );
    }

    #[test]
    fn only_a_card_in_line_is_in_the_way() {
        let b = r(900., 0., 400., 300.);
        // Another row entirely: the card at y 500 does not stop a slide at y 0.
        let other_row = r(425., 500., 400., 300.);
        let a = r(0., 0., 400., 300.);
        assert_eq!(
            go(b, Direction::Left, &[a, other_row]),
            Some(r(425., 0., 400., 300.))
        );
        // A card that overlaps the span by a sliver still blocks.
        let sliver = r(0., 299., 400., 300.);
        assert_eq!(
            go(b, Direction::Left, &[sliver]),
            Some(r(425., 0., 400., 300.))
        );
    }

    #[test]
    fn the_nearest_card_wins_and_a_card_already_against_it_stays() {
        let near = r(0., 0., 300., 300.);
        let far = r(-800., 0., 300., 300.);
        let b = r(700., 0., 300., 300.);
        assert_eq!(
            go(b, Direction::Left, &[far, near]),
            Some(r(325., 0., 300., 300.))
        );
        let snug = r(325., 0., 300., 300.);
        assert_eq!(go(snug, Direction::Left, &[far, near]), None);
    }

    #[test]
    fn with_nothing_ahead_it_goes_to_the_next_free_slot_in_line() {
        // A full-size card on the grid: the next slot to the right is one slot
        // plus a gutter on, and a card sitting there blocks it.
        let a = r(12.5, 12.5, 1000., 800.);
        assert_eq!(
            go(a, Direction::Right, &[]),
            Some(r(1037.5, 12.5, 1000., 800.))
        );
        let there = r(1037.5, 12.5, 1000., 800.);
        // The slot is taken, but then the card IS ahead: it is already snug.
        assert_eq!(go(a, Direction::Right, &[there]), None);
        // Nothing to the left of the first slot within reach but more slots.
        assert!(go(a, Direction::Left, &[]).is_some());
    }

    #[test]
    fn a_slide_never_overlaps_a_card() {
        let a = r(12.5, 12.5, 400., 300.);
        // A card straight ahead but out of line vertically does not block, and
        // the slide must still not land on one.
        let blocker = r(500., 200., 300., 300.);
        if let Some(next) = go(a, Direction::Right, &[blocker]) {
            assert!(!rects_overlap(next, blocker));
        }
    }
}
