//! The stress harness, development builds only. Port of `commands/dev.ts`.
//! Kept in the tree because the first one was built, measured and deleted,
//! and the numbers could not be reproduced. Driven from a script with
//! `ift dev-run <command>`.
use super::{Effect, Model, NewCard};

/// Ten fits, one every 600 ms: a 240 ms fit, its tail, and a reading.
const STRESS_ZOOM_STEPS: u32 = 10;
const STRESS_ZOOM_GAP_MS: f64 = 600.;

impl Model {
    /// One step of `dev.stress.zoom`: fit the card, then everything, and
    /// so on; the fps of the step before is what the ui logs at each.
    pub(super) fn stress_zoom_step(&mut self, n: u32, now: f64) {
        let cmd = if n % 2 == 1 {
            "canvas.zoom.fitAll"
        } else {
            "canvas.zoom.fitCard"
        };
        self.effects.push(Effect::RunCommand(cmd.to_string()));
        self.effects.push(Effect::LogFps(n));
        self.stress_zoom = (n + 1 < STRESS_ZOOM_STEPS).then_some((n + 1, now + STRESS_ZOOM_GAP_MS));
    }

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
    r.register(
        "dev.stress.zoom",
        "Stress: alternate fit card / fit all ten times, log fps",
        |m| {
            if m.dev_build {
                let now = m.now_ms;
                m.stress_zoom_step(0, now);
            }
        },
    );
    r.register(
        "dev.stress.dims",
        "Stress: log every terminal grid and element size",
        |m| {
            if !m.dev_build {
                return;
            }
            let lines: Vec<String> = m
                .here()
                .iter()
                .map(|c| {
                    format!(
                        "card {} {:?} pane={:?} path={} rect={}x{}",
                        &c.id[..c.id.len().min(8)],
                        c.kind,
                        c.pane_id,
                        c.path.as_deref().unwrap_or("-"),
                        c.rect.w,
                        c.rect.h
                    )
                })
                .collect();
            for line in lines {
                m.effects.push(Effect::Log(line));
            }
            m.effects.push(Effect::LogDims);
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stress_zoom_alternates_fits_on_the_clock() {
        let mut m = Model::new();
        m.dev_build = true;
        m.tick(1000.);
        m.stress_zoom_step(0, 1000.);
        let first = m.take_effects();
        assert!(matches!(&first[0], Effect::RunCommand(c) if c == "canvas.zoom.fitCard"));
        assert!(matches!(first[1], Effect::LogFps(0)));
        m.tick(1500.);
        assert!(m.take_effects().is_empty(), "not due yet");
        m.tick(1600.);
        let second = m.take_effects();
        assert!(matches!(&second[0], Effect::RunCommand(c) if c == "canvas.zoom.fitAll"));
        for t in 2..10 {
            m.tick(1000. + 600. * t as f64);
            m.take_effects();
        }
        assert!(m.stress_zoom.is_none(), "ten steps and it stops");
        m.tick(100_000.);
        assert!(m.take_effects().is_empty());
    }
}
