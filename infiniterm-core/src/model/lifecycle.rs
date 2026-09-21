//! Closing a card, from wherever it is closed. Port of
//! `cardLifecycle.svelte.ts`.
//!
//! Two ways and they must do the same thing: the close command, and the
//! shell itself exiting (`exit`, Ctrl+D). The second once had no handler and
//! left a dead card printing `[exited 0]`. The split partners flush against
//! the closed card take its space (`reclaim`), focus goes to the card that
//! took the space, else the card focused BEFORE this one (`focus_trail`),
//! which is where a person expects to land after closing an editor they
//! opened from a terminal, else the nearest by geometry: the list is
//! creation order, which on a rearranged canvas is no order at all.
use super::Card;
use super::{Effect, Model};
use crate::cards::{CardRect, GUTTER};
use crate::grid::Rect;
use crate::navigate::nearest_to;
use crate::saved_layout::CardKind;

/// Cells, not pixels: what a person reads a layout in.
fn rect_of(r: Rect) -> String {
    format!(
        "{},{} {}x{}",
        (r.x / 25.).round(),
        (r.y / 25.).round(),
        (r.w / 25.).round(),
        (r.h / 25.).round()
    )
}

fn short(id: &str) -> &str {
    &id[..id.len().min(8)]
}

/// How many closed cards can be reopened. Deep enough to undo a mistake,
/// shallow enough that it is an undo and not a graveyard.
pub const CLOSED_RING: usize = 10;

impl Model {
    /// `already_exited` says the shell is gone, so there is nothing to kill.
    pub fn close_card(&mut self, id: &str, already_exited: bool) {
        self.close_card_with(id, already_exited, true);
    }

    /// `reclaim` false leaves the closed card's space free instead of handing
    /// it to the split partner (`card.close.leave`); the soft group is still
    /// dropped when one card is left of it, so the survivor's own Cmd+W
    /// later does not look for a partner that is gone.
    pub fn close_card_with(&mut self, id: &str, already_exited: bool, reclaim: bool) {
        let Some(card) = self.card(id).cloned() else {
            return;
        };
        // A protected card is not closed by anything: a shell that exited
        // is replaced (the pane goes, `terminals.rs` spawns a fresh one),
        // a close is refused with the way out named.
        if card.protected {
            if already_exited {
                if let Some(c) = self.card_mut(id) {
                    c.pane_id = None;
                }
            } else {
                self.notify("card is locked: Cmd+Shift+L to unlock it");
            }
            return;
        }

        // Remembered before anything is torn down, so Cmd+Ctrl+T can put it
        // back. Runtime facts are stripped for the same reason the save file
        // strips them: a restored card is a fresh shell in the same
        // directory, never a resurrected process claiming to be working.
        self.closed.push(Card {
            pane_id: None,
            agent: crate::agent_state::AgentState::None,
            dirty: false,
            command: None,
            transcript_path: None,
            ..card.clone()
        });
        if self.closed.len() > CLOSED_RING {
            self.closed.remove(0);
        }
        // And a step on the undo trail, unless an undo is what closes it.
        if let Some(kept) = self.closed.last().cloned() {
            self.record_undo(super::UndoStep::Closed(Box::new(kept)));
        }

        // Kill the PTY explicitly: dropping the body would leave the shell
        // running with nothing reading it.
        if let (false, Some(pane)) = (already_exited, card.pane_id) {
            self.effects.push(Effect::KillPane(pane));
        }
        // Closing is the one act that means "discard": the unsaved buffer
        // kept for a quit is not kept for this.
        if card.kind == CardKind::Editor {
            self.effects.push(Effect::DraftDelete(card.id.clone()));
        }

        // Only soft-group cards qualify, and only while they still tile the
        // edge exactly; see split.rs.
        let siblings: Vec<CardRect> = match &card.soft_group_id {
            Some(sg) => self
                .cards
                .iter()
                .filter(|c| c.id != id && c.soft_group_id.as_ref() == Some(sg))
                .map(|c| CardRect {
                    id: c.id.clone(),
                    rect: c.rect,
                })
                .collect(),
            None => vec![],
        };
        // The partner: what this card was split from, else the last card
        // split from it. Only a hint; reclaim checks it still tiles the edge.
        let partner = siblings
            .iter()
            .find(|s| Some(&s.id) == card.split_from.as_ref())
            .map(|s| s.id.clone())
            .or_else(|| {
                self.cards
                    .iter()
                    .rev()
                    .find(|c| {
                        siblings.iter().any(|s| s.id == c.id) && c.split_from.as_deref() == Some(id)
                    })
                    .map(|c| c.id.clone())
            });
        let grown = if reclaim {
            crate::split::reclaim(card.rect, &siblings, GUTTER, partner.as_deref())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        self.log(format!(
            "close {} {}rect={} soft={} siblings={} grown={}",
            short(&card.id),
            if already_exited { "(exited) " } else { "" },
            rect_of(card.rect),
            card.soft_group_id.as_deref().map_or("-", short),
            siblings.len(),
            if grown.is_empty() {
                "none".to_string()
            } else {
                grown
                    .iter()
                    .map(|g| format!("{}->{}", short(&g.id), rect_of(g.rect)))
                    .collect::<Vec<_>>()
                    .join(",")
            }
        ));
        if !grown.is_empty() {
            let grown_ids: Vec<String> = grown.iter().map(|g| g.id.clone()).collect();
            self.mark_swap(&grown_ids);
        }
        for g in &grown {
            if let Some(sibling) = self.card_mut(&g.id) {
                sibling.rect = g.rect;
            }
        }
        // A soft group of one is nothing; clearing it keeps a later neighbour
        // from being mistaken for a partner.
        if siblings.len() == 1 {
            if let Some(only) = self.card_mut(&siblings[0].id) {
                only.soft_group_id = None;
            }
        }
        // The card that took the space inherits the closed card's parent
        // link, so a later close still finds its way up.
        for g in &grown {
            if let Some(sibling) = self.card_mut(&g.id) {
                if sibling.split_from.as_deref() == Some(id) {
                    sibling.split_from = card.split_from.clone();
                }
            }
        }

        // The cards on the SAME canvas, for handing focus on.
        let here: Vec<crate::cards::PlacedCard> = self
            .cards
            .iter()
            .filter(|c| c.workspace_id == card.workspace_id && c.id != id)
            .map(|c| crate::cards::PlacedCard {
                id: c.id.clone(),
                rect: c.rect,
                group_id: c.group_id.clone(),
            })
            .collect();
        self.remove_card(id);
        // Closing the last card in a group closes the group, the way closing
        // the last pane in a tab closes the tab.
        self.prune_empty_groups();

        // Only when the card that closed was the active one: a shell exiting
        // in some other card must not steal focus from what you are in.
        if self.selection.focused_id.as_deref() == Some(id) {
            self.selection.maximized = false;
            let before = self
                .focus_trail
                .iter()
                .rev()
                .find(|t| here.iter().any(|c| &c.id == *t))
                .cloned();
            let next = grown
                .first()
                .map(|g| g.id.clone())
                .or(before)
                .or_else(|| nearest_to(&here, card.rect).map(|c| c.id.clone()));
            self.set_focus(next.as_deref());
            // The card focused before may be off screen; the closed one was
            // in view, so the eye needs to be taken there.
            self.reveal_focused();
        } else {
            self.selection.extra.retain(|e| e != id);
        }
    }
}
