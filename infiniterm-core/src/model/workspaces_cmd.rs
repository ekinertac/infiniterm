//! Workspace commands. Switching never touches a card: everything stays
//! with its shell running, because an inactive workspace is a card scrolled
//! off screen, just further off. Port of `commands/workspaces.ts`.
use super::palette_state::Source;
use super::{Model, NewCard, Pending};
use crate::workspaces::{after_closing, step_workspace};

/// The `MoveTo` row that makes a workspace to move into. Not a workspace
/// id: those are uuids.
pub const NEW_WORKSPACE_ROW: &str = "new";

impl Model {
    /// The selection (else the focused card) to workspace `to`, or a new
    /// one: each card keeps its size and takes the destination's first free
    /// block slot (Cmd+T's rule, `layout::block_slot`), in the order you
    /// read them, so none lands on a card already there. They leave their
    /// groups, which belong to one workspace. You stay where you are: a
    /// move is usually tidying (Ekin, 2026-09-25); the destination
    /// remembers the first card as its focus, so going there lands on it.
    pub fn move_selection_to(&mut self, to: &str) {
        let here = self.active_workspace.clone().unwrap_or_default();
        let mut ids = self.selected_ids();
        if ids.is_empty() {
            return;
        }
        // An in-place editor goes with the terminal it covers.
        let bases: Vec<String> = ids
            .iter()
            .filter_map(|id| self.covers.get(id).cloned())
            .collect();
        ids.extend(bases);
        let to = if to == NEW_WORKSPACE_ROW {
            self.add_workspace(None)
        } else if self.workspaces.iter().any(|w| w.id == to) && to != here {
            to.to_string()
        } else {
            return;
        };
        let rects: Vec<crate::grid::Rect> = ids
            .iter()
            .filter_map(|id| self.card(id).map(|c| c.rect))
            .collect();
        let order = crate::workspaces::reading_order(&rects);
        let mut placed: Vec<crate::grid::Rect> =
            self.cards_on(Some(&to)).iter().map(|c| c.rect).collect();
        let mut moved: Vec<String> = vec![];
        for i in order {
            let id = ids[i].clone();
            // A covered terminal follows its cover's rect below.
            if self.covered_by(&id).is_some() {
                continue;
            }
            let Some(card) = self.card(&id).cloned() else {
                continue;
            };
            let size = crate::grid::Size {
                w: card.rect.w,
                h: card.rect.h,
            };
            let origin = crate::grid::Point {
                x: crate::grid::HALF_CELL,
                y: crate::grid::HALF_CELL,
            };
            let rect = crate::layout::block_slot(&placed, size, origin, crate::cards::GUTTER);
            placed.push(rect);
            for mover in std::iter::once(id.clone()).chain(self.covers.get(&id).cloned()) {
                if let Some(c) = self.card_mut(&mover) {
                    c.workspace_id = to.clone();
                    c.rect = rect;
                    c.group_id = None;
                    c.soft_group_id = None;
                    c.split_from = None;
                }
                if let Some(at) = self.cover_at.get_mut(&mover) {
                    *at = rect;
                }
            }
            moved.push(id);
        }
        if moved.is_empty() {
            return;
        }
        self.prune_empty_groups();
        if let Some(w) = self.workspaces.iter_mut().find(|w| w.id == to) {
            w.focused = moved.first().cloned();
        }
        // The focus stays here: the card you were on before, else none.
        let back = self
            .focus_trail
            .iter()
            .rev()
            .find(|t| self.card(t).is_some_and(|c| c.workspace_id == here))
            .cloned();
        self.set_focus(back.as_deref());
        let name = self
            .workspaces
            .iter()
            .find(|w| w.id == to)
            .map(|w| w.name.clone())
            .unwrap_or_default();
        let what = match moved.as_slice() {
            [one] => self
                .card(one)
                .map(|c| format!("#{}", c.number))
                .unwrap_or_default(),
            many => format!("{} cards", many.len()),
        };
        self.notify(format!("moved {what} to {name}"));
        self.dirty_layout = true;
    }

