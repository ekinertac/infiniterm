//! The font size text is RASTERISED at, as opposed to the size it is laid
//! out at (#758).
//!
//! gpui caches each glyph per exact font size and never evicts one, and a
//! card's text is drawn at `font_px * zoom`, a continuous value: every frame
//! of a zoom animation rasterised a fresh copy of every visible glyph into
//! 1 MB atlas pages that lived until the app quit (211 pages, 286 MB of GPU
//! memory, on a viewer after 18 hours).
//!
//! Two kinds of text, because snapping a size means different things to them:
//! - CELL-PLACED (the terminal): each glyph is painted at its own cell, so a
//!   snapped size changes the glyph a little and the layout not at all. It
//!   always snaps.
//! - ADVANCE-PLACED (editor, page, transcript, diff, the card label): gpui
//!   lays the line out by the shaped advances, so a snapped size makes the
//!   whole line narrower or wider (5% at fit-all on the welcome card). It
//!   snaps only while the zoom is changing, and the frame after the zoom
//!   settles draws it at its exact size.
//!
//! Chosen by measurement, 2026-11-12 (#758): five candidates, three runs each
//! on the mini, 8 zoom passes over 5 terminals, an editor and a page, each
//! pass resting at a new scale. Today's exact sizes grew 29 pages a pass and
//! ended at 314-329 MB; this rule grew about 1 page a pass (one exact set of
//! advance-placed glyphs per new resting scale) and ended at 84-85 MB, with
//! p95 frame 7.6 ms against 18.0 and the worst frame 18 ms against 41.
//! Snapping terminal text alone was not enough (275-292 MB); clearing gpui's
//! atlas past a limit got the memory back but not the frame time, and needed
//! a gpui fork. The scenario and numbers are in issue #758.

use gpui::{px, Pixels, Window};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The snap grid, in DEVICE pixels. Measured against 0.5: the same memory
/// within a few pages, a lower worst frame (18 ms against 23), and at rest
/// the terminal's lines end at the same x as exact sizes.
const SNAP_STEP_DEVICE_PX: f32 = 1.0;

/// How long after the scale last changed text still counts as zooming. The
/// frame after that is drawn exact, so text comes to rest at its true size.
/// Longer than one frame at 60 Hz with margin; short enough not to be seen.
const ZOOM_HOLD: Duration = Duration::from_millis(150);

/// Set once per frame by the app: an animation, a pan or a gesture is on.
static MOVING: AtomicBool = AtomicBool::new(false);
/// The scale seen last frame and when it last changed: a pinch or a
/// Cmd+scroll sets the viewport directly, with no animation to ask about.
static SCALE: Mutex<(u64, Option<Instant>)> = Mutex::new((0, None));

pub fn set_moving(moving: bool) {
    MOVING.store(moving, Ordering::Relaxed);
}

pub fn note_scale(scale: f64) {
    if let Ok(mut s) = SCALE.lock() {
        if s.0 != scale.to_bits() {
            *s = (scale.to_bits(), Some(Instant::now()));
        }
    }
}

/// Still inside the hold after the last scale change: the app keeps asking
/// for frames until it ends, so the resting frame is drawn exact.
pub fn zoom_settling() -> bool {
    SCALE
        .lock()
        .ok()
        .and_then(|s| s.1)
        .is_some_and(|t| t.elapsed() < ZOOM_HOLD)
}

fn zooming() -> bool {
    MOVING.load(Ordering::Relaxed) || zoom_settling()
}

/// `logical` rounded to the nearest multiple of `step` device pixels, never
/// to zero.
pub fn snap(logical: f32, scale_factor: f32, step: f32) -> f32 {
    let device = logical * scale_factor;
    let snapped = ((device / step).round() * step).max(step);
    snapped / scale_factor
}

fn raster(size: Pixels, snaps: bool, window: &Window) -> Pixels {
    if snaps {
        px(snap(
            f32::from(size),
            window.scale_factor(),
            SNAP_STEP_DEVICE_PX,
        ))
    } else {
        size
    }
}

/// The terminal's glyph size at this zoom: always on the device-pixel grid.
pub fn cell_placed(world_px: f64, scale: f64, window: &Window) -> Pixels {
    raster(px((world_px * scale) as f32), true, window)
}

/// An editor's, page's or transcript's font size at this zoom: on the grid
/// while zooming, exact at rest.
pub fn zoomed(world_px: f64, scale: f64, window: &Window) -> Pixels {
    raster(px((world_px * scale) as f32), zooming(), window)
}

/// The same for a size already in screen pixels (the card label).
pub fn advance_placed(size: Pixels, window: &Window) -> Pixels {
    raster(size, zooming(), window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snap_lands_on_the_device_pixel_grid() {
        assert_eq!(snap(11.3, 1., 1.), 11.);
        assert_eq!(snap(11.6, 1., 1.), 12.);
        // Retina: 11.3 logical is 22.6 device, nearest whole device pixel 23.
        assert_eq!(snap(11.3, 2., 1.), 11.5);
    }

    #[test]
    fn a_snap_is_idempotent_so_two_call_sites_agree() {
        for i in 0..400 {
            let v = 3. + i as f32 * 0.037;
            let once = snap(v, 1., SNAP_STEP_DEVICE_PX);
            assert_eq!(snap(once, 1., SNAP_STEP_DEVICE_PX), once);
        }
    }

    #[test]
    fn a_zoom_sweep_needs_few_sizes() {
        // A 14 px font zoomed from 20% to 200% in 1000 frames: exact sizes
        // are all different, whole device pixels are 25.
        let sizes: std::collections::HashSet<u32> = (0..1000)
            .map(|i| snap(14. * (0.2 + 1.8 * i as f32 / 999.), 1., SNAP_STEP_DEVICE_PX).to_bits())
            .collect();
        assert!(sizes.len() <= 26, "{}", sizes.len());
    }

    #[test]
    fn a_tiny_size_never_snaps_to_zero() {
        assert_eq!(snap(0.1, 1., 1.), 1.);
    }

    #[test]
    fn a_scale_change_counts_as_zooming_for_the_hold_then_stops() {
        note_scale(0.5);
        note_scale(0.75);
        assert!(zoom_settling());
        std::thread::sleep(ZOOM_HOLD + Duration::from_millis(20));
        assert!(!zoom_settling());
        // The same scale again is not a change.
        note_scale(0.75);
        assert!(!zoom_settling());
    }
}
