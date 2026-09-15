//! Lettered default-size slots around cards, in reading order.
//! Port of slots.ts and its tests; placement UI supplies occupied cards and frames.
//! Deduplicate equal proposals, then reject overlaps before assigning keys.
use crate::{
    cards::PlacedCard,
    grid::{Rect, Size},
    layout::rects_overlap,
    navigate::{empty_slot_beside, Direction},
};
pub const SLOT_KEYS: &str = "asdfghjklqwertyuiopzxcvbnm";
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub rect: Rect,
    pub group_id: Option<String>,
    pub key: char,
}
pub fn free_slots_around(
    cards: &[PlacedCard],
    size: Size,
    gutter: f64,
    occupied: &[Rect],
) -> Vec<Slot> {
    let mut found: Vec<Slot> = vec![];
    for card in cards {
        for dir in [
            Direction::Right,
            Direction::Down,
            Direction::Left,
            Direction::Up,
        ] {
            if let Some(rect) = empty_slot_beside(card.rect, dir, gutter, occupied, size) {
                if !found.iter().any(|f| f.rect == rect) {
                    found.push(Slot {
                        rect,
                        group_id: card.group_id.clone(),
                        key: ' ',
                    });
                }
            }
        }
    }
    found.sort_by(|a, b| {
        a.rect
            .y
            .total_cmp(&b.rect.y)
            .then(a.rect.x.total_cmp(&b.rect.x))
    });
    let mut kept: Vec<Slot> = vec![];
    for f in found {
        if !kept.iter().any(|k| rects_overlap(k.rect, f.rect)) {
            kept.push(f);
        }
    }
    kept.truncate(SLOT_KEYS.len());
    for (slot, key) in kept.iter_mut().zip(SLOT_KEYS.chars()) {
        slot.key = key;
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{card, r};
    const SIZE: Size = Size { w: 100., h: 200. };
    fn at(x: f64, y: f64) -> PlacedCard {
        let mut c = card("", x, y);
        c.rect.h = 200.;
        c
    }
    fn around(c: &[PlacedCard]) -> Vec<Slot> {
        free_slots_around(c, SIZE, 10., &c.iter().map(|c| c.rect).collect::<Vec<_>>())
    }
    #[test]
    fn rings_single_card_in_reading_order() {
        let slots = around(&[at(0., 0.)]);
        assert_eq!(
            slots
                .iter()
                .map(|s| (s.rect.x, s.rect.y, s.key))
                .collect::<Vec<_>>(),
            vec![
                (0., -210., 'a'),
                (-110., 0., 's'),
                (110., 0., 'd'),
                (0., 210., 'f')
            ]
        );
    }
    #[test]
    fn hole_offered_once() {
        let s = around(&[at(0., 0.), at(220., 0.)]);
        assert_eq!(
            s.iter()
                .filter(|s| s.rect.x == 110. && s.rect.y == 0.)
                .count(),
            1
        );
    }
    #[test]
    fn frame_blocks_slot() {
        let a = at(0., 0.);
        let s = free_slots_around(
            std::slice::from_ref(&a),
            SIZE,
            10.,
            &[a.rect, r(105., -50., 300., 300.)],
        );
        assert!(!s.iter().any(|s| s.rect.x == 110. && s.rect.y == 0.));
    }
    #[test]
    fn slot_inherits_group() {
        let mut a = at(0., 0.);
        a.group_id = Some("g1".into());
        assert!(around(&[a])
            .iter()
            .all(|s| s.group_id.as_deref() == Some("g1")));
    }
    #[test]
    fn overlapping_proposals_dropped() {
        let mut a = at(0., 0.);
        a.rect.w = 45.;
        let mut b = at(55., 0.);
        b.rect.w = 45.;
        let s = around(&[a, b]);
        for i in 0..s.len() {
            for j in i + 1..s.len() {
                assert!(!rects_overlap(s[i].rect, s[j].rect));
            }
        }
    }
    #[test]
    fn caps_at_key_count() {
        let c = (0..40).map(|i| at(i as f64 * 220., 0.)).collect::<Vec<_>>();
        let s = around(&c);
        assert_eq!(s.len(), SLOT_KEYS.len());
        let keys = s
            .iter()
            .map(|s| s.key)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(keys.len(), SLOT_KEYS.len());
    }
}
