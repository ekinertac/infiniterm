//! Spatial focus, adjacent empty slots, and minimal viewport reveal.
//! Port of navigate.ts and its tests; callers supply only the active workspace.
//! Exactly diagonal cards belong to no direction; off-screen jumps land centered.
use crate::{
    cards::PlacedCard,
    grid::{Point, Rect, Size},
    layout::rects_overlap,
    viewport::Viewport,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}
fn centre(r: Rect) -> Point {
    Point {
        x: r.x + r.w / 2.,
        y: r.y + r.h / 2.,
    }
}
pub fn nearest_in_direction<'a>(
    cards: &'a [PlacedCard],
    from_id: &str,
    dir: Direction,
) -> Option<&'a PlacedCard> {
    let from = cards.iter().find(|c| c.id == from_id)?;
    let a = centre(from.rect);
    let mut best = None;
    let mut best_dist = f64::INFINITY;
    for card in cards {
        if card.id == from_id {
            continue;
        }
        let b = centre(card.rect);
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let inside = match dir {
            Direction::Right => dx > 0. && dy.abs() < dx,
            Direction::Left => dx < 0. && dy.abs() < -dx,
            Direction::Down => dy > 0. && dx.abs() < dy,
            Direction::Up => dy < 0. && dx.abs() < -dy,
        };
        if !inside {
            continue;
        }
        let dist = dx.hypot(dy);
        if dist < best_dist {
            best_dist = dist;
            best = Some(card);
        }
    }
    best
}
pub fn nearest_to(cards: &[PlacedCard], rect: Rect) -> Option<&PlacedCard> {
    let a = centre(rect);
    let mut best = None;
    let mut best_dist = f64::INFINITY;
    for c in cards {
        let b = centre(c.rect);
        let dist = (b.x - a.x).hypot(b.y - a.y);
        if dist < best_dist {
            best_dist = dist;
            best = Some(c);
        }
    }
    best
}
pub fn empty_slot_beside(
    rect: Rect,
    dir: Direction,
    gutter: f64,
    occupied: &[Rect],
    size: Size,
) -> Option<Rect> {
    let mut slot = Rect {
        x: rect.x,
        y: rect.y,
        w: size.w,
        h: size.h,
    };
    match dir {
        Direction::Right => slot.x = rect.x + rect.w + gutter,
        Direction::Left => slot.x = rect.x - size.w - gutter,
        Direction::Down => slot.y = rect.y + rect.h + gutter,
        Direction::Up => slot.y = rect.y - size.h - gutter,
    }
    if occupied.iter().any(|&o| rects_overlap(slot, o)) {
        None
    } else {
        Some(slot)
    }
}
pub const REVEAL_PADDING: f64 = 40.;
pub fn ensure_visible(rect: Rect, vp: Viewport, size: Size, padding: f64) -> Viewport {
    let mut v = vp;
    let pad = padding / vp.scale;
    let w = size.w / vp.scale;
    let h = size.h / vp.scale;
    if !rects_overlap(
        rect,
        Rect {
            x: vp.x,
            y: vp.y,
            w,
            h,
        },
    ) {
        v.x = if rect.w + pad * 2. > w {
            rect.x - pad
        } else {
            rect.x + rect.w / 2. - w / 2.
        };
        v.y = if rect.h + pad * 2. > h {
            rect.y - pad
        } else {
            rect.y + rect.h / 2. - h / 2.
        };
        return v;
    }
    if rect.x < v.x || rect.x + rect.w > v.x + w {
        v.x = if rect.w + pad * 2. > w || rect.x < v.x {
            rect.x - pad
        } else {
            rect.x + rect.w + pad - w
        };
    }
    if rect.y < v.y || rect.y + rect.h > v.y + h {
        v.y = if rect.h + pad * 2. > h || rect.y < v.y {
            rect.y - pad
        } else {
            rect.y + rect.h + pad - h
        };
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{card, r};
    fn cards() -> Vec<PlacedCard> {
        vec![
            card("c", 500., 500.),
            card("right", 800., 500.),
            card("left", 200., 500.),
            card("up", 500., 200.),
            card("down", 500., 800.),
            card("diag", 800., 800.),
        ]
    }
    fn id(c: Option<&PlacedCard>) -> Option<&str> {
        c.map(|c| c.id.as_str())
    }
    #[test]
    fn nearest_by_center_not_list_order() {
        let mut others = cards();
        others.remove(0);
        assert_eq!(
            id(nearest_to(&others, r(500., 500., 100., 100.))),
            Some("right")
        );
        others[4].rect.x = 650.;
        others[4].rect.y = 650.;
        assert_eq!(
            id(nearest_to(&others, r(500., 500., 100., 100.))),
            Some("diag")
        );
        assert!(nearest_to(&[], r(500., 500., 100., 100.)).is_none());
    }
    #[test]
    fn picks_each_direction() {
        let c = cards();
        for (d, expected) in [
            (Direction::Right, "right"),
            (Direction::Left, "left"),
            (Direction::Up, "up"),
            (Direction::Down, "down"),
        ] {
            assert_eq!(id(nearest_in_direction(&c, "c", d)), Some(expected));
        }
    }
    #[test]
    fn nearest_of_two_candidates() {
        let c = vec![
            card("c", 0., 0.),
            card("near", 300., 0.),
            card("far", 900., 0.),
        ];
        assert_eq!(
            id(nearest_in_direction(&c, "c", Direction::Right)),
            Some("near")
        );
    }
    #[test]
    fn exact_diagonal_has_no_direction() {
        let c = vec![cards()[0].clone(), cards()[5].clone()];
        assert!(nearest_in_direction(&c, "c", Direction::Right).is_none());
        assert!(nearest_in_direction(&c, "c", Direction::Down).is_none());
    }
    #[test]
    fn nothing_that_way() {
        assert!(nearest_in_direction(&cards()[..2], "c", Direction::Left).is_none());
    }
    #[test]
    fn unknown_origin() {
        assert!(nearest_in_direction(&cards(), "ghost", Direction::Right).is_none());
    }
    const VP: Viewport = Viewport {
        x: 0.,
        y: 0.,
        scale: 1.,
    };
    const SIZE: Size = Size { w: 1000., h: 800. };
    fn reveal(rect: Rect, vp: Viewport) -> Viewport {
        ensure_visible(rect, vp, SIZE, REVEAL_PADDING)
    }
    #[test]
    fn visible_card_does_not_move() {
        assert_eq!(reveal(r(200., 200., 100., 100.), VP), VP);
    }
    #[test]
    fn partly_right_scrolls_minimum() {
        assert_eq!(
            reveal(r(950., 0., 100., 100.), VP),
            Viewport { x: 90., ..VP }
        );
    }
    #[test]
    fn partly_left_scrolls_with_padding() {
        assert_eq!(reveal(r(-50., 0., 100., 100.), VP).x, -90.);
    }
    #[test]
    fn partly_bottom_moves_only_vertically() {
        assert_eq!(
            reveal(r(0., 750., 100., 100.), VP),
            Viewport { y: 90., ..VP }
        );
    }
    #[test]
    fn padding_scales_with_zoom() {
        assert_eq!(
            reveal(r(-50., 0., 100., 100.), Viewport { scale: 0.5, ..VP }).x,
            -130.
        );
    }
    #[test]
    fn entirely_offscreen_centers_both_axes() {
        assert_eq!(
            reveal(r(1200., 0., 100., 100.), VP),
            Viewport {
                x: 750.,
                y: -350.,
                ..VP
            }
        );
    }
    #[test]
    fn centering_accounts_for_zoom() {
        assert_eq!(
            reveal(r(2000., 2000., 100., 100.), Viewport { scale: 2., ..VP }),
            Viewport {
                x: 1800.,
                y: 1850.,
                scale: 2.
            }
        );
    }
    #[test]
    fn huge_card_aligns_top_left() {
        assert_eq!(
            reveal(r(5000., 5000., 4000., 3000.), VP),
            Viewport {
                x: 4960.,
                y: 4960.,
                ..VP
            }
        );
    }
    #[test]
    fn empty_slot_one_gutter_over() {
        let me = r(0., 0., 100., 200.);
        for (d, expected) in [
            (Direction::Right, r(110., 0., 100., 200.)),
            (Direction::Down, r(0., 210., 100., 200.)),
            (Direction::Left, r(-110., 0., 100., 200.)),
            (Direction::Up, r(0., -210., 100., 200.)),
        ] {
            assert_eq!(
                empty_slot_beside(me, d, 10., &[me], Size { w: me.w, h: me.h }),
                Some(expected)
            );
        }
    }
    #[test]
    fn slots_blocked_by_any_overlap() {
        let me = r(0., 0., 100., 200.);
        let n = r(110., 0., 100., 200.);
        let s = Size { w: 100., h: 200. };
        for other in [n, r(160., 100., 100., 200.)] {
            assert!(empty_slot_beside(me, Direction::Right, 10., &[me, other], s).is_none());
        }
        assert_eq!(
            empty_slot_beside(me, Direction::Right, 10., &[me, r(210., 0., 100., 200.)], s),
            Some(n)
        );
    }
    #[test]
    fn slots_use_default_size_not_split_half() {
        let half = r(0., 0., 50., 200.);
        let s = Size { w: 100., h: 200. };
        assert_eq!(
            empty_slot_beside(half, Direction::Right, 10., &[half], s),
            Some(r(60., 0., 100., 200.))
        );
        assert_eq!(
            empty_slot_beside(half, Direction::Left, 10., &[half], s),
            Some(r(-110., 0., 100., 200.))
        );
        assert!(empty_slot_beside(
            half,
            Direction::Right,
            10.,
            &[half, r(130., 0., 100., 200.)],
            s
        )
        .is_none());
    }
}
