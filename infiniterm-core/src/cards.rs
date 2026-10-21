//! Card geometry inputs and default sizes shared by pure canvas operations.
//! Port of sizing in cards.svelte.ts and cardSize.test.ts. UI owns live state.
//! Sizes and the measured viewport are arguments, so configuration cannot leak in.
use crate::grid::{snap, Rect, Size, GRID_SIZE};
/// Only geometry and membership: card bodies never enter the pure algorithms.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedCard {
    pub id: String,
    pub rect: Rect,
    pub group_id: Option<String>,
}
/// A geometry update returned by split and swap commands, without card state.
#[derive(Clone, Debug, PartialEq)]
pub struct CardRect {
    pub id: String,
    pub rect: Rect,
}
pub const CARD_CELLS: Size = Size { w: 69., h: 80. };
pub const GUTTER: f64 = GRID_SIZE;
pub const TYPICAL_CARDS: usize = 12;
/// A card sized from the window is never wider than this (80 cells): on a
/// 4K window "the whole window" is a 450-column terminal. It keeps the
/// window's shape as it shrinks.
pub const AUTO_MAX_W: f64 = GRID_SIZE * 80.;
pub fn viewport_measured(view: Size) -> bool {
    view.w > 0. && view.h > 0.
}
pub fn fixed_size(cells: Size) -> Size {
    Size {
        w: cells.w * GRID_SIZE,
        h: cells.h * GRID_SIZE,
    }
}
/// A new card sized from the window: the largest card of the chosen SHAPE
/// that fits the canvas at 100% less a margin all round, so one card fills
/// the view the way a terminal window would (Ekin, 2026-09-24: the fixed 69
/// by 80 cells were portrait and suited only his 32-inch 4K). `shape` is
/// width over height (`cards.shape`, 16:9 by default, so a canvas looks the
/// same on every Mac), or `None` for the window's own shape. Capped at
/// `AUTO_MAX_W`, shape kept, on the grid, never under 24 cells a side.
pub fn auto_size(view: Size, shape: Option<f64>) -> Size {
    let margin = GRID_SIZE * 2.;
    let minimum = GRID_SIZE * 24.;
    let (mut w, mut h) = if viewport_measured(view) {
        (view.w - margin * 2., view.h - margin * 2.)
    } else {
        (minimum, minimum)
    };
    if let Some(aspect) = shape.filter(|a| *a > 0.) {
        if w / h > aspect {
            w = h * aspect;
        } else {
            h = w / aspect;
        }
    }
    if w > AUTO_MAX_W {
        h *= AUTO_MAX_W / w;
        w = AUTO_MAX_W;
    }
    Size {
        w: snap(w).max(minimum),
        h: snap(h).max(minimum),
    }
}
/// `cards.shape` as width over height: "16:9" is 16/9, "window" is `None`
/// (the window's own shape). Anything else is not a shape.
pub fn parse_shape(s: &str) -> Option<Option<f64>> {
    if s.trim().eq_ignore_ascii_case("window") {
        return Some(None);
    }
    let (w, h) = s.split_once(':')?;
    let (w, h): (f64, f64) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w > 0. && h > 0. && w.is_finite() && h.is_finite()).then_some(Some(w / h))
}
pub fn shape_is_valid(s: &str) -> bool {
    parse_shape(s).is_some()
}
/// The size of a new card: each side from the settings (`cards.width` /
/// `cards.height`, in cells) when set, else from the window.
pub fn default_size(cells: Size, view: Size, shape: Option<f64>) -> Size {
    let auto = auto_size(view, shape);
    let fixed = fixed_size(cells);
    Size {
        w: if cells.w > 0. { fixed.w } else { auto.w },
        h: if cells.h > 0. { fixed.h } else { auto.h },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_card_matches_reference_terminal_grid() {
        let s = fixed_size(CARD_CELLS);
        assert!((s.w / 9.).floor() >= 191.);
        assert!((s.h / 24.).floor() >= 83.);
    }
    #[test]
    fn fixed_card_whole_cells() {
        let s = fixed_size(CARD_CELLS);
        assert_eq!(s.w % GRID_SIZE, 0.);
        assert_eq!(s.h % GRID_SIZE, 0.);
        assert_eq!(s.w, CARD_CELLS.w * GRID_SIZE);
        assert_eq!(s.h, CARD_CELLS.h * GRID_SIZE);
    }
    #[test]
    fn fixed_card_portrait() {
        let s = fixed_size(CARD_CELLS);
        assert!(s.h > s.w);
    }
    // A laptop's window gives a landscape card that nearly fills it; a 4K
    // window's is capped at 80 cells wide with the window's shape kept;
    // cells set in the settings win, side by side.
    #[test]
    fn a_card_from_the_window_is_landscape_and_capped() {
        let air = auto_size(Size { w: 2048., h: 1240. }, None);
        assert!(air.w > air.h, "{air:?}");
        assert_eq!((air.w, air.h), (1950., 1150.));
        assert_eq!(air.w % GRID_SIZE, 0.);
        assert_eq!(air.h % GRID_SIZE, 0.);
        let k4 = auto_size(Size { w: 3840., h: 2040. }, None);
        assert_eq!(k4.w, AUTO_MAX_W);
        assert!(k4.w > k4.h, "{k4:?}");
        let view = Size { w: 2048., h: 1240. };
        assert_eq!(default_size(CARD_CELLS, view, None), fixed_size(CARD_CELLS));
        let mixed = default_size(Size { w: 69., h: 0. }, view, None);
        assert_eq!((mixed.w, mixed.h), (1725., 1150.));
        assert_eq!(default_size(Size { w: 0., h: 0. }, view, None), air);
    }
    // 16:9 on the Air's 16:10-ish window and on the 4K: the same shape on
    // both, within the grid's rounding, and never past the window.
    #[test]
    fn a_shaped_card_keeps_its_shape_on_every_window() {
        for view in [
            Size { w: 2048., h: 1240. },
            Size { w: 3840., h: 2040. },
            Size { w: 1200., h: 1100. },
        ] {
            let s = auto_size(view, Some(16. / 9.));
            assert!((s.w / s.h - 16. / 9.).abs() < 0.05, "{view:?} gave {s:?}");
            assert!(s.w <= view.w && s.h <= view.h, "{view:?} gave {s:?}");
        }
    }
    #[test]
    fn shapes_parse_as_a_ratio_or_the_window() {
        assert_eq!(parse_shape("16:9"), Some(Some(16. / 9.)));
        assert_eq!(parse_shape(" 4 : 3 "), Some(Some(4. / 3.)));
        assert_eq!(parse_shape("Window"), Some(None));
        for bad in ["", "16x9", "0:9", "16:", "wide", "-1:2"] {
            assert_eq!(parse_shape(bad), None, "{bad}");
        }
    }
    #[test]
    fn auto_size_before_measurement() {
        assert_eq!(
            auto_size(Size { w: 0., h: 0. }, None),
            Size { w: 600., h: 600. }
        );
    }
    #[test]
    fn fixed_card_wider_than_80_columns() {
        let cell = 20. * 0.5 - 1.;
        assert_eq!(cell, 9.);
        assert!(fixed_size(CARD_CELLS).w / cell > 100.);
    }
}
