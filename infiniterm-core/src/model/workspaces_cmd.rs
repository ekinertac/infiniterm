//! Workspace commands. Switching never touches a card: everything stays
//! with its shell running, because an inactive workspace is a card scrolled
//! off screen, just further off. Port of `commands/workspaces.ts`.
use super::{Model, NewCard, Pending};
use crate::workspaces::{after_closing, step_workspace};

impl Model {
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
    // Follow the card: sending it somewhere you cannot see reads as losing it.
    for step in [1isize, -1] {
        r.register(
            &format!("workspace.move.{}", step_id(step)),
            &format!("Workspace: move card to the {}", which(step)),
            move |m| {
                m.with_active_card(|m, id| {
                    let ids = m.workspace_ids();
                    let Some(to) =
                        step_workspace(&ids, m.active_workspace.as_deref(), step).map(String::from)
                    else {
                        return;
                    };
                    if m.card(&id).is_some_and(|c| c.workspace_id == to) {
                        return;
                    }
                    if let Some(c) = m.card_mut(&id) {
                        c.workspace_id = to.clone();
                    }
                    m.show_workspace(&to);
                    m.set_focus(Some(&id));
                })
            },
        );
    }
}
