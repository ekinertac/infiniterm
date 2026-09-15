//! Shared fixture constructors for the ported geometry tests.
//! Test modules use these to preserve the reference coordinates without UI state.
//! No production code depends on this module.
use crate::{cards::PlacedCard, grid::Rect};
pub fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { x, y, w, h }
}
pub fn card(id: &str, x: f64, y: f64) -> PlacedCard {
    PlacedCard {
        id: id.into(),
        rect: r(x, y, 100., 100.),
        group_id: None,
    }
}
