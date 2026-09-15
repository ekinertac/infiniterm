//! The viewport: where the canvas is looked at from, and the zoom maths.
//!
//! Ported from the Tauri app's `zoomActions.ts` and `zoomAnimation.ts`
//! with their tests, rules included:
//!
//! - A fit never magnifies past actual size (`MAX_FIT_SCALE`): "look at
//!   this one" must not turn a small card into a wall of 40px glyphs.
//!   Zooming in further is a deliberate act, on the zoom keys.
//! - Actual size keeps the point you were looking at centred; it never
//!   resets to the origin.
//! - An animated fit interpolates the CENTRE, not the corner: the corner is
//!   a different part of the world at each scale, and interpolating it
//!   slides the view sideways while it zooms. Scale ramps geometrically so
//!   every step is the same ratio.
//!
//! Pure core module for the future infiniterm-ui canvas. No UI dependencies.
//! Related reference: spikes/canvas/src/viewport.rs, retained until the canvas
//! itself is replaced. All 20 original TS test cases are ported below.
//! Geometry uses f64 to retain TypeScript number precision until UI conversion.
//! Everything that moves the viewport goes through here.

pub use crate::grid::{Point, Rect, Size};

/// Top-left corner in world units plus the scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
}

pub const FIT_PADDING: f64 = 48.0;
pub const MIN_SCALE: f64 = 0.05;
pub const MAX_SCALE: f64 = 4.0;
pub const MAX_FIT_SCALE: f64 = 1.0;
pub const PAN_DURATION_MS: f64 = 160.0;
pub const FIT_DURATION_MS: f64 = 240.0;
pub const ZOOM_DURATION_MS: f64 = 130.0;

pub fn bounding_rect(rects: &[Rect]) -> Option<Rect> {
    let first = rects.first()?;
    let (mut min_x, mut min_y, mut max_x, mut max_y) =
        (first.x, first.y, first.x + first.w, first.y + first.h);
    for r in rects {
        min_x = min_x.min(r.x);
        min_y = min_y.min(r.y);
        max_x = max_x.max(r.x + r.w);
        max_y = max_y.max(r.y + r.h);
    }
    Some(Rect {
        x: min_x,
        y: min_y,
        w: max_x - min_x,
        h: max_y - min_y,
    })
}

/// Largest scale (capped) at which `rect` plus padding fits, centred.
pub fn fit_rect(rect: Rect, size: Size) -> Viewport {
    fit_rect_with_padding(rect, size, FIT_PADDING)
}

/// Explicit padding supports callers that fit content with different chrome.
pub fn fit_rect_with_padding(rect: Rect, size: Size, padding: f64) -> Viewport {
    let usable_w = (size.w - padding * 2.0).max(1.0);
    let usable_h = (size.h - padding * 2.0).max(1.0);
    let scale = (usable_w / rect.w)
        .min(usable_h / rect.h)
        .clamp(MIN_SCALE, MAX_FIT_SCALE);
    centre_on(
        Point {
            x: rect.x + rect.w / 2.0,
            y: rect.y + rect.h / 2.0,
        },
        scale,
        size,
    )
}

/// Viewport with `point` (world) in the middle of the content area.
pub fn centre_on(point: Point, scale: f64, size: Size) -> Viewport {
    let clamped = scale.clamp(MIN_SCALE, MAX_SCALE);
    Viewport {
        scale: clamped,
        x: point.x - size.w / clamped / 2.0,
        y: point.y - size.h / clamped / 2.0,
    }
}

/// The world point at the centre of the content area.
pub fn viewport_centre(vp: Viewport, size: Size) -> Point {
    Point {
        x: vp.x + size.w / vp.scale / 2.0,
        y: vp.y + size.h / vp.scale / 2.0,
    }
}

pub fn ease_out_cubic(t: f64) -> f64 {
    let c = t.clamp(0.0, 1.0);
    1.0 - (1.0 - c).powi(3)
}

/// Geometric interpolation: each step of the ramp is the same zoom ratio.
pub fn scale_at(from: f64, to: f64, t: f64) -> f64 {
    from * (to / from).powf(t)
}

