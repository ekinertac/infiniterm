//! Reading mode's geometry: where the canvas looks while you read the bottom
//! of one terminal card at a larger size (`Model::read_card`, #313).
//!
//! A regular 50 by 80 card fits the window at about 100%, which is small on a
//! big screen; zooming the whole canvas loses the prompt off the bottom. So the
//! view is zoomed to `ui.readZoom` with the card's bottom edge at the window's
//! bottom and the card centred across; Cmd+Up and Cmd+Down then move the view
//! along the card and no further than its edges. Pure, so the limits are tested
//! here; the model keeps the state and the ui draws what it is told.
//!
//! Related: `viewport` (the coordinates: a viewport's x and y are the world
//! point at the window's top-left), `Model::frame_card` (the plain fit).
use crate::grid::{Rect, Size};

/// A little card edge shown beyond the text, in screen px: the zoom never
/// lands the last row on the window's very edge.
pub const PAD_PX: f64 = 16.;

/// The view of one card: `x` is fixed, `y` ranges from `lo` (card top) to
/// `hi` (card bottom at the window's bottom).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub lo: f64,
    pub hi: f64,
}

/// The frame for `rect` seen through a window of `view` at `scale`. A card
/// that fits whole at that scale has nowhere to pan: `lo` and `hi` are the
/// centred position.
pub fn frame(rect: Rect, view: Size, scale: f64) -> Frame {
    let (vw, vh) = (view.w / scale, view.h / scale);
    let pad = PAD_PX / scale;
    let x = rect.x + rect.w / 2. - vw / 2.;
    let top = rect.y - pad;
    let bottom = rect.y + rect.h + pad - vh;
    if bottom <= top {
        let centred = rect.y + rect.h / 2. - vh / 2.;
        return Frame {
            x,
            lo: centred,
            hi: centred,
        };
    }
    Frame {
        x,
        lo: top,
        hi: bottom,
    }
}

/// `y` moved by `delta` world px and held inside the frame.
pub fn pan(y: f64, delta: f64, frame: &Frame) -> f64 {
    (y + delta).clamp(frame.lo, frame.hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: Size = Size { w: 3840., h: 2000. };

    fn card() -> Rect {
        Rect {
            x: 100.,
            y: 200.,
            w: 600.,
            h: 1800.,
        }
    }

    #[test]
    fn the_card_bottom_sits_at_the_window_bottom_and_the_card_is_centred() {
        let f = frame(card(), VIEW, 1.5);
        let vh = VIEW.h / 1.5;
        // The window's bottom edge, in world px, is the card's bottom plus the pad.
        assert!((f.hi + vh - (200. + 1800. + PAD_PX / 1.5)).abs() < 1e-9);
        // The card's centre is the window's centre across.
        assert!((f.x + VIEW.w / 1.5 / 2. - 400.).abs() < 1e-9);
        // The top of the pan range shows the card's top with a pad above it.
        assert!((f.lo - (200. - PAD_PX / 1.5)).abs() < 1e-9);
        assert!(f.lo < f.hi);
    }

    #[test]
    fn panning_stops_at_both_ends_of_the_card() {
        let f = frame(card(), VIEW, 1.5);
        assert_eq!(pan(f.hi, 300., &f), f.hi, "already at the bottom");
        assert_eq!(pan(f.hi, -300., &f), f.hi - 300.);
        assert_eq!(pan(f.lo + 10., -300., &f), f.lo, "stops at the top");
        assert_eq!(pan(f.lo, 1e9, &f), f.hi);
    }

    #[test]
    fn a_card_that_fits_whole_has_nowhere_to_pan() {
        let small = Rect {
            x: 0.,
            y: 0.,
            w: 400.,
            h: 300.,
        };
        let f = frame(small, VIEW, 1.5);
        assert_eq!(f.lo, f.hi);
        assert_eq!(pan(f.lo, 500., &f), f.lo);
        // Centred on the card.
        assert!((f.lo + VIEW.h / 1.5 / 2. - 150.).abs() < 1e-9);
    }
}
