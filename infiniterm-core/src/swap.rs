//! Whole-rectangle swaps with peers, preferring the empty slot beside the
//! card. Port of swap.ts and its tests, with one change from the reference:
//! an empty slot is taken whether or not a neighbour exists beyond it. The
//! reference refused to move with no peer in that direction, which meant the
//! card at the end of a row could not be nudged into the phantom slot next
//! to it on the same chord that swaps everywhere else. Commands supply cards
//! from one workspace. Every card and foreign group frame blocks holes; only
//! same-group peers swap.
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
    // No hole, so it is a swap, and a swap needs a peer to swap with.
    let peers: Vec<_> = cards
        .iter()
        .filter(|c| c.group_id == from.group_id)
        .cloned()
        .collect();
    let target = nearest_in_direction(&peers, from_id, dir)?;
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

/// The selection moved as one block a block-width (or height) in `dir`,
/// swapping with whatever cards sit there (#238). The block is the selected
/// cards' bounding box; the slot it moves into is that box shifted by its own
/// size plus a gutter. A card in the slot goes the other way, into the box
/// the block left, so two blocks trade places and an empty slot is a plain
/// move. Returns the new rect of every card that changes, or `None` when the
/// move would split a card that straddles the slot, run into a card or a
/// foreign group frame outside it, or when nothing is selected.
pub fn move_block(
    cards: &[PlacedCard],
    selected: &[String],
    dir: Direction,
    gutter: f64,
    occupied: &[Rect],
) -> Option<Vec<CardRect>> {
    let chosen: Vec<&PlacedCard> = cards.iter().filter(|c| selected.contains(&c.id)).collect();
    let bounds =
        crate::viewport::bounding_rect(&chosen.iter().map(|c| c.rect).collect::<Vec<_>>())?;
    let (dx, dy) = match dir {
        Direction::Left => (-(bounds.w + gutter), 0.),
        Direction::Right => (bounds.w + gutter, 0.),
        Direction::Up => (0., -(bounds.h + gutter)),
        Direction::Down => (0., bounds.h + gutter),
    };
    let slot = Rect {
        x: bounds.x + dx,
        y: bounds.y + dy,
        ..bounds
    };
    // A pixel of tolerance: rects come off a float grid.
    let inside = |r: Rect| {
        r.x >= slot.x - 1.
            && r.y >= slot.y - 1.
            && r.x + r.w <= slot.x + slot.w + 1.
            && r.y + r.h <= slot.y + slot.h + 1.
    };
    let mut displaced = vec![];
    for c in cards.iter().filter(|c| !selected.contains(&c.id)) {
        if crate::layout::rects_overlap(c.rect, slot) {
            if !inside(c.rect) {
                return None;
            }
            displaced.push(c);
        }
    }
    let shifted = |c: &PlacedCard, sign: f64| CardRect {
        id: c.id.clone(),
        rect: Rect {
            x: c.rect.x + dx * sign,
            y: c.rect.y + dy * sign,
            ..c.rect
        },
    };
    let moves: Vec<CardRect> = chosen
        .iter()
        .map(|c| shifted(c, 1.))
        .chain(displaced.iter().map(|c| shifted(c, -1.)))
        .collect();
    // Nothing may land on a card that stays or on a foreign group's frame.
    let stay: Vec<Rect> = cards
        .iter()
        .filter(|c| !moves.iter().any(|m| m.id == c.id))
        .map(|c| c.rect)
        .chain(occupied.iter().copied())
        .collect();
    let clash = moves.iter().any(|m| {
        stay.iter()
            .any(|r| crate::layout::rects_overlap(m.rect, *r))
    });
    (!clash).then_some(moves)
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
    // The end of a row moves on into the phantom slot beside it, on the
    // same chord that swaps everywhere else. The reference refused this,
    // and the card at the edge was the one that could not be moved.
    #[test]
    fn no_neighbour_moves_into_the_empty_slot() {
        let s = go(&row(), "right", Direction::Right, 100.).unwrap();
        assert_eq!(
            s.a.rect,
            r(600., 0., 100., 100.),
            "one slot right, one gutter over"
        );
        assert!(s.b.is_none(), "nothing to swap with");
        let s = go(&row(), "mid", Direction::Up, 100.).unwrap();
        assert_eq!(s.a.rect, r(200., -200., 100., 100.));
        assert!(s.b.is_none());
    }

    // With no hole AND no peer there is still nothing to do: a foreign
    // group's frame fills the slot and no same-group card lies beyond it.
    #[test]
    fn no_hole_and_no_peer_is_no_move() {
        let s = swap_with_neighbour(
            &row(),
            "right",
            Direction::Right,
            100.,
            &[r(600., 0., 100., 100.)],
        );
        assert!(s.is_none());
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
        // The slot between them is free, so this is a move into it; the
        // point is that it is never a swap with the other group's card.
        let s = go(
            &[grouped("mine", 0., "g1"), grouped("theirs", 200., "g2")],
            "mine",
            Direction::Right,
            0.,
        )
        .unwrap();
        assert!(
            s.b.is_none(),
            "a card in another group is never a swap partner"
        );
        assert_eq!(s.a.rect, r(100., 0., 100., 100.));
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
        // A card on the diagonal is not to the right or below, so it is
        // never the swap partner; with the adjacent slots free, both are
        // plain moves.
        let c = [card("here", 0., 0.), card("corner", 200., 200.)];
        assert!(target(go(&c, "here", Direction::Right, 0.)).is_none());
        assert!(target(go(&c, "here", Direction::Down, 0.)).is_none());
    }

    #[cfg(test)]
    mod move_block_tests {
        use super::*;
        use crate::test_support::{card, r};

        fn at(moves: &[CardRect], id: &str) -> Rect {
            moves.iter().find(|m| m.id == id).unwrap().rect
        }
        fn ids(v: &[&str]) -> Vec<String> {
            v.iter().map(|s| s.to_string()).collect()
        }

        // Two stacked cards move right into free space, rigid.
        #[test]
        fn a_block_moves_into_free_space() {
            let cards = [card("a", 0., 0.), card("b", 0., 1000.)];
            let moves = move_block(&cards, &ids(&["a", "b"]), Direction::Right, 25., &[]).unwrap();
            assert_eq!(moves.len(), 2);
            let (a, b) = (cards[0].rect, cards[1].rect);
            let w = a.w + 25.;
            assert_eq!((at(&moves, "a").x, at(&moves, "a").y), (a.x + w, a.y));
            assert_eq!((at(&moves, "b").x, at(&moves, "b").y), (b.x + w, b.y));
        }

        // A block of two beside another block of two: they trade places.
        #[test]
        fn two_blocks_trade_places() {
            let (a, b) = (card("a", 0., 0.), card("b", 0., 1000.));
            let w = a.rect.w + 25.;
            let (c, d) = (card("c", w, 0.), card("d", w, 1000.));
            let cards = [a.clone(), b.clone(), c.clone(), d.clone()];
            let moves = move_block(&cards, &ids(&["a", "b"]), Direction::Right, 25., &[]).unwrap();
            assert_eq!(moves.len(), 4);
            assert_eq!(at(&moves, "a"), c.rect);
            assert_eq!(at(&moves, "b"), d.rect);
            assert_eq!(at(&moves, "c"), a.rect);
            assert_eq!(at(&moves, "d"), b.rect);
        }

        // A card that straddles the slot would be split by the move.
        #[test]
        fn a_card_straddling_the_slot_blocks_the_move() {
            let a = card("a", 0., 0.);
            let straddle = r(a.rect.w + 25. + 50., 0., a.rect.w, a.rect.h);
            let cards = [
                a.clone(),
                PlacedCard {
                    id: "s".into(),
                    rect: straddle,
                    group_id: None,
                },
            ];
            assert_eq!(
                move_block(&cards, &ids(&["a"]), Direction::Right, 25., &[]),
                None
            );
        }

        // A card beyond the slot, or a foreign frame in the way, blocks it too.
        #[test]
        fn a_frame_in_the_slot_blocks_the_move() {
            let a = card("a", 0., 0.);
            let frame = r(a.rect.w + 25., 0., 50., 50.);
            assert_eq!(
                move_block(&[a], &ids(&["a"]), Direction::Right, 25., &[frame]),
                None
            );
        }

        #[test]
        fn up_and_left_walk_the_other_way_and_nothing_selected_is_none() {
            let a = card("a", 5000., 5000.);
            let moves = move_block(
                std::slice::from_ref(&a),
                &ids(&["a"]),
                Direction::Up,
                25.,
                &[],
            )
            .unwrap();
            assert_eq!(at(&moves, "a").y, a.rect.y - (a.rect.h + 25.));
            let moves = move_block(
                std::slice::from_ref(&a),
                &ids(&["a"]),
                Direction::Left,
                25.,
                &[],
            )
            .unwrap();
            assert_eq!(at(&moves, "a").x, a.rect.x - (a.rect.w + 25.));
            assert_eq!(move_block(&[a], &[], Direction::Left, 25., &[]), None);
        }
    }
}