    fn workspace_ids(&self) -> Vec<String> {
        self.workspaces.iter().map(|w| w.id.clone()).collect()
    }

    /// The confirmed half of `workspace.close`: cards go with it, shells and
    /// all, since a card on no workspace is invisible and still running.
    pub fn close_workspace_confirmed(&mut self, id: &str) {
        // A locked card holds its workspace too: removing the workspace
        // and refusing the card would leave the card on no canvas.
        if self
            .cards
            .iter()
            .any(|c| c.workspace_id == id && c.protected)
        {
            self.notify("a locked card is in this workspace: Cmd+Shift+L on it first");
            return;
        }
        let ids = self.workspace_ids();
        let next = after_closing(&ids, id).map(String::from);
        for card in self.remove_workspace(id) {
            self.close_card(&card, false);
        }
        if let Some(next) = next {
            self.active_workspace = None; // force a switch rather than a no-op
            self.show_workspace(&next);
        }
    }
}

pub fn register(r: &mut crate::commands::CommandRegistry<Model>) {
    use super::context::{step_id, which};
    for step in [1isize, -1] {
        r.register(
            &format!("workspace.{}", step_id(step)),
            &format!("Workspace: go to the {}", which(step)),
            move |m| {
                let ids = m.workspace_ids();
                if let Some(id) =
                    step_workspace(&ids, m.active_workspace.as_deref(), step).map(String::from)
                {
                    m.show_workspace(&id);
                }
            },
        );
    }
    // Straight to the Nth workspace; nine, because that is how many a number
    // row has. Does nothing when there is no Nth, rather than creating one.
    for n in 1..=9usize {
        r.register(
            &format!("workspace.show.{n}"),
            &format!("Workspace: go to {n}"),
            move |m| {
                if let Some(id) = m.workspaces.get(n - 1).map(|w| w.id.clone()) {
                    m.show_workspace(&id);
                }
            },
        );
    }
    // Numbered, not prompted: a workspace is made far more often than named.
    // Opens with one terminal, like the app itself.
    r.register("workspace.new", "Workspace: new", |m| {
        let ws = m.add_workspace(None);
        m.show_workspace(&ws);
        let start = m.start_dir.clone();
        let id = m.add_card(
            &start,
            NewCard {
                workspace_id: Some(ws),
                ..Default::default()
            },
        );
        m.set_focus(Some(&id));
        m.reveal_focused();
    });
    r.register("workspace.rename", "Workspace: rename", |m| {
        let Some(ws) = m.active_ws().cloned() else {
            return;
        };
        m.prompt
            .ask("workspace name", &ws.name, Pending::RenameWorkspace(ws.id));
    });
    // The last workspace stays. Asked first, because this is the one key
    // that kills several shells at once; an empty workspace goes unasked.
    r.register("workspace.close", "Workspace: close", |m| {
        let Some(ws) = m.active_ws().cloned().filter(|_| m.workspaces.len() >= 2) else {
            m.notify("the last workspace stays");
            return;
        };
        let count = m.cards_on(Some(&ws.id)).len();
        if count == 0 {
            m.close_workspace_confirmed(&ws.id);
            return;
        }
        let label = format!(
            "close \"{}\" and its {count} card{}?",
            ws.name,
            if count == 1 { "" } else { "s" }
        );
        m.prompt
            .confirm(&label, "Close workspace", Pending::CloseWorkspace(ws.id));
    });
    // Moving a card to another workspace is a palette job (Ekin,
    // 2026-09-25: "a command center action"): pick this, then the
    // workspace. It replaced workspace.move.next/prev, which had no key,
    // followed the card, and kept its rect, so it could land on a card.
    r.register("card.moveToWorkspace", "Card: move to workspace…", |m| {
        if m.selection.focused_id.is_some() {
            m.open_palette(Source::MoveTo);
        }
    });
}