/// The viewport that puts `anchor_world` at `anchor_screen` at `scale`.
pub fn anchored_viewport(anchor_world: Point, anchor_screen: Point, scale: f64) -> Viewport {
    Viewport {
        scale,
        x: anchor_world.x - anchor_screen.x / scale,
        y: anchor_world.y - anchor_screen.y / scale,
    }
}

pub fn screen_pos_of(world: Point, vp: Viewport) -> Point {
    Point {
        x: (world.x - vp.x) * vp.scale,
        y: (world.y - vp.y) * vp.scale,
    }
}

pub fn world_pos_of(screen: Point, vp: Viewport) -> Point {
    Point {
        x: vp.x + screen.x / vp.scale,
        y: vp.y + screen.y / vp.scale,
    }
}

/// One frame of a combined zoom-and-pan at eased progress `t`.
pub fn fit_frame(from: Viewport, to: Viewport, size: Size, t: f64) -> Viewport {
    let e = ease_out_cubic(t);
    let a = viewport_centre(from, size);
    let b = viewport_centre(to, size);
    centre_on(
        Point {
            x: a.x + (b.x - a.x) * e,
            y: a.y + (b.y - a.y) * e,
        },
        scale_at(from.scale, to.scale, e),
        size,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    const SIZE: Size = Size { w: 1000., h: 800. };
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-10, "{a} != {b}");
    }
    #[test]
    fn fit_scales_to_tighter_axis() {
        close(
            fit_rect(
                Rect {
                    x: 0.,
                    y: 0.,
                    w: 2000.,
                    h: 1000.,
                },
                SIZE,
            )
            .scale,
            (1000. - FIT_PADDING * 2.) / 2000.,
        );
    }
    #[test]
    fn fit_centers_card() {
        let c = viewport_centre(
            fit_rect(
                Rect {
                    x: 500.,
                    y: 500.,
                    w: 2000.,
                    h: 1000.,
                },
                SIZE,
            ),
            SIZE,
        );
        close(c.x, 1500.);
        close(c.y, 1000.);
    }
    #[test]
    fn small_card_fit_respects_ceiling() {
        assert_eq!(
            fit_rect(
                Rect {
                    x: 0.,
                    y: 0.,
                    w: 100.,
                    h: 100.
                },
                SIZE
            )
            .scale,
            MAX_FIT_SCALE
        );
    }
    #[test]
    fn fit_exact_scale_below_ceiling() {
        close(
            fit_rect(
                Rect {
                    x: 0.,
                    y: 0.,
                    w: 1600.,
                    h: 1600.,
                },
                SIZE,
            )
            .scale,
            (800. - FIT_PADDING * 2.) / 1600.,
        );
    }
    #[test]
    fn fit_never_magnifies() {
        assert_eq!(
            fit_rect(
                Rect {
                    x: 0.,
                    y: 0.,
                    w: 400.,
                    h: 400.
                },
                SIZE
            )
            .scale,
            1.
        );
        assert_eq!(MAX_FIT_SCALE, 1.);
    }
    #[test]
    fn centre_on_requested_scale() {
        let vp = centre_on(Point { x: 1000., y: 1000. }, 1., SIZE);
        assert_eq!(viewport_centre(vp, SIZE), Point { x: 1000., y: 1000. });
        assert_eq!(vp.scale, 1.);
    }
    #[test]
    fn centre_on_clamps() {
        assert_eq!(centre_on(Point { x: 0., y: 0. }, 99., SIZE).scale, 4.);
        assert_eq!(centre_on(Point { x: 0., y: 0. }, 0.0001, SIZE).scale, 0.05);
    }
    #[test]
    fn actual_size_keeps_world_center() {
        let before = viewport_centre(
            Viewport {
                x: -400.,
                y: -300.,
                scale: 0.2,
            },
            SIZE,
        );
        let after = viewport_centre(centre_on(before, 1., SIZE), SIZE);
        close(after.x, before.x);
        close(after.y, before.y);
    }
    #[test]
    fn bounds_span_all_cards() {
        assert_eq!(
            bounding_rect(&[
                Rect {
                    x: 0.,
                    y: 0.,
                    w: 100.,
                    h: 100.
                },
                Rect {
                    x: 500.,
                    y: 300.,
                    w: 200.,
                    h: 200.
                }
            ]),
            Some(Rect {
                x: 0.,
                y: 0.,
                w: 700.,
                h: 500.
            })
        );
    }
    #[test]
    fn no_cards_have_no_bounds() {
        assert_eq!(bounding_rect(&[]), None);
    }
    #[test]
    fn easing_endpoints() {
        assert_eq!(ease_out_cubic(0.), 0.);
        assert_eq!(ease_out_cubic(1.), 1.);
    }
    #[test]
    fn easing_decelerates() {
        assert!(ease_out_cubic(0.5) > 0.5);
    }
    #[test]
    fn easing_clamps() {
        assert_eq!(ease_out_cubic(-1.), 0.);
        assert_eq!(ease_out_cubic(2.), 1.);
    }
    #[test]
    fn geometric_interpolation() {
        close(scale_at(1., 4., 0.5), 2.);
        assert_eq!(scale_at(1., 4., 0.), 1.);
        close(scale_at(1., 4., 1.), 4.);
    }
    #[test]
    fn geometric_interpolation_symmetric() {
        close(scale_at(4., 1., 0.5), 2.);
    }
    #[test]
    fn anchor_stays_on_screen() {
        let world = Point { x: 500., y: 400. };
        let screen = Point { x: 300., y: 200. };
        for scale in [0.2, 0.5, 1., 2., 4.] {
            let p = screen_pos_of(world, anchored_viewport(world, screen, scale));
            close(p.x, screen.x);
            close(p.y, screen.y);
        }
    }
    #[test]
    fn centering_independent_of_card_position() {
        let center = Point { x: 500., y: 400. };
        for p in [Point { x: 100., y: 100. }, Point { x: 9000., y: 9000. }] {
            assert_eq!(screen_pos_of(p, anchored_viewport(p, center, 2.)), center);
        }
    }
    #[test]
    fn fit_frame_starts_at_current_viewport() {
        let from = Viewport {
            x: 0.,
            y: 0.,
            scale: 1.,
        };
        let to = Viewport {
            x: -500.,
            y: -400.,
            scale: 0.25,
        };
        assert_eq!(fit_frame(from, to, SIZE, 0.), from);
    }
    #[test]
    fn fit_frame_lands_on_target() {
        let from = Viewport {
            x: 0.,
            y: 0.,
            scale: 1.,
        };
        let to = fit_rect(
            Rect {
                x: 200.,
                y: 200.,
                w: 400.,
                h: 400.,
            },
            SIZE,
        );
        let end = fit_frame(from, to, SIZE, 1.);
        close(end.scale, to.scale);
        close(end.x, to.x);
        close(end.y, to.y);
    }
    #[test]
    fn fit_frame_keeps_center_throughout() {
        let p = Point { x: 200., y: 200. };
        let from = centre_on(p, 2., SIZE);
        let to = centre_on(p, 0.25, SIZE);
        for t in [0., 0.25, 0.5, 0.75, 1.] {
            let c = viewport_centre(fit_frame(from, to, SIZE, t), SIZE);
            close(c.x, p.x);
            close(c.y, p.y);
        }
    }
    // Additional native port checks for API gaps found during Phase 0.
    #[test]
    fn custom_fit_padding_matches_reference() {
        let rect = Rect {
            x: 0.,
            y: 0.,
            w: 2000.,
            h: 1000.,
        };
        let vp = fit_rect_with_padding(rect, SIZE, 100.);
        close(vp.scale, 0.4);
        assert_eq!(viewport_centre(vp, SIZE), Point { x: 1000., y: 500. });
    }
    #[test]
    fn durations_match_reference() {
        assert_eq!(PAN_DURATION_MS, 160.);
        assert_eq!(FIT_DURATION_MS, 240.);
        assert_eq!(ZOOM_DURATION_MS, 130.);
    }
    #[test]
    fn screen_world_round_trip() {
        let world = Point {
            x: -12345.625,
            y: 9876.25,
        };
        for scale in [0.05, 0.24, 0.6, 1., 4.] {
            let vp = Viewport {
                x: 321.5,
                y: -765.25,
                scale,
            };
            let back = world_pos_of(screen_pos_of(world, vp), vp);
            close(back.x, world.x);
            close(back.y, world.y);
        }
    }
}
