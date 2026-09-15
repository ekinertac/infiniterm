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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeMode {
    Fixed,
    Auto,
}
pub const SIZE_MODE: SizeMode = SizeMode::Fixed;
pub fn viewport_measured(view: Size) -> bool {
    view.w > 0. && view.h > 0.
}
pub fn fixed_size(cells: Size) -> Size {
    Size {
        w: cells.w * GRID_SIZE,
        h: cells.h * GRID_SIZE,
    }
}
pub fn auto_size(view: Size) -> Size {
    let margin = GRID_SIZE * 2.;
    let minimum = GRID_SIZE * 24.;
    let (w, h) = if viewport_measured(view) {
        (view.w / 2. - margin, view.h - margin * 2.)
    } else {
        (minimum, minimum)
    };
    Size {
        w: snap(w).max(minimum),
        h: snap(h).max(minimum),
    }
}
pub fn default_size(cells: Size, view: Size) -> Size {
    match SIZE_MODE {
        SizeMode::Fixed => fixed_size(cells),
        SizeMode::Auto => auto_size(view),
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
    #[test]
    fn auto_size_still_portrait() {
        let s = auto_size(Size { w: 1400., h: 860. });
        assert!(s.h > s.w);
        assert_eq!(s.w % GRID_SIZE, 0.);
        assert_eq!(s.h % GRID_SIZE, 0.);
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
