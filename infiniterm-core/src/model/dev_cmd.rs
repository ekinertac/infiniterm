//! The stress harness, development builds only. Port of `commands/dev.ts`.
//! Kept in the tree because the first one was built, measured and deleted,
//! and the numbers could not be reproduced. Driven from a script with
//! `ift dev-run <command>`.
use super::{Effect, Model, NewCard};

impl Model {
    fn send_all(&mut self, data: &[u8]) {
        let panes: Vec<_> = self.here().iter().filter_map(|c| c.pane_id).collect();
        for pane in panes {
            self.effects.push(Effect::WritePane(pane, data.to_vec()));
        }
    }
}

pub fn register(r: &mut crate::commands::CommandRegistry<Model>) {
    r.register(
        "dev.stress.cards",
        "Stress: open 25 cards on a new workspace",
        |m| {
            if !m.dev_build {
                return;
            }
            let ws = m.add_workspace(Some("stress"));
            m.show_workspace(&ws);
            for _ in 0..25 {
                let after = m.focused().map(|c| c.rect);
                let start = m.start_dir.clone();
                let id = m.add_card(
                    &start,
                    NewCard {
                        workspace_id: Some(ws.clone()),
                        after,
                        ..Default::default()
                    },
                );
                m.set_focus(Some(&id));
            }
        },
    );
    r.register(
        "dev.stress.flood",
        "Stress: run yes in every card here, log fps",
        |m| m.send_all(b"yes\n"),
    );
    r.register("dev.stress.calm", "Stress: stop the flood", |m| {
        m.send_all(b"\x03")
    });
    r.register(
        "dev.stress.lines",
        "Stress: 10000 lines in the active card",
        |m| {
            m.with_active_card(|m, id| {
                if let Some(pane) = m.card(&id).and_then(|c| c.pane_id) {
                    m.effects.push(Effect::WritePane(
                        pane,
                        b"for i in $(seq 1 10000); do echo line $i; done\n".to_vec(),
                    ));
                }
            })
        },
    );
    // `dev.stress.zoom` (the continuous zoom) and `dev.stress.dims` are the
    // ui crate's: they need a frame clock and the drawn cell size.
}
