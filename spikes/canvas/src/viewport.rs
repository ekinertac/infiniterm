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
//! Pure; everything that moves the viewport goes through here.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

/// Top-left corner in world units plus the scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
}

pub const FIT_PADDING: f32 = 48.0;
pub const MIN_SCALE: f32 = 0.05;
pub const MAX_SCALE: f32 = 4.0;
pub const MAX_FIT_SCALE: f32 = 1.0;
pub const FIT_DURATION_MS: f32 = 240.0;
pub const ZOOM_DURATION_MS: f32 = 130.0;

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
    Some(Rect { x: min_x, y: min_y, w: max_x - min_x, h: max_y - min_y })
}

/// Largest scale (capped) at which `rect` plus padding fits, centred.
pub fn fit_rect(rect: Rect, size: Size) -> Viewport {
    let usable_w = (size.w - FIT_PADDING * 2.0).max(1.0);
    let usable_h = (size.h - FIT_PADDING * 2.0).max(1.0);
    let scale = (usable_w / rect.w).min(usable_h / rect.h).max(MIN_SCALE).min(MAX_FIT_SCALE);
    centre_on(Point { x: rect.x + rect.w / 2.0, y: rect.y + rect.h / 2.0 }, scale, size)
}

/// Viewport with `point` (world) in the middle of the content area.
pub fn centre_on(point: Point, scale: f32, size: Size) -> Viewport {
    let clamped = scale.clamp(MIN_SCALE, MAX_SCALE);
    Viewport {
        scale: clamped,
        x: point.x - size.w / clamped / 2.0,
        y: point.y - size.h / clamped / 2.0,
    }
}

/// The world point at the centre of the content area.
pub fn viewport_centre(vp: Viewport, size: Size) -> Point {
    Point { x: vp.x + size.w / vp.scale / 2.0, y: vp.y + size.h / vp.scale / 2.0 }
}

pub fn ease_out_cubic(t: f32) -> f32 {
    let c = t.clamp(0.0, 1.0);
    1.0 - (1.0 - c).powi(3)
}

/// Geometric interpolation: each step of the ramp is the same zoom ratio.
pub fn scale_at(from: f32, to: f32, t: f32) -> f32 {
    from * (to / from).powf(t)
}

/// The viewport that puts `anchor_world` at `anchor_screen` at `scale`.
pub fn anchored_viewport(anchor_world: Point, anchor_screen: Point, scale: f32) -> Viewport {
    Viewport {
        scale,
        x: anchor_world.x - anchor_screen.x / scale,
        y: anchor_world.y - anchor_screen.y / scale,
    }
}

pub fn screen_pos_of(world: Point, vp: Viewport) -> Point {
    Point { x: (world.x - vp.x) * vp.scale, y: (world.y - vp.y) * vp.scale }
}

pub fn world_pos_of(screen: Point, vp: Viewport) -> Point {
    Point { x: vp.x + screen.x / vp.scale, y: vp.y + screen.y / vp.scale }
}

