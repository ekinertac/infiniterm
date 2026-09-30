//! The status bar's frame counter (`ui.showFps`), measured only where a
//! frame rate means something: while frames are drawn back to back (an
//! animation, a drag, a pan, a flood of output). The app draws a frame only
//! when something changes, and a crowded canvas refreshes card content at
//! most every `chrome::FAR_REFRESH_MS` on purpose, so frames per wall-clock
//! second read "6 fps" at rest, in the warning colour, next to the film's
//! 110 (issue #16). Nothing was slow; the number measured idleness.
//!
//! Fed by `AppView::frame` (paint.rs) once per frame, read by the status
//! bar (overlays.rs). Pure, so it is tested here.
use std::collections::VecDeque;

/// Frames further apart than this are not one run: a 10 fps floor, which a
/// real stutter stays above and the 250 ms content rationing does not.
pub const RUN_GAP_MS: f64 = 100.;
/// The rate is over the last second of the run.
pub const WINDOW_MS: f64 = 1000.;
/// A run needs this many frames before its rate is worth showing.
pub const MIN_FRAMES: usize = 5;
/// After the last frame of a run the rate stays up this long, then goes.
pub const SHOW_FOR_MS: f64 = 1000.;

#[derive(Debug, Default)]
pub struct FrameRate {
    /// The current run's frame times, trimmed to `WINDOW_MS`.
    run: VecDeque<f64>,
    rate: Option<f64>,
}

impl FrameRate {
    pub fn frame(&mut self, now: f64) {
        if self.run.back().is_some_and(|&last| now - last > RUN_GAP_MS) {
            self.run.clear();
        }
        self.run.push_back(now);
        while self.run.front().is_some_and(|&t| now - t > WINDOW_MS) {
            self.run.pop_front();
        }
        if self.run.len() >= MIN_FRAMES {
            let span = now - self.run.front().copied().unwrap_or(now);
            if span > 0. {
                self.rate = Some((self.run.len() - 1) as f64 * 1000. / span);
            }
        } else if self.run.len() == 1 {
            // A new run: the old rate says nothing about it.
            self.rate = None;
        }
    }

    /// A rate is still up though its run ended over `SHOW_FOR_MS` ago:
    /// one more frame takes it down (the ui asks for it in `needs_frame`,
    /// or the last rate of a zoom stayed on screen until something else
    /// drew).
    pub fn expired(&self, now: f64) -> bool {
        self.rate.is_some()
            && self
                .run
                .back()
                .is_some_and(|&last| now - last > SHOW_FOR_MS)
    }

    /// The rate to show, or nothing at rest.
    pub fn shown(&self, now: f64) -> Option<f64> {
        let last = *self.run.back()?;
        (now - last <= SHOW_FOR_MS).then_some(self.rate).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: &mut FrameRate, from: f64, every: f64, n: usize) -> f64 {
        let mut t = from;
        for _ in 0..n {
            f.frame(t);
            t += every;
        }
        t - every
    }

    // A 120 Hz animation reads 120.
    #[test]
    fn frames_back_to_back_give_their_rate() {
        let mut f = FrameRate::default();
        let end = run(&mut f, 0., 1000. / 120., 60);
        assert!((f.shown(end).unwrap() - 120.).abs() < 1.);
    }

    // Four content refreshes a second on a crowded canvas are not a frame
    // rate: nothing is shown, and certainly not a slow one.
    #[test]
    fn rationed_refreshes_at_rest_show_nothing() {
        let mut f = FrameRate::default();
        let end = run(&mut f, 0., 250., 20);
        assert_eq!(f.shown(end), None);
    }

    // A real stutter is still reported, and the rate goes a second after
    // the run ends.
    #[test]
    fn a_slow_run_shows_and_then_goes() {
        let mut f = FrameRate::default();
        let end = run(&mut f, 0., 1000. / 20., 30);
        assert!((f.shown(end).unwrap() - 20.).abs() < 1.);
        assert!(f.shown(end + SHOW_FOR_MS + 1.).is_none());
        assert!(
            f.expired(end + SHOW_FOR_MS + 1.),
            "a frame is owed to take it down"
        );
        f.frame(end + SHOW_FOR_MS + 1.);
        assert!(!f.expired(end + 5. * SHOW_FOR_MS));
    }
}
