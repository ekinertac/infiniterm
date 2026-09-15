//! Geometric splitting and soft-group reclamation, without a pane tree.
//! Port of split.ts and its tests. Close commands pass only soft-group siblings.
//! A valid split partner wins first; otherwise an exactly tiled side can grow.
use crate::{
    cards::CardRect,
    grid::{Rect, GRID_SIZE},
    resize::{MIN_CARD_H, MIN_CARD_W},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitSide {
    Right,
    Down,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Split {
    pub kept: Rect,
    pub made: Rect,
}
pub fn split_rect(rect: Rect, side: SplitSide, gutter: f64) -> Option<Split> {
    let mut kept = rect;
    let mut made = rect;
    match side {
        SplitSide::Right => {
            made.w = ((rect.w - gutter) / 2. / GRID_SIZE).floor() * GRID_SIZE;
            kept.w = rect.w - gutter - made.w;
            if made.w < MIN_CARD_W || kept.w < MIN_CARD_W {
                return None;
            }
            made.x = rect.x + kept.w + gutter;
        }
        SplitSide::Down => {
            made.h = ((rect.h - gutter) / 2. / GRID_SIZE).floor() * GRID_SIZE;
            kept.h = rect.h - gutter - made.h;
            if made.h < MIN_CARD_H || kept.h < MIN_CARD_H {
                return None;
            }
            made.y = rect.y + kept.h + gutter;
        }
    }
    Some(Split { kept, made })
}
pub fn reclaim(
    closing: Rect,
    siblings: &[CardRect],
    gutter: f64,
    partner: Option<&str>,
) -> Option<Vec<CardRect>> {
    let sides = [(true, true), (true, false), (false, true), (false, false)];
    if let Some(partner) = partner.filter(|p| !p.is_empty()) {
        let only: Vec<_> = siblings.iter().filter(|s| s.id == partner).collect();
        for (horizontal, forward) in sides {
            if let Some(grown) = absorb(closing, &only, horizontal, forward, gutter) {
                return Some(grown);
            }
        }
    }
    let all: Vec<_> = siblings.iter().collect();
    for (horizontal, forward) in sides {
        if let Some(grown) = absorb(closing, &all, horizontal, forward, gutter) {
            return Some(grown);
        }
    }
    None
}
fn absorb(
    closing: Rect,
    siblings: &[&CardRect],
    horizontal: bool,
    forward: bool,
    gutter: f64,
) -> Option<Vec<CardRect>> {
    // Normalize each side into position/length and the axis that must tile exactly.
    let axes = |r: Rect| {
        if horizontal {
            (r.x, r.w, r.y, r.h)
        } else {
            (r.y, r.h, r.x, r.w)
        }
    };
    let (pos, len, across, across_len) = axes(closing);
    let mut flush: Vec<_> = siblings
        .iter()
        .copied()
        .filter(|s| {
            let (p, l, a, al) = axes(s.rect);
            let adjacent = if forward {
                p == pos + len + gutter
            } else {
                p + l + gutter == pos
            };
            adjacent && a < across + across_len && a + al > across
        })
        .collect();
    if flush.is_empty() {
        return None;
    }
    flush.sort_by(|a, b| axes(a.rect).2.total_cmp(&axes(b.rect).2));
    let mut cursor = across;
    for s in &flush {
        let (_, _, a, al) = axes(s.rect);
        if a != cursor {
            return None;
        }
        cursor = a + al + gutter;
    }
    if cursor - gutter != across + across_len {
        return None;
    }
    Some(
        flush
            .into_iter()
            .map(|s| {
                let mut rect = s.rect;
                if horizontal {
                    if forward {
                        rect.x = closing.x;
                    }
                    rect.w += len + gutter;
                } else {
                    if forward {
                        rect.y = closing.y;
                    }
                    rect.h += len + gutter;
                }
                CardRect {
                    id: s.id.clone(),
                    rect,
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::r;
    const G: f64 = GRID_SIZE;
    const RECT: Rect = Rect {
        x: 12.5,
        y: 12.5,
        w: 40. * G,
        h: 80. * G,
    };
    fn cr(id: &str, rect: Rect) -> CardRect {
        CardRect {
            id: id.into(),
            rect,
        }
    }
    fn split(rect: Rect, side: SplitSide) -> Split {
        split_rect(rect, side, G).unwrap()
    }
    #[test]
    fn halves_width_to_right() {
        let s = split(RECT, SplitSide::Right);
        assert_eq!(s.kept, Rect { w: 20. * G, ..RECT });
        assert_eq!(
            s.made,
            Rect {
                x: 12.5 + 21. * G,
                w: 19. * G,
                ..RECT
            }
        );
    }
    #[test]
    fn halves_height_below() {
        let s = split(RECT, SplitSide::Down);
        assert_eq!(s.kept, Rect { h: 40. * G, ..RECT });
        assert_eq!(
            s.made,
            Rect {
                y: 12.5 + 41. * G,
                h: 39. * G,
                ..RECT
            }
        );
    }
    #[test]
    fn tiles_original_with_one_gutter() {
        let h = split(RECT, SplitSide::Right);
        assert_eq!(h.kept.w + G + h.made.w, RECT.w);
        let v = split(RECT, SplitSide::Down);
        assert_eq!(v.kept.h + G + v.made.h, RECT.h);
    }
    #[test]
    fn odd_cell_goes_to_existing_card() {
        let s = split(RECT, SplitSide::Right);
        assert_eq!(s.kept.w - s.made.w, G);
    }
    #[test]
    fn even_share_splits_evenly() {
        let s = split(Rect { w: 41. * G, ..RECT }, SplitSide::Right);
        assert_eq!(s.kept.w, 20. * G);
        assert_eq!(s.made.w, 20. * G);
    }
    #[test]
    fn halves_on_whole_cells() {
        let s = split(RECT, SplitSide::Down);
        assert_eq!(s.kept.h % G, 0.);
        assert_eq!(s.made.h % G, 0.);
    }
    #[test]
    fn refuses_too_small_halves() {
        assert!(split_rect(
            Rect {
                w: MIN_CARD_W * 2.,
                ..RECT
            },
            SplitSide::Right,
            G
        )
        .is_none());
        assert!(split_rect(
            Rect {
                h: MIN_CARD_H * 2.,
                ..RECT
            },
            SplitSide::Down,
            G
        )
        .is_none());
    }
    #[test]
    fn allows_exact_minimum() {
        let s = split(
            Rect {
                w: MIN_CARD_W * 2. + G,
                ..RECT
            },
            SplitSide::Right,
        );
        assert_eq!(s.kept.w, MIN_CARD_W);
        assert_eq!(s.made.w, MIN_CARD_W);
    }
    #[test]
    fn leaves_other_axis() {
        let s = split(RECT, SplitSide::Right);
        for r in [s.kept, s.made] {
            assert_eq!(r.y, RECT.y);
            assert_eq!(r.h, RECT.h);
        }
    }
    fn nested() -> [CardRect; 3] {
        [
            cr("L", r(12.5, 12.5, 20. * G, 80. * G)),
            cr("RT", r(12.5 + 21. * G, 12.5, 19. * G, 40. * G)),
            cr("RB", r(12.5 + 21. * G, 12.5 + 41. * G, 19. * G, 39. * G)),
        ]
    }
    #[test]
    fn closed_half_returns_to_original() {
        let s = split(RECT, SplitSide::Down);
        assert_eq!(
            reclaim(s.made, &[cr("k", s.kept)], G, None),
            Some(vec![cr("k", RECT)])
        );
    }
    #[test]
    fn closing_original_grows_made() {
        let s = split(RECT, SplitSide::Right);
        assert_eq!(
            reclaim(s.kept, &[cr("m", s.made)], G, None),
            Some(vec![cr("m", RECT)])
        );
    }
    #[test]
    fn widens_every_edge_tiler() {
        let [l, rt, rb] = nested();
        assert_eq!(
            reclaim(l.rect, &[rt, rb], G, None),
            Some(vec![
                cr("RT", r(12.5, 12.5, 40. * G, 40. * G)),
                cr("RB", r(12.5, 12.5 + 41. * G, 40. * G, 39. * G))
            ])
        );
    }
    #[test]
    fn stack_grows_partner_not_column() {
        let [l, rt, rb] = nested();
        let full = r(12.5 + 21. * G, 12.5, 19. * G, 80. * G);
        assert_eq!(
            reclaim(rt.rect, &[l.clone(), rb.clone()], G, None),
            Some(vec![cr("RB", full)])
        );
        assert_eq!(
            reclaim(rb.rect, &[l, rt], G, None),
            Some(vec![cr("RT", full)])
        );
    }
    #[test]
    fn incomplete_tiling_leaves_hole() {
        let [l, rt, mut rb] = nested();
        assert!(reclaim(l.rect, std::slice::from_ref(&rt), G, None).is_none());
        rb.rect.y += G;
        assert!(reclaim(l.rect, &[rt, rb], G, None).is_none());
    }
    #[test]
    fn no_siblings_or_adjacency() {
        let l = nested()[0].clone();
        assert!(reclaim(l.rect, &[], G, None).is_none());
        assert!(reclaim(
            l.rect,
            &[cr("far", r(5000., 5000., 20. * G, 80. * G))],
            G,
            None
        )
        .is_none());
    }
    #[test]
    fn requires_exact_gutter() {
        let [l, mut rt, _] = nested();
        rt.rect.x = l.rect.x + l.rect.w;
        assert!(reclaim(l.rect, &[rt], G, None).is_none());
    }
    fn four() -> (Split, [CardRect; 4]) {
        let s1 = split(Rect { w: 69. * G, ..RECT }, SplitSide::Right);
        let s2 = split(s1.made, SplitSide::Down);
        let s3 = split(s1.kept, SplitSide::Down);
        (
            s1,
            [
                cr("LT", s3.kept),
                cr("LB", s3.made),
                cr("RT", s2.kept),
                cr("RB", s2.made),
            ],
        )
    }
    #[test]
    fn two_by_two_half_returns_to_partner() {
        let (s, [lt, lb, rt, rb]) = four();
        assert_eq!(
            reclaim(
                lb.rect,
                &[lt.clone(), rt.clone(), rb.clone()],
                G,
                Some("LT")
            ),
            Some(vec![cr("LT", s.kept)])
        );
        assert_eq!(
            reclaim(rb.rect, &[lt, rt, rb.clone()], G, Some("RT")),
            Some(vec![cr("RT", s.made)])
        );
    }
    #[test]
    fn two_by_two_original_returns_to_made() {
        let (s, [lt, lb, rt, rb]) = four();
        assert_eq!(
            reclaim(lt.rect, &[lb, rt, rb], G, Some("LB")),
            Some(vec![cr("LB", s.kept)])
        );
    }
    #[test]
    fn two_by_two_no_hint_still_finds_tiler() {
        let (_, [lt, lb, rt, rb]) = four();
        assert_eq!(reclaim(lb.rect, &[lt, rt, rb], G, None).unwrap().len(), 1);
    }
    #[test]
    fn moved_partner_is_ignored() {
        let (_, [mut lt, lb, rt, rb]) = four();
        lt.rect.y -= G;
        let expected = cr(
            "RB",
            Rect {
                x: lb.rect.x,
                w: 69. * G,
                ..rb.rect
            },
        );
        assert_eq!(
            reclaim(lb.rect, &[lt, rt, rb], G, Some("LT")),
            Some(vec![expected])
        );
    }
    #[test]
    fn whole_column_widens_both_stacked_cards() {
        let (s, [_, _, rt, rb]) = four();
        let expected = vec![
            cr(
                "RT",
                Rect {
                    x: 12.5,
                    w: 69. * G,
                    ..rt.rect
                },
            ),
            cr(
                "RB",
                Rect {
                    x: 12.5,
                    w: 69. * G,
                    ..rb.rect
                },
            ),
        ];
        assert_eq!(reclaim(s.kept, &[rt, rb], G, Some("RT")), Some(expected));
    }
}
