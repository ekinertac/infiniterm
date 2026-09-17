//! What every command module shares: the active card, the selection, the
//! viewport moves, and the placement of a group's block. Port of
//! `commands/context.ts`.
//!
//! Every fit and jump-to-scale goes through `apply_viewport`, animated: a
//! cut is what loses your place on a canvas, since nothing connects where
//! you were to where you landed. `open_in_card` is the one path for `ift
//! <path>`, the placement menu and Cmd+click, so the three cannot drift.
use super::{Card, Effect, Model, NewCard};
use crate::cards::GUTTER;
use crate::grid::{snap_rect, Point, Rect, Size, HALF_CELL};
use crate::groups::{group_bounds, group_slot_size, GROUP_PAD};
use crate::ift::OpenPlan;
use crate::layout::{best_cols, first_free_slot};
use crate::navigate::{ensure_visible, REVEAL_PADDING};
use crate::saved_layout::CardKind;
use crate::viewport::{bounding_rect, fit_rect, Viewport};

/// Direction words that read as English in a command label: "Swap card
/// with the one to the up" is what the raw direction gives, and labels are
/// what the palette lists.
pub fn where_(dir: crate::navigate::Direction) -> &'static str {
    use crate::navigate::Direction::*;
    match dir {
        Left => "to the left",
        Right => "to the right",
        Up => "above",
        Down => "below",
    }
}

/// The label word for a ring step; `step_id` is the id suffix.
pub fn which(step: isize) -> &'static str {
    if step > 0 {
        "next"
    } else {
        "previous"
    }
}

pub fn step_id(step: isize) -> &'static str {
    if step > 0 {
        "next"
    } else {
        "prev"
    }
}

impl Model {
    /// The active card, unless maximised: moving or resizing a maximised
    /// card is invisible and would silently change where it lands on
    /// restore, so those commands are inert while maximised.
    pub fn with_active_card(&mut self, f: impl FnOnce(&mut Model, String)) {
        if self.selection.maximized {
            return;
        }
        if let Some(id) = self
            .selection
            .focused_id
            .clone()
            .filter(|id| self.card(id).is_some())
        {
            f(self, id);
        }
    }

    /// Like `with_active_card`, for commands that mean "every selected card".
    pub fn for_selected(&mut self, mut f: impl FnMut(&mut Model, String)) {
        if self.selection.maximized {
            return;
        }
        for id in self.selected_ids() {
            f(self, id);
        }
    }

    /// Where a card CARVED OUT of `from` starts: a split, or a card opened
    /// to work on what `from` is showing. `cards.inheritDirectory` turns it
    /// off for anyone who does not want it.
    ///
    /// Plain new cards do not come through here. They start at
    /// `startingDir`, because a new card is a new place to work, where a
    /// split is a statement about carrying on in this one.
    pub fn cwd_beside(&self, from: Option<&Card>) -> String {
        if self.config.cards.inherit_directory {
            if let Some(c) = from {
                return c.cwd.clone();
            }
        }
        self.start_dir.clone()
    }

    /// A url as typed: `example.com` is taken as https; anything with a
    /// scheme is left alone. Empty or the bare placeholder means cancel.
    pub fn normalise_url(typed: Option<&str>) -> Option<String> {
        let raw = typed.map(str::trim).unwrap_or("");
        if raw.is_empty() || raw == "https://" {
            return None;
        }
        Some(if raw.contains("://") {
            raw.to_string()
        } else {
            format!("https://{raw}")
        })
    }

    /// Scrolls the focused card into view if it is off screen, animated;
    /// hops between neighbours you can see cost no motion. While framing,
    /// re-fits on the card instead.
    pub fn reveal_focused(&mut self) {
        let Some(card) = self.focused().cloned() else {
            return;
        };
        let rect = card.rect;
        if self.framing {
            // The slot, not the half: a split moves focus to the new piece
            // and the frame must not follow it down into half the space.
            let bounds = self
                .slot_bounds(&[rect], std::slice::from_ref(&card.soft_group_id))
                .unwrap_or(rect);
            self.frame_card(bounds);
            return;
        }
        let next = ensure_visible(rect, self.viewport, self.view_size, REVEAL_PADDING);
        if next.x != self.viewport.x || next.y != self.viewport.y {
            self.effects.push(Effect::AnimatePan {
                x: next.x,
                y: next.y,
            });
        }
    }

    /// Any canvas-level zoom is a statement about the canvas, so it drops out
    /// of maximise rather than changing something you cannot see.
    pub fn apply_viewport(&mut self, next: Viewport) {
        self.selection.maximized = false;
        self.framing = false;
        self.effects.push(Effect::AnimateFit(next));
    }

    /// Fits the view on one rect and keeps framing: arrows re-fit from here.
    /// The SLOT a selection sits in: the cards themselves, plus the halves
    /// they were split from, as long as those pieces still add up to about
    /// one card's worth of space.
    ///
    /// Splitting a card while framing used to fit the new half, which is
    /// half a slot: the view dropped onto the bottom piece and the top one
    /// went off screen. A split does not move you somewhere else, it
    /// subdivides where you already are, so the frame stays on the slot.
    ///
    /// The size check is what keeps it honest. A half dragged across the
    /// canvas is no longer part of a slot, and framing the pair would zoom
    /// out to nothing.
    pub fn slot_bounds(&self, cards: &[Rect], soft_groups: &[Option<String>]) -> Option<Rect> {
        let mut rects: Vec<Rect> = cards.to_vec();
        for group in soft_groups.iter().flatten() {
            for card in &self.cards {
                if card.soft_group_id.as_deref() == Some(group.as_str())
                    && !rects.contains(&card.rect)
                {
                    rects.push(card.rect);
                }
            }
        }
        let widened = bounding_rect(&rects)?;
        let one = self.default_size();
        // Still one slot? Then the pieces belong together.
        if widened.w <= one.w + GUTTER && widened.h <= one.h + GUTTER {
            return Some(widened);
        }
        bounding_rect(cards)
    }

