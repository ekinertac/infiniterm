//! Whole-rectangle swaps with peers, preferring bounded holes beside the card.
//! Port of swap.ts and its tests. Commands supply cards from one workspace.
//! Every card and foreign group frame blocks holes; only same-group peers swap.
use crate::{
    cards::{CardRect, PlacedCard},
    grid::{Rect, Size},
    navigate::{empty_slot_beside, nearest_in_direction, Direction},
};
#[derive(Clone, Debug, PartialEq)]
pub struct Swap {
    pub a: CardRect,
    pub b: Option<CardRect>,
}
pub fn swap_with_neighbour(
    cards: &[PlacedCard],
    from_id: &str,
    dir: Direction,
    gutter: f64,
    occupied: &[Rect],
) -> Option<Swap> {
    let from = cards.iter().find(|c| c.id == from_id)?;
    let peers: Vec<_> = cards
        .iter()
        .filter(|c| c.group_id == from.group_id)
        .cloned()
        .collect();
    let target = nearest_in_direction(&peers, from_id, dir)?;
    let blocked: Vec<_> = cards
        .iter()
        .filter(|c| c.id != from_id)
        .map(|c| c.rect)
        .chain(occupied.iter().copied())
        .collect();
    if let Some(hole) = empty_slot_beside(
        from.rect,
        dir,
        gutter,
        &blocked,
        Size {
            w: from.rect.w,
            h: from.rect.h,
        },
    ) {
        return Some(Swap {
            a: CardRect {
                id: from.id.clone(),
                rect: hole,
            },
            b: None,
        });
    }
    Some(Swap {
        a: CardRect {
            id: from.id.clone(),
            rect: target.rect,
        },
        b: Some(CardRect {
            id: target.id.clone(),
            rect: from.rect,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{card, r};
    fn row() -> Vec<PlacedCard> {
        vec![
            card("left", 0., 0.),
            card("mid", 200., 0.),
            card("right", 400., 0.),
        ]
    }
    fn go(cards: &[PlacedCard], id: &str, d: Direction, g: f64) -> Option<Swap> {
        swap_with_neighbour(cards, id, d, g, &[])
    }
    fn target(s: Option<Swap>) -> Option<String> {
        s.and_then(|s| s.b.map(|b| b.id))
    }
    fn grouped(id: &str, x: f64, g: &str) -> PlacedCard {
        let mut c = card(id, x, 0.);
        c.group_id = Some(g.into());
        c
    }
    #[test]
    fn exchanges_rects() {
        let s = go(&row(), "mid", Direction::Right, 100.).unwrap();
        assert_eq!(
            s.a,
            CardRect {
                id: "mid".into(),
                rect: r(400., 0., 100., 100.)
            }
        );
        assert_eq!(
            s.b,
            Some(CardRect {
                id: "right".into(),
                rect: r(200., 0., 100., 100.)
            })
        );
    }
    #[test]
    fn works_every_direction() {
        assert_eq!(
            target(go(&row(), "mid", Direction::Left, 100.)).as_deref(),
            Some("left")
        );
        let col = [card("top", 0., 0.), card("bottom", 0., 200.)];
        assert_eq!(
            target(go(&col, "top", Direction::Down, 100.)).as_deref(),
            Some("bottom")
        );
        assert_eq!(
            target(go(&col, "bottom", Direction::Up, 100.)).as_deref(),
            Some("top")
        );
    }
    #[test]
    fn exchanges_size_too() {
        let a = card("small", 0., 0.);
        let mut b = card("big", 200., 0.);
        b.rect.w = 400.;
        b.rect.h = 300.;
        let s = go(&[a, b], "small", Direction::Right, 100.).unwrap();
        assert_eq!(s.a.rect, r(200., 0., 400., 300.));
        assert_eq!(s.b.unwrap().rect, r(0., 0., 100., 100.));
    }
    #[test]
    fn no_neighbour_no_swap() {
        assert!(go(&row(), "right", Direction::Right, 100.).is_none());
        assert!(go(&row(), "mid", Direction::Up, 100.).is_none());
    }
    fn gap() -> Vec<PlacedCard> {
        vec![card("left", 0., 0.), card("right", 240., 0.)]
    }
    #[test]
    fn moves_into_hole_alone() {
        for (id, dir) in [("left", Direction::Right), ("right", Direction::Left)] {
            let s = go(&gap(), id, dir, 20.).unwrap();
            assert_eq!(
                s.a,
                CardRect {
                    id: id.into(),
                    rect: r(120., 0., 100., 100.)
                }
            );
            assert!(s.b.is_none());
        }
    }
    #[test]
    fn open_space_is_not_hole() {
        assert!(go(&gap(), "right", Direction::Right, 20.).is_none());
    }
    #[test]
    fn any_card_blocks_hole() {
        let c = [
            grouped("a", 0., "g"),
            card("loose", 120., 0.),
            grouped("b", 240., "g"),
        ];
        assert_eq!(
            target(go(&c, "a", Direction::Right, 20.)).as_deref(),
            Some("b")
        );
    }
    #[test]
    fn foreign_frame_blocks_hole() {
        assert_eq!(
            target(swap_with_neighbour(
                &gap(),
                "left",
                Direction::Right,
                20.,
                &[r(110., -10., 120., 120.)]
            ))
            .as_deref(),
            Some("right")
        );
    }
    #[test]
    fn hole_uses_own_size() {
        let mut c = gap();
        for c in &mut c {
            c.rect.h = 40.;
        }
        assert_eq!(
            go(&c, "left", Direction::Right, 20.).unwrap().a.rect,
            r(120., 0., 100., 40.)
        );
    }
    #[test]
    fn missing_origin_no_swap() {
        assert!(go(&row(), "ghost", Direction::Left, 0.).is_none());
        assert!(go(&[], "mid", Direction::Left, 0.).is_none());
    }
    #[test]
    fn never_crosses_groups() {
        assert!(go(
            &[grouped("mine", 0., "g1"), grouped("theirs", 200., "g2")],
            "mine",
            Direction::Right,
            0.
        )
        .is_none());
    }
    #[test]
    fn grouped_peers_ignore_loose_target() {
        let c = [
            grouped("a", 0., "g1"),
            card("loose", 200., 0.),
            grouped("b", 400., "g1"),
        ];
        assert_eq!(
            target(go(&c, "a", Direction::Right, 100.)).as_deref(),
            Some("b")
        );
    }
    #[test]
    fn loose_cards_are_peers() {
        let c = [
            card("a", 0., 0.),
            grouped("grouped", 200., "g1"),
            card("b", 400., 0.),
        ];
        assert_eq!(
            target(go(&c, "a", Direction::Right, 100.)).as_deref(),
            Some("b")
        );
    }
    #[test]
    fn diagonal_is_in_no_cone() {
        let c = [card("here", 0., 0.), card("corner", 200., 200.)];
        assert!(go(&c, "here", Direction::Right, 0.).is_none());
        assert!(go(&c, "here", Direction::Down, 0.).is_none());
    }
}
