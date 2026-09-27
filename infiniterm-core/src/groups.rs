//! Derived group frames and the spatial unit ring. A frame says which
//! cards belong together and carries NO agent state: a group holds several
//! sessions, and one colour cannot say which of them wants you.
//! Port of groups.ts and its tests; UI supplies cards and group frames per workspace.
//! Membership belongs to cards. Loose cards form one trailing navigation stop.
use crate::{
    cards::{CardRect, PlacedCard},
    grid::{Rect, Size, GRID_SIZE},
    viewport::bounding_rect,
};
/// Room a group's frame keeps around its cards: half the gutter, so two
/// frames, or a frame and a card, fit in one gutter and groups sit on the
/// placement grid like any card. It was two grid cells (50) until
/// 2026-09-27, twice the gutter, and every tidy and every Cmd+T beside a
/// group had to skip half a slot to clear the frame.
pub const GROUP_PAD: f64 = GRID_SIZE / 2.;
pub const GROUP_RESERVE_COLS: usize = 2;
pub const GROUP_RESERVE_ROWS: usize = 2;
pub const UNGROUPED: &str = "ungrouped";
pub fn group_bounds(rects: &[Rect], pad: f64) -> Option<Rect> {
    let b = bounding_rect(rects)?;
    Some(Rect {
        x: b.x - pad,
        y: b.y - pad,
        w: b.w + pad * 2.,
        h: b.h + pad * 2.,
    })
}
pub fn group_slot_size(card: Size, gutter: f64, pad: f64) -> Size {
    Size {
        w: GROUP_RESERVE_COLS as f64 * card.w + (GROUP_RESERVE_COLS - 1) as f64 * gutter + pad * 2.,
        h: GROUP_RESERVE_ROWS as f64 * card.h + (GROUP_RESERVE_ROWS - 1) as f64 * gutter + pad * 2.,
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Group,
    Ungrouped,
}
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasUnit {
    pub kind: UnitKind,
    pub id: String,
    pub rect: Rect,
    pub card_ids: Vec<String>,
}
pub fn canvas_units(cards: &[PlacedCard], frames: &[CardRect]) -> Vec<CanvasUnit> {
    let order = |a: Rect, b: Rect| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x));
    let mut frames: Vec<_> = frames.iter().collect();
    frames.sort_by(|a, b| order(a.rect, b.rect));
    let mut cards: Vec<_> = cards.iter().collect();
    cards.sort_by(|a, b| order(a.rect, b.rect));
    let mut units: Vec<_> = frames
        .into_iter()
        .map(|f| CanvasUnit {
            kind: UnitKind::Group,
            id: f.id.clone(),
            rect: f.rect,
            card_ids: cards
                .iter()
                .filter(|c| c.group_id.as_deref() == Some(&f.id))
                .map(|c| c.id.clone())
                .collect(),
        })
        .collect();
    let loose: Vec<_> = cards.into_iter().filter(|c| c.group_id.is_none()).collect();
    if let Some(rect) = group_bounds(&loose.iter().map(|c| c.rect).collect::<Vec<_>>(), GROUP_PAD) {
        units.push(CanvasUnit {
            kind: UnitKind::Ungrouped,
            id: UNGROUPED.into(),
            rect,
            card_ids: loose.iter().map(|c| c.id.clone()).collect(),
        });
    }
    units
}
pub fn step_ring<'a, T: PartialEq>(ring: &'a [T], current: &T, step: isize) -> Option<&'a T> {
    if ring.is_empty() {
        return None;
    }
    let Some(i) = ring.iter().position(|v| v == current) else {
        return if step > 0 { ring.first() } else { ring.last() };
    };
    let n = ring.len() as isize;
    ring.get((i as isize + step % n).rem_euclid(n) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{card, r};
    fn bounds(rects: &[Rect]) -> Option<Rect> {
        group_bounds(rects, GROUP_PAD)
    }
    #[test]
    fn empty_group_has_no_bounds() {
        assert_eq!(bounds(&[]), std::option::Option::None);
    }
    #[test]
    fn pads_single_card() {
        assert_eq!(
            group_bounds(&[r(100., 200., 300., 400.)], 10.),
            Some(r(90., 190., 320., 420.))
        );
    }
    #[test]
    fn spans_members() {
        assert_eq!(
            group_bounds(&[r(0., 0., 100., 100.), r(300., 50., 100., 100.)], 0.),
            Some(r(0., 0., 400., 150.))
        );
    }
    // Two frames, or a frame and a card, fit in one gutter.
    #[test]
    fn a_frame_fits_in_half_a_gutter() {
        assert_eq!(GROUP_PAD * 2., crate::cards::GUTTER);
        assert_eq!(
            bounds(&[r(0., 0., 100., 100.)]),
            Some(r(
                -GROUP_PAD,
                -GROUP_PAD,
                100. + GROUP_PAD * 2.,
                100. + GROUP_PAD * 2.
            ))
        );
    }
    #[test]
    fn empty_ring_no_stop() {
        assert!(step_ring::<&str>(&[], &"a", 1).is_none());
    }
    #[test]
    fn ring_forward_and_back() {
        assert_eq!(step_ring(&["a", "b", "c"], &"b", 1), Some(&"c"));
        assert_eq!(step_ring(&["a", "b", "c"], &"b", -1), Some(&"a"));
    }
    #[test]
    fn ring_wraps_both_ends() {
        assert_eq!(step_ring(&["a", "b", "c"], &"c", 1), Some(&"a"));
        assert_eq!(step_ring(&["a", "b", "c"], &"a", -1), Some(&"c"));
    }
    #[test]
    fn unknown_ring_current_enters_near_end() {
        assert_eq!(step_ring(&["a", "b"], &"zz", 1), Some(&"a"));
        assert_eq!(step_ring(&["a", "b"], &"zz", -1), Some(&"b"));
    }
    #[test]
    fn null_is_real_ring_stop() {
        let ring = [Option::None, Some("g1"), Some("g2")];
        assert_eq!(step_ring(&ring, &Option::None, 1), Some(&Some("g1")));
        assert_eq!(step_ring(&ring, &Some("g2"), 1), Some(&Option::None));
        assert_eq!(step_ring(&ring, &Some("g1"), -1), Some(&Option::None));
    }
    #[test]
    fn reserves_two_by_two_plus_padding() {
        assert_eq!(
            group_slot_size(Size { w: 100., h: 200. }, 10., 5.),
            Size { w: 220., h: 420. }
        );
    }
    fn grouped(id: &str, x: f64, y: f64, group: &str) -> PlacedCard {
        let mut c = card(id, x, y);
        c.group_id = Some(group.into());
        c
    }
    fn frame(id: &str, rect: Rect) -> CardRect {
        CardRect {
            id: id.into(),
            rect,
        }
    }
    #[test]
    fn group_and_one_loose_stop() {
        let u = canvas_units(
            &[
                card("c1", 0., 0.),
                card("c2", 0., 200.),
                grouped("g1a", 500., 0., "g1"),
            ],
            &[frame("g1", r(450., -50., 200., 200.))],
        );
        assert_eq!(
            u.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(),
            vec!["g1", UNGROUPED]
        );
    }
    #[test]
    fn loose_card_count_does_not_grow_ring() {
        let c = (0..5)
            .map(|i| card(&format!("c{i}"), i as f64 * 200., 0.))
            .collect::<Vec<_>>();
        assert_eq!(canvas_units(&c, &[]).len(), 1);
    }
    #[test]
    fn members_in_reading_order() {
        let u = canvas_units(
            &[
                card("bottom", 0., 900.),
                card("right", 600., 0.),
                card("left", 0., 0.),
            ],
            &[],
        );
        assert_eq!(u[0].card_ids, vec!["left", "right", "bottom"]);
    }
    #[test]
    fn loose_cards_share_frame() {
        let u = canvas_units(&[card("a", 0., 0.), card("b", 400., 300.)], &[]);
        assert_eq!(
            u[0].rect,
            r(
                -GROUP_PAD,
                -GROUP_PAD,
                500. + GROUP_PAD * 2.,
                400. + GROUP_PAD * 2.
            )
        );
    }
    #[test]
    fn no_loose_stop_if_all_grouped() {
        let u = canvas_units(
            &[grouped("c1", 0., 0., "g1")],
            &[frame("g1", r(0., 0., 200., 200.))],
        );
        assert_eq!(
            u.iter().map(|u| u.kind).collect::<Vec<_>>(),
            vec![UnitKind::Group]
        );
    }
    #[test]
    fn groups_in_reading_order_loose_last() {
        let u = canvas_units(
            &[
                card("loose", 0., 0.),
                grouped("a", 0., 900., "far"),
                grouped("b", 600., 0., "near"),
            ],
            &[
                frame("far", r(0., 850., 200., 200.)),
                frame("near", r(550., -50., 200., 200.)),
            ],
        );
        assert_eq!(
            u.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(),
            vec!["near", "far", UNGROUPED]
        );
    }
}
