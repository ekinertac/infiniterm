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

/// How long a closed terminal card's program must be QUIET before it is
/// ended: no output, nothing under its shell on the CPU (`inspect` `busy`),
/// no agent turn running. A closed card is watched, not timed (Ekin,
/// 2026-09-29): a five-minute build closed after one minute runs to the end,
/// an idle dev server or shell goes after this, and an agent WAITING for
/// you is kept until the app quits (it is unfinished work, and Alt+T
/// brings it back). Ekin's mouse had Cmd+W on a thumb button and closed
/// running agents; a fixed minute (ten on the first day) killed builds
/// closed on purpose.
pub const PARK_MS: f64 = 60. * 1000.;

/// A closed terminal card whose program is still running, watched until
/// it is quiet. The pane stays attached, so its output keeps being read
/// (and thrown away: a noisy build must not stall on a full buffer).
#[derive(Clone, Debug)]
pub struct Parked {
    /// The card as it was closed, its agent state kept current by hooks.
    pub card: Card,
    pub pane: crate::backend::PaneId,
    /// The last sign of work: output, CPU, an agent turn. Starts at the
    /// close.
    pub active_at: f64,
}

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

        // An in-place editor closing is `:wq`, not a card going away: no
        // reopen ring, no undo step, no partner reclaiming its space (the
        // terminal under it already has it).
        let covered = self.covers.get(id).cloned();
        self.end_cover(id);
        if let Some(base) = covered {
            self.effects.push(Effect::DraftDelete(card.id.clone()));
            self.remove_card(id);
            // Back to the terminal it covered, as vim hands the screen back.
            if self.card(&base).is_some() {
                self.set_focus(Some(&base));
            }
            return;
        }
        // A terminal's session is PARKED rather than killed: it keeps
        // running for `PARK_MS`, and a reopen in that time adopts it, the
        // program still mid-work (`end_parked` kills it after).
        let parked = match (already_exited, card.pane_id, &card.session) {
            (false, Some(pane), Some(_)) if self.can_park && card.kind == CardKind::Terminal => {
                self.parked.push(Parked {
                    card: card.clone(),
                    pane,
                    active_at: self.now_ms,
                });
                true
            }
            _ => false,
        };
        // Remembered before anything is torn down, so Cmd+Shift+T can put it
        // back. Runtime facts are stripped for the same reason the save file
        // strips them: a restored card is a fresh shell in the same
        // directory, never a resurrected process claiming to be working.
        // A parked card keeps its agent's state and transcript: its process
        // IS still running and comes back with it.
        self.closed.push(Card {
            pane_id: None,
            agent: if parked {
                card.agent
            } else {
                crate::agent_state::AgentState::None
            },
            dirty: false,
            command: None,
            transcript_path: if parked {
                card.transcript_path.clone()
            } else {
                None
            },
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
        if let (false, false, Some(pane)) = (parked, already_exited, card.pane_id) {
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
            self.selection.maximized = self.maximise_sticky;
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

impl Model {
    /// A parked card that has been quiet `PARK_MS` is ended; one whose
    /// agent is mid-turn is busy; one whose agent waits for you is kept.
    /// Run from `tick`.
    pub(super) fn end_parked(&mut self) {
        use crate::agent_state::AgentState;
        let now = self.now_ms;
        let mut ended = vec![];
        self.parked.retain_mut(|p| match p.card.agent {
            AgentState::Waiting => true,
            AgentState::Working => {
                p.active_at = now;
                true
            }
            _ if now - p.active_at >= PARK_MS => {
                ended.push((p.pane, p.card.id.clone()));
                false
            }
            _ => true,
        });
        for (pane, id) in ended {
            self.effects.push(Effect::KillPane(pane));
            self.log(format!(
                "ended closed card {} after {} s quiet",
                short(&id),
                PARK_MS / 1000.
            ));
        }
    }

    /// Output from a parked card's pane: a sign of work. An exit means the
    /// program is gone and there is nothing left to end.
    pub(super) fn parked_pane_event(
        &mut self,
        pane: crate::backend::PaneId,
        event: &crate::backend::PaneEvent,
    ) {
        use crate::backend::PaneEvent;
        let now = self.now_ms;
        match event {
            PaneEvent::Output(_) => {
                if let Some(p) = self.parked.iter_mut().find(|p| p.pane == pane) {
                    p.active_at = now;
                }
            }
            PaneEvent::Exited { .. } => self.parked.retain(|p| p.pane != pane),
            _ => {}
        }
    }

    /// The closed card an agent is waiting in, newest first: what the
    /// status bar names and Alt+T reopens.
    pub fn waiting_parked(&self) -> Option<&Card> {
        self.parked
            .iter()
            .rev()
            .map(|p| &p.card)
            .find(|c| c.agent == crate::agent_state::AgentState::Waiting)
    }

    /// Alt+T while a closed card waits for you: that card comes back.
    pub fn reopen_waiting(&mut self) -> bool {
        let Some(card) = self.waiting_parked().cloned() else {
            return false;
        };
        self.closed.retain(|c| c.id != card.id);
        self.reopen_card(card);
        true
    }

    /// A reopened card takes its parked program back: the pane is let go
    /// and adopted again by the new body (`Effect::ReleasePane`), and the
    /// card wears the state the program has now.
    pub(super) fn unpark(&mut self, card: &mut Card) {
        let Some(i) = self.parked.iter().position(|p| p.card.id == card.id) else {
            return;
        };
        let p = self.parked.remove(i);
        card.agent = p.card.agent;
        card.agent_session = p.card.agent_session.clone();
        card.agent_kind = p.card.agent_kind.clone();
        card.transcript_path = p.card.transcript_path.clone();
        self.effects.push(Effect::ReleasePane(p.pane));
    }
}
