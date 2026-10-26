//! "Resize to window" (`card.size.window`, #585): the selected cards keep
//! their width and take the window's aspect ratio, reflowed so nothing
//! overlaps. "Default size" (`card.size.default`): the selected cards take the
//! default card size, same reflow. Both are the last rows of the `card.size`
//! picker (Cmd+Alt+S), so the palette, the keymap and `ift run` reach them.
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
use crate::window_fit::{reflow_to, Body, Target};

/// The picker rows' ids and labels. The default-size label is Ekin's
/// placeholder (he did not pick a name): a rename is this one line.
pub const WINDOW_ROW: (&str, &str) = ("window", "Resize to window");
pub const DEFAULT_ROW: (&str, &str) = (
    "default",
    "Default size: reset every selected card to the size in settings",
);

impl Model {
    /// Resizes the selected cards to the window's proportions.
    pub fn resize_to_window(&mut self) {
        let aspect = self.view_size.w / self.view_size.h;
        let target = viewport_measured(self.view_size).then_some(Target::Window(aspect));
        self.reflow_selection(target, "already the window's shape");
    }

    /// Takes every selected card to the default size of `cards.width` and
    /// `cards.height` (else from the window), reflowed so nothing overlaps.
    /// `Fraction::Full` acts on the one active card; this acts on all.
    pub fn reset_selection_to_default_size(&mut self) {
        let target = Target::Size(self.default_size());
        self.reflow_selection(Some(target), "already the default size");
    }

    /// The selection resized to `target` and reflowed among the other cards
    /// of this workspace, as one undo step. `None` is a window with no size.
    fn reflow_selection(&mut self, target: Option<Target>, unchanged: &str) {
        if self.selection.maximized {
            self.notify("restore the card first: it is maximised");
            return;
        }
        let Some(target) = target else {
            self.notify("the window has no size yet");
            return;
        };
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
        let next = reflow_to(&selected, &fixed, target, self.gap());
        if next
            .iter()
            .zip(&selected)
            .all(|(n, old)| n.rect == old.rect)
        {
            self.notify(unchanged);
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
    r.register("card.size.default", DEFAULT_ROW.1, |m| {
        m.reset_selection_to_default_size()
    });
}