    pub fn frame_card(&mut self, rect: Rect) {
        self.apply_viewport(fit_rect(rect, self.view_size));
        self.framing = true;
    }

    pub fn fit_group(&mut self, group_id: &str) {
        let rects: Vec<Rect> = self
            .cards
            .iter()
            .filter(|c| c.group_id.as_deref() == Some(group_id))
            .map(|c| c.rect)
            .collect();
        if let Some(bounds) = group_bounds(&rects, GROUP_PAD) {
            self.apply_viewport(fit_rect(bounds, self.view_size));
        }
    }

    /// Opens what a path resolved to beside `from`, in its group and on its
    /// canvas, exactly as a new card would. Returns the new card's id, or
    /// nothing for a refused plan (the reason goes to the notice).
    pub fn open_in_card(&mut self, plan: OpenPlan, from: Option<&str>) -> Option<String> {
        let from = from.and_then(|id| self.card(id)).cloned();
        let workspace_id = from
            .as_ref()
            .map(|c| c.workspace_id.clone())
            .or_else(|| self.active_workspace.clone())
            .unwrap_or_default();
        let (kind, cwd, path, root, line, url) = match plan {
            OpenPlan::Editor {
                cwd,
                path,
                root,
                line,
            } => (CardKind::Editor, cwd, path, root, line, None),
            OpenPlan::Diff { cwd, path, root } => {
                (CardKind::Diff, cwd, path, Some(root), None, None)
            }
            OpenPlan::Browser { cwd, url } => (CardKind::Browser, cwd, None, None, None, Some(url)),
            OpenPlan::Transcript { cwd, path } => {
                (CardKind::Transcript, cwd, Some(path), None, None, None)
            }
            OpenPlan::Refused { text } => {
                self.notify(text);
                return None;
            }
        };
        let group_id = from.as_ref().and_then(|c| c.group_id.clone());
        let opts = NewCard {
            kind,
            // An editor on a directory shows its tree; a diff always shows its list.
            explorer: root.is_some() && (kind == CardKind::Diff || path.is_none()),
            path,
            root,
            line,
            url,
            // A turn list reads like an inbox: a line per turn wants the
            // width, and the answer under it wants the rest.
            sidebar_top: kind == CardKind::Transcript,
            avoid: self.other_frames(group_id.as_deref(), &workspace_id),
            group_id,
            workspace_id: Some(workspace_id),
            after: from.as_ref().map(|c| c.rect),
            ..Default::default()
        };
        let id = self.add_card(&cwd, opts);
        self.selection.maximized = false;
        self.set_focus(Some(&id));
        self.reveal_focused();
        Some(id)
    }

    /// Moves a card, or a selection as one block, into open canvas with room
    /// around it for its group to grow. The one deliberate exception to
    /// positions being permanent: making room for a group means moving
    /// something. Groups live to the RIGHT of the loose grid, never in it,
    /// measured from the grid so the first group lands in the same place
    /// whether there are two loose cards or ten.
    pub fn move_to_free_block(&mut self, moving: &[String]) {
        let movers: Vec<Card> = moving
            .iter()
            .filter_map(|id| self.card(id).cloned())
            .collect();
        let Some(card) = movers.first() else { return };
        let size = self.default_size();
        let reserve = group_slot_size(size, GUTTER, GROUP_PAD);
        let bounds =
            bounding_rect(&movers.iter().map(|c| c.rect).collect::<Vec<_>>()).expect("movers");
        let block = Size {
            w: reserve.w.max(bounds.w + GROUP_PAD * 2.),
            h: reserve.h.max(bounds.h + GROUP_PAD * 2.),
        };
        let mut taken: Vec<Rect> = self
            .cards
            .iter()
            .filter(|c| c.workspace_id == card.workspace_id && !moving.contains(&c.id))
            .map(|c| c.rect)
            .collect();
        taken.extend(self.other_frames(card.group_id.as_deref(), &card.workspace_id));
        // Right of EVERYTHING that is already there, measured rather than
        // assumed. This used to start at the right edge of a hypothetical
        // twelve-card grid, which was true while placement used a fixed
        // column count; since cards take the nearest free slot they spread
        // as far as they like, and a group would land in a gap inside the
        // loose cluster instead of clear of it.
        let cluster_right = taken.iter().map(|r| r.x + r.w).fold(HALF_CELL, f64::max);
        // A wider gap than between two cards, so the groups read as a region
        // of their own rather than as more cards.
        let group_gap = GUTTER * 4.;
        let spot = first_free_slot(
            &taken,
            block,
            Point {
                x: cluster_right + group_gap,
                y: HALF_CELL,
            },
            GUTTER,
            best_cols(
                crate::cards::TYPICAL_CARDS,
                block.w,
                block.h,
                GUTTER,
                self.view_size.w,
                self.view_size.h,
            ),
            10_000,
            0,
        );
        // Inset by the frame padding, so the block is the FRAME's footprint.
        // Every mover shifts by the same delta, keeping the selection's shape.
        let (dx, dy) = (spot.x + GROUP_PAD - bounds.x, spot.y + GROUP_PAD - bounds.y);
        for id in moving {
            if let Some(c) = self.card_mut(id) {
                c.rect = snap_rect(Rect {
                    x: c.rect.x + dx,
                    y: c.rect.y + dy,
                    ..c.rect
                });
            }
        }
        self.dirty_layout = true;
    }
}
