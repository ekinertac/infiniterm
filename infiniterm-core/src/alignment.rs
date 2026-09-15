//! Alignment guides for a card being dragged or resized: which of its
//! edges and centres line up with another card's, and where to draw the
//! line. Native-port addition (2026-09-15, Ekin's ask); the Tauri app is
//! being told so the two stay in step.
//!
//! Exact matches only. Every rect is snapped to the grid before this is
//! asked, so "lines up" means equal coordinates and no tolerance is
//! needed; a magnet that pulled cards toward each other would fight the
//! grid snap. The guide spans from the nearer edge of the moving card to
//! the far edge of the card it matches, so the eye can follow it.
use crate::grid::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// A vertical line at `at` (an x), from `from` to `to` in y.
    Vertical,
    /// A horizontal line at `at` (a y), from `from` to `to` in x.
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guide {
    pub axis: Axis,
    pub at: f64,
    pub from: f64,
    pub to: f64,
}

fn xs(r: Rect) -> [f64; 3] {
    [r.x, r.x + r.w / 2., r.x + r.w]
}

fn ys(r: Rect) -> [f64; 3] {
    [r.y, r.y + r.h / 2., r.y + r.h]
}

/// The guides for `moving` against `others`, one per matching line, each
/// line once however many cards share it.
pub fn guides(moving: Rect, others: &[Rect]) -> Vec<Guide> {
    let mut out: Vec<Guide> = vec![];
    let mut add = |g: Guide| match out.iter_mut().find(|o| o.axis == g.axis && o.at == g.at) {
        Some(o) => {
            o.from = o.from.min(g.from);
            o.to = o.to.max(g.to);
        }
        None => out.push(g),
    };
    for o in others {
        for x in xs(moving) {
            if xs(*o).contains(&x) {
                add(Guide {
                    axis: Axis::Vertical,
                    at: x,
                    from: moving.y.min(o.y),
                    to: (moving.y + moving.h).max(o.y + o.h),
                });
            }
        }
        for y in ys(moving) {
            if ys(*o).contains(&y) {
                add(Guide {
                    axis: Axis::Horizontal,
                    at: y,
                    from: moving.x.min(o.x),
                    to: (moving.x + moving.w).max(o.x + o.w),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn a_shared_left_edge_is_one_vertical_guide_spanning_both() {
        let g = guides(r(100., 500., 100., 100.), &[r(100., 0., 300., 100.)]);
        assert_eq!(
            g,
            [Guide {
                axis: Axis::Vertical,
                at: 100.,
                from: 0.,
                to: 600.
            }]
        );
    }

    #[test]
    fn centres_and_far_edges_count_too() {
        let g = guides(r(0., 0., 100., 100.), &[r(300., 50., 100., 100.)]);
        // The moving centre y (50) is the other's top; the moving top (0)
        // matches nothing; the moving bottom (100) is the other's centre.
        let ys: Vec<f64> = g
            .iter()
            .filter(|g| g.axis == Axis::Horizontal)
            .map(|g| g.at)
            .collect();
        assert_eq!(ys, [50., 100.]);
        assert!(g.iter().all(|g| g.axis == Axis::Horizontal));
    }

    #[test]
    fn a_line_shared_by_several_cards_is_one_guide_over_all_of_them() {
        let g = guides(
            r(0., 1000., 100., 100.),
            &[r(0., 0., 100., 100.), r(0., 400., 100., 100.)],
        );
        let v: Vec<_> = g
            .iter()
            .filter(|g| g.axis == Axis::Vertical && g.at == 0.)
            .collect();
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].from, v[0].to), (0., 1100.));
    }

    #[test]
    fn nothing_lines_up_nothing_is_drawn() {
        assert!(guides(r(0., 0., 100., 100.), &[r(333., 777., 10., 10.)]).is_empty());
    }
}
