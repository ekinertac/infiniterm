//! "Resize to window" (`card.size.window`, #585): the selected cards keep
//! their width and take the window's aspect ratio, reflowed so nothing
//! overlaps. Also the last row of the `card.size` picker (Cmd+Alt+S), so the
//! palette, the keymap and `ift run` all reach it.
//!
//! Called from `Model::palette_run` (`Source::Sizes`) and the registry. The
//! geometry is `window_fit.rs`; this file gathers the bodies (the selection
//! as moving, every other card of the SAME workspace as fixed), applies the
//! rects as ONE undo step, and frames the result.
//!
//! Decisions: the aspect is `view_size` (the canvas area in the window). A
//! maximised card refuses (it shows no rect to change). `locked` is the
//! keyboard lock of a browser or editor and does not stop a resize. Cards in
//! other workspaces are never read. The view fits the selection afterwards,
//! because the point is to see the cards in the window's shape.
use super::Model;
use crate::cards::viewport_measured;
use crate::grid::Rect;
use crate::viewport::bounding_rect;
use crate::window_fit::{reflow, Body};

/// The picker row's id and label.
pub const WINDOW_ROW: (&str, &str) = ("window", "Resize to window");

impl Model {
    /// Resizes the selected cards to the window's proportions.
    pub fn resize_to_window(&mut self) {
        if self.selection.maximized {
            self.notify("restore the card first: it is maximised");
            return;
        }
        if !viewport_measured(self.view_size) {
            self.notify("the window has no size yet");
            return;
        }
        let aspect = self.view_size.w / self.view_size.h;
        let body = |c: &super::Card| Body {
            id: c.id.clone(),
            rect: c.rect,
            group: c.group_id.clone(),
        };
        let selected: Vec<Body> = self.selected().iter().map(|c| body(c)).collect();
        if selected.is_empty() {
            return;
        }
        let fixed: Vec<Body> = self
            .here()
            .iter()
            .filter(|c| !selected.iter().any(|s| s.id == c.id))
            .map(|c| body(c))
            .collect();
        let next = reflow(&selected, &fixed, aspect, self.gap());
        if next
            .iter()
            .zip(&selected)
            .all(|(n, old)| n.rect == old.rect)
        {
            self.notify("already the window's shape");
            return;
        }
        let ids: Vec<String> = selected.iter().map(|b| b.id.clone()).collect();
        self.remember_layout();
        self.mark_swap(&ids);
        for b in &next {
            if let Some(c) = self.card_mut(&b.id) {
                c.rect = b.rect;
                c.soft_group_id = None;
            }
        }
        self.dirty_layout = true;
        let rects: Vec<Rect> = next.iter().map(|b| b.rect).collect();
        if let Some(bounds) = bounding_rect(&rects) {
            self.apply_viewport(self.fit_viewport(bounds));
        }
    }
}

pub fn register(r: &mut crate::commands::CommandRegistry<Model>) {
    r.register("card.size.window", "Card: resize to window", |m| {
        m.resize_to_window()
    });
}
