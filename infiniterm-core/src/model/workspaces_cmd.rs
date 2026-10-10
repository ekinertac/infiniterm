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
    /// Puts workspace `id` at tab position `to` (`workspaces::reordered`),
    /// the order the tabs, `workspace.show.N` and Ctrl+digit read. The saved
    /// layout writes the order, so it stays.
    pub fn move_workspace(&mut self, id: &str, to: usize) {
        let order: Vec<String> = self.workspaces.iter().map(|w| w.id.clone()).collect();
        let Some(next) = crate::workspaces::reordered(&order, id, to) else {
            return;
        };
        let mut old = std::mem::take(&mut self.workspaces);
        self.workspaces = next
            .iter()
            .filter_map(|n| {
                old.iter()
                    .position(|w| &w.id == n)
                    .map(|i| old.swap_remove(i))
            })
            .collect();
        self.dirty_layout = true;
    }

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
            let rect = crate::layout::block_slot(&placed, size, origin, self.gap());
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

    /// The focused card's whole group to workspace `to`, or a new one: the
    /// frame, the name and the cards in their arrangement and sizes, moved
    /// as one block (#348). The block takes the destination's first free
    /// slot of its size (`layout::block_slot`, as a card does), frame pad
    /// included. A group is all or nothing: one card of it focused or
    /// selected moves the lot. You stay where you are.
    pub fn move_group_to(&mut self, to: &str) {
        let here = self.active_workspace.clone().unwrap_or_default();
        let Some(group) = self.focused().and_then(|c| c.group_id.clone()) else {
            self.notify("the focused card is in no group");
            return;
        };
        let mut ids: Vec<String> = self
            .cards
            .iter()
            .filter(|c| c.group_id.as_deref() == Some(group.as_str()) && c.workspace_id == here)
            .map(|c| c.id.clone())
            .collect();
        // An in-place editor goes with the terminal it covers.
        let bases: Vec<String> = ids
            .iter()
            .filter_map(|id| self.covers.get(id).cloned())
            .filter(|b| !ids.contains(b))
            .collect();
        ids.extend(bases);
        let Some(frame) = self.group_frame(&group, &here) else {
            return;
        };
        let to = if to == NEW_WORKSPACE_ROW {
            self.add_workspace(None)
        } else if self.workspaces.iter().any(|w| w.id == to) && to != here {
            to.to_string()
        } else {
            return;
        };
        let mut placed: Vec<crate::grid::Rect> =
            self.cards_on(Some(&to)).iter().map(|c| c.rect).collect();
        placed.extend(self.other_frames(None, &to));
        let origin = crate::grid::Point {
            x: crate::grid::HALF_CELL,
            y: crate::grid::HALF_CELL,
        };
        let size = crate::grid::Size {
            w: frame.w,
            h: frame.h,
        };
        let slot = crate::layout::block_slot(&placed, size, origin, self.gap());
        let (dx, dy) = (slot.x - frame.x, slot.y - frame.y);
        for id in &ids {
            if let Some(c) = self.card_mut(id) {
                c.workspace_id = to.clone();
                c.rect.x += dx;
                c.rect.y += dy;
            }
            if let Some(at) = self.cover_at.get_mut(id) {
                at.x += dx;
                at.y += dy;
            }
        }
        if let Some(w) = self.workspaces.iter_mut().find(|w| w.id == to) {
            w.focused = self.selection.focused_id.clone();
        }
        let back = self
            .focus_trail
            .iter()
            .rev()
            .find(|t| self.card(t).is_some_and(|c| c.workspace_id == here))
            .cloned();
        self.set_focus(back.as_deref());
        let (name, group_name) = (
            self.workspaces
                .iter()
                .find(|w| w.id == to)
                .map(|w| w.name.clone())
                .unwrap_or_default(),
            self.group(&group)
                .map(|g| g.name.clone())
                .unwrap_or_default(),
        );
        self.notify(format!("moved group \"{group_name}\" to {name}"));
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
    // The tab row's order, from the keyboard as well as by dragging a tab.
    for (name, label, step) in [
        ("left", "Workspace: move this tab left", -1_isize),
        ("right", "Workspace: move this tab right", 1),
    ] {
        r.register(&format!("workspace.reorder.{name}"), label, move |m| {
            let Some(id) = m.active_workspace.clone() else {
                return;
            };
            let Some(at) = m.workspaces.iter().position(|w| w.id == id) else {
                return;
            };
            m.move_workspace(&id, (at as isize + step).max(0) as usize);
        });
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
    r.register("workspace.rename", "Workspace: rename…", |m| {
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
    r.register(
        "group.moveToWorkspace",
        "Group: move to workspace…",
        |m| {
            if m.focused().is_some_and(|c| c.group_id.is_some()) {
                m.open_palette(Source::MoveGroupTo);
            } else if m.selection.focused_id.is_some() {
                m.notify("the focused card is in no group");
            }
        },
    );
}
