//! "Resize to window" commands (SPIKE, #585): the selected cards take the
//! window's aspect ratio, without overlapping, in the variants of
//! `window_fit::VARIANTS`. One command per variant (`card.size.window.area`,
//! `.width`, `.fill`, `.block`) and the same four as rows of the `card.size`
//! picker (Cmd+Alt+S), so the palette, the keymap and `ift run` all get them.
//!
//! Called from `Model::palette_run` (`Source::Sizes`) and the registry. The
//! geometry is `window_fit.rs`; this file only gathers the bodies (selection
//! as moving, every other card of the SAME workspace as fixed), applies the
//! rects as ONE undo step, and frames the result.
//!
//! Decisions: the aspect is `view_size` (the canvas area in the window), the
//! same size `cards::auto_size` reads. A maximised card refuses (it shows no
//! rect to change). `locked` is the keyboard lock of a browser or editor and
//! does not stop a resize. Cards in other workspaces are never read.
use super::Model;
use crate::cards::{auto_size, viewport_measured};
use crate::grid::{Rect, Size};
use crate::viewport::bounding_rect;
use crate::window_fit::{
    reflow, scale_block, sweep, window_size, Body, Placement, Variant, VARIANTS,
};

impl Model {
    /// Resizes the selected cards to the window's proportions by `variant`.
    pub fn resize_to_window(&mut self, variant: &Variant) {
        if self.selection.maximized {
            self.notify("restore the card first: it is maximised");
            return;
        }
        if !viewport_measured(self.view_size) {
            self.notify("the window has no size yet");
            return;
        }
        let aspect = self.view_size.w / self.view_size.h;
        let fill = auto_size(self.view_size, None);
        let selected: Vec<Body> = self
            .selected()
            .iter()
            .map(|c| Body {
                id: c.id.clone(),
                rect: c.rect,
                group: c.group_id.clone(),
            })
            .collect();
        if selected.is_empty() {
            return;
        }
        let fixed: Vec<Body> = self
            .here()
            .iter()
            .filter(|c| !selected.iter().any(|s| s.id == c.id))
            .map(|c| Body {
                id: c.id.clone(),
                rect: c.rect,
                group: c.group_id.clone(),
            })
            .collect();
        let sizes: Vec<Size> = selected
            .iter()
            .map(|b| {
                window_size(
                    Size {
                        w: b.rect.w,
                        h: b.rect.h,
                    },
                    aspect,
                    variant.size,
                    fill,
                )
            })
            .collect();
        let next: Vec<Body> = match variant.placement {
            Placement::Reflow => reflow(&selected, &sizes, &fixed, self.gap()),
            Placement::Block => {
                let block = scale_block(&selected, &sizes, self.gap());
                sweep(&block, &fixed, self.gap())
            }
        };
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
        for body in &next {
            if let Some(c) = self.card_mut(&body.id) {
                c.rect = body.rect;
                c.soft_group_id = None;
            }
        }
        self.dirty_layout = true;
        // The point of the command is to see the cards in the window's shape.
        let rects: Vec<Rect> = next.iter().map(|b| b.rect).collect();
        if let Some(bounds) = bounding_rect(&rects) {
            self.apply_viewport(self.fit_viewport(bounds));
        }
    }

    /// A row of the size picker that is a window variant.
    pub fn resize_to_window_row(&mut self, id: &str) -> bool {
        let Some(v) = VARIANTS.iter().find(|v| v.id == id) else {
            return false;
        };
        self.resize_to_window(v);
        true
    }
}

pub fn register(r: &mut crate::commands::CommandRegistry<Model>) {
    for v in VARIANTS {
        r.register(
            &format!("card.size.{}", v.id.replacen("window-", "window.", 1)),
            &format!("Card: {}", v.label),
            move |m| m.resize_to_window(v),
        );
    }
}
