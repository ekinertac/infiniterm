//! Group commands: iTerm2's Tabs > Panes without the splitting. Port of
//! `commands/groups.ts`. The group rings are in `focus_cmd.rs` with the
//! other rings.
use super::{Model, Pending};
use crate::groups::step_ring;

impl Model {
    fn group_of_active(&self) -> Option<String> {
        self.focused().and_then(|c| c.group_id.clone())
    }
}

pub fn register(r: &mut crate::commands::CommandRegistry<Model>) {
    use super::context::{step_id, which};
    r.register("group.new", "Group: the selection", |m| {
        let ids = m.selected_ids();
        let Some(first) = ids.first().and_then(|id| m.card(id)).cloned() else {
            return;
        };
        let label = m.label_of(&first);
        m.prompt.ask("group name", &label, Pending::NameGroup(ids));
    });
    // Releases the cards where they stand: deleting a frame must never move a card.
    r.register("group.dissolve", "Group: dissolve", |m| {
        if let Some(g) = m.group_of_active() {
            m.remove_group(&g);
        }
    });
    r.register("group.rename", "Group: rename", |m| {
        let Some(g) = m.group_of_active() else { return };
        let name = m.group(&g).map(|g| g.name.clone()).unwrap_or_default();
        m.prompt.ask("group name", &name, Pending::RenameGroup(g));
    });
    // Moves the active card between groups, with "no group" as a real stop,
    // so the same two keys that move a card into a group also take it out.
    for step in [1isize, -1] {
        r.register(
            &format!("group.card.{}", step_id(step)),
            &format!("Group: move card to the {}", which(step)),
            move |m| {
                let Some(card) = m.focused().cloned() else {
                    return;
                };
                let mut ring: Vec<Option<String>> = vec![None];
                ring.extend(m.groups.iter().map(|g| Some(g.id.clone())));
                let Some(next) = step_ring(&ring, &card.group_id, step).cloned() else {
                    return;
                };
                if let Some(c) = m.card_mut(&card.id) {
                    c.group_id = next;
                }
                m.prune_empty_groups(); // its old group may now be empty
                m.dirty_layout = true;
            },
        );
    }
    r.register(
        "canvas.zoom.fitGroup",
        "Canvas: fit the selection, the group or the block",
        |m| m.fit_cluster(),
    );
}