/// One frame of a combined zoom-and-pan at eased progress `t`.
pub fn fit_frame(from: Viewport, to: Viewport, size: Size, t: f32) -> Viewport {
    let e = ease_out_cubic(t);
    let a = viewport_centre(from, size);
    let b = viewport_centre(to, size);
    centre_on(
        Point { x: a.x + (b.x - a.x) * e, y: a.y + (b.y - a.y) * e },
        scale_at(from.scale, to.scale, e),
        size,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Size = Size { w: 1000.0, h: 800.0 };

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn fit_scales_a_large_card_down_to_the_tighter_axis() {
        let vp = fit_rect(Rect { x: 0.0, y: 0.0, w: 2000.0, h: 1000.0 }, SIZE);
        assert!(close(vp.scale, (1000.0 - FIT_PADDING * 2.0) / 2000.0));
    }

    #[test]
    fn fit_centres_the_card() {
        let vp = fit_rect(Rect { x: 500.0, y: 500.0, w: 2000.0, h: 1000.0 }, SIZE);
        let c = viewport_centre(vp, SIZE);
        assert!(close(c.x, 1500.0) && close(c.y, 1000.0));
    }

    #[test]
    fn fit_never_magnifies_past_actual_size() {
        assert_eq!(fit_rect(Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, SIZE).scale, MAX_FIT_SCALE);
        assert_eq!(fit_rect(Rect { x: 0.0, y: 0.0, w: 400.0, h: 400.0 }, SIZE).scale, 1.0);
        assert_eq!(MAX_FIT_SCALE, 1.0);
    }

    #[test]
    fn fit_picks_the_exact_scale_below_the_ceiling() {
        let vp = fit_rect(Rect { x: 0.0, y: 0.0, w: 1600.0, h: 1600.0 }, SIZE);
        assert!(close(vp.scale, (800.0 - FIT_PADDING * 2.0) / 1600.0));
    }

    #[test]
    fn centre_on_puts_the_point_in_the_middle_and_clamps() {
        let vp = centre_on(Point { x: 1000.0, y: 1000.0 }, 1.0, SIZE);
        assert_eq!(viewport_centre(vp, SIZE), Point { x: 1000.0, y: 1000.0 });
        assert_eq!(centre_on(Point { x: 0.0, y: 0.0 }, 99.0, SIZE).scale, 4.0);
        assert_eq!(centre_on(Point { x: 0.0, y: 0.0 }, 0.0001, SIZE).scale, 0.05);
    }

    #[test]
    fn actual_size_keeps_you_where_you_were_looking() {
        let zoomed_out = Viewport { x: -400.0, y: -300.0, scale: 0.2 };
        let before = viewport_centre(zoomed_out, SIZE);
        let after = viewport_centre(centre_on(before, 1.0, SIZE), SIZE);
        assert!(close(after.x, before.x) && close(after.y, before.y));
    }

    #[test]
    fn bounding_rect_spans_every_card_and_none_is_none() {
        let b = bounding_rect(&[
            Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 },
            Rect { x: 500.0, y: 300.0, w: 200.0, h: 200.0 },
        ]);
        assert_eq!(b, Some(Rect { x: 0.0, y: 0.0, w: 700.0, h: 500.0 }));
        assert_eq!(bounding_rect(&[]), None);
    }

    #[test]
    fn easing_starts_ends_decelerates_and_clamps() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert!(ease_out_cubic(0.5) > 0.5);
        assert_eq!(ease_out_cubic(-1.0), 0.0);
        assert_eq!(ease_out_cubic(2.0), 1.0);
    }

    #[test]
    fn scale_interpolates_geometrically_both_ways() {
        assert!(close(scale_at(1.0, 4.0, 0.5), 2.0));
        assert_eq!(scale_at(1.0, 4.0, 0.0), 1.0);
        assert!(close(scale_at(1.0, 4.0, 1.0), 4.0));
        assert!(close(scale_at(4.0, 1.0, 0.5), 2.0));
    }

    #[test]
    fn an_anchored_viewport_keeps_the_anchor_on_screen() {
        let world = Point { x: 500.0, y: 400.0 };
        let screen = Point { x: 300.0, y: 200.0 };
        for scale in [0.2, 0.5, 1.0, 2.0, 4.0] {
            let back = screen_pos_of(world, anchored_viewport(world, screen, scale));
            assert!(close(back.x, screen.x) && close(back.y, screen.y));
        }
    }

    #[test]
    fn fit_frame_starts_at_from_and_lands_on_to() {
        let from = Viewport { x: 0.0, y: 0.0, scale: 1.0 };
        let to = fit_rect(Rect { x: 200.0, y: 200.0, w: 400.0, h: 400.0 }, SIZE);
        let start = fit_frame(from, to, SIZE, 0.0);
        assert!(close(start.x, from.x) && close(start.y, from.y) && close(start.scale, from.scale));
        let end = fit_frame(from, to, SIZE, 1.0);
        assert!(close(end.x, to.x) && close(end.y, to.y) && close(end.scale, to.scale));
    }

    #[test]
    fn fit_frame_keeps_a_centred_target_centred_throughout() {
        let point = Point { x: 200.0, y: 200.0 };
        let from = centre_on(point, 2.0, SIZE);
        let to = centre_on(point, 0.25, SIZE);
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let c = viewport_centre(fit_frame(from, to, SIZE, t), SIZE);
            assert!(close(c.x, point.x) && close(c.y, point.y));
        }
    }
}
