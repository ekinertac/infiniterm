//! "Resize to window" (`card.size.window`, #585): the selected cards keep
//! their width and take the window's aspect ratio, reflowed so nothing
//! overlaps. "Default size" (`card.size.default`): the selected cards take the
//! default card size, same reflow. "Custom size…" (`card.size.custom`) asks
//! for a width and height in grid squares (`custom_size.rs` parses) and applies
//! them the same way. All are the last rows of the `card.size` picker
//! (Cmd+Alt+S), so the palette, the keymap and `ift run` reach them.
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
pub const CUSTOM_ROW: (&str, &str) = ("custom", "Custom size…");
/// The prompt's title; the unit is the one of `cards.width`.
pub const CUSTOM_PROMPT: &str = "Custom size: width x height in grid squares";
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
    /// True when cards moved or changed size.
    fn reflow_selection(&mut self, target: Option<Target>, unchanged: &str) -> bool {
        if self.selection.maximized {
            self.notify("restore the card first: it is maximised");
            return false;
        }
        let Some(target) = target else {
            self.notify("the window has no size yet");
            return false;
        };
        let body = |c: &super::Card| Body {
            id: c.id.clone(),
            rect: c.rect,
            group: c.group_id.clone(),
        };
        let selected: Vec<Body> = self.selected().iter().map(|c| body(c)).collect();
        if selected.is_empty() {
            return false;
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
            return false;
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
        true
    }

    /// "Custom size…": asks for width and height in grid squares, starting
    /// from the last custom size or else the default size.
    pub fn ask_custom_size(&mut self) {
        if self.selection.maximized {
            self.notify("restore the card first: it is maximised");
            return;
        }
        if self.selection.focused_id.is_none() {
            return;
        }
        let start = self
            .custom_size_last
            .map(crate::custom_size::squares)
            .unwrap_or_else(|| crate::custom_size::squares(self.default_size()));
        self.prompt
            .ask(CUSTOM_PROMPT, &start, super::Pending::CustomSize);
    }

    /// The prompt's answer. Bad text asks again with what was typed and
    /// says why (the prompt has no line of its own for it); a size applies
    /// to the selection like "Default size", and the notice gives the
    /// pixels, because the unit typed is a grid square.
    pub fn answer_custom_size(&mut self, text: Option<String>) {
        let Some(text) = text else { return };
        let parsed = match crate::custom_size::parse(&text) {
            Ok(p) => p,
            Err(why) => {
                self.prompt
                    .ask(CUSTOM_PROMPT, text.trim(), super::Pending::CustomSize);
                self.notify(why);
                return;
            }
        };
        self.custom_size_last = Some(parsed.size);
        let shown = crate::custom_size::squares(parsed.size);
        if self.reflow_selection(Some(Target::Size(parsed.size)), "already that size") {
            let px = format!("{} x {} px", parsed.size.w, parsed.size.h);
            self.notify(if parsed.clamped {
                format!("{shown} squares ({px}): the minimum card, what you typed was smaller")
            } else {
                format!("{shown} squares ({px})")
            });
        }
    }
}

pub fn register(r: &mut crate::commands::CommandRegistry<Model>) {
    r.register(
        "card.size.custom",
        &format!("Card: {}", CUSTOM_ROW.1),
        |m| m.ask_custom_size(),
    );
    r.register("card.size.window", "Card: resize to window", |m| {
        m.resize_to_window()
    });
    r.register("card.size.default", DEFAULT_ROW.1, |m| {
        m.reset_selection_to_default_size()
    });
}
