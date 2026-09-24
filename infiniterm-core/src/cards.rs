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
/// A new card sized from the window: the canvas at 100% less a margin all
/// round, so one card fills the view the way a terminal window would, and
/// is landscape on a landscape screen (Ekin, 2026-09-24: the fixed 69 by 80
/// cells were portrait and suited only his 32-inch 4K). Capped at
/// `AUTO_MAX_W`, shape kept, on the grid, never under 24 cells a side.
pub fn auto_size(view: Size) -> Size {
    let margin = GRID_SIZE * 2.;
    let minimum = GRID_SIZE * 24.;
    let (mut w, mut h) = if viewport_measured(view) {
        (view.w - margin * 2., view.h - margin * 2.)
    } else {
        (minimum, minimum)
    };
    if w > AUTO_MAX_W {
        h *= AUTO_MAX_W / w;
        w = AUTO_MAX_W;
    }
    Size {
        w: snap(w).max(minimum),
        h: snap(h).max(minimum),
    }
}
/// The size of a new card: each side from the settings (`cards.width` /
/// `cards.height`, in cells) when set, else from the window.
pub fn default_size(cells: Size, view: Size) -> Size {
    let auto = auto_size(view);
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
        let air = auto_size(Size { w: 2048., h: 1240. });
        assert!(air.w > air.h, "{air:?}");
        assert_eq!((air.w, air.h), (1950., 1150.));
        assert_eq!(air.w % GRID_SIZE, 0.);
        assert_eq!(air.h % GRID_SIZE, 0.);
        let k4 = auto_size(Size { w: 3840., h: 2040. });
        assert_eq!(k4.w, AUTO_MAX_W);
        assert!(k4.w > k4.h, "{k4:?}");
        let view = Size { w: 2048., h: 1240. };
        assert_eq!(default_size(CARD_CELLS, view), fixed_size(CARD_CELLS));
        let mixed = default_size(Size { w: 69., h: 0. }, view);
        assert_eq!((mixed.w, mixed.h), (1725., 1150.));
        assert_eq!(default_size(Size { w: 0., h: 0. }, view), air);
    }
    #[test]
    fn auto_size_before_measurement() {
        assert_eq!(auto_size(Size { w: 0., h: 0. }), Size { w: 600., h: 600. });
    }
    #[test]
    fn fixed_card_wider_than_80_columns() {
        let cell = 20. * 0.5 - 1.;
        assert_eq!(cell, 9.);
        assert!(fixed_size(CARD_CELLS).w / cell > 100.);
    }
}
