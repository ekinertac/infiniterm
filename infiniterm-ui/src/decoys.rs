//! The decoy over a masked card (`card.mask`, Cmd+Shift+H): somebody is
//! reading your screen in a cafe, and the card keeps running under a
//! second pane that looks like work and says nothing.
//!
//! A decoy is a whole `TerminalBody` on a pane of its own, running
//! `terminal.decoyCommand` in the card's directory at the card's size, and
//! `paint.rs` draws it in the card's place while `card.masked` holds. The
//! real pane is untouched: its output still reaches its grid, only unseen.
//! Reconciled from the flag each frame like the terminal bodies are from
//! the cards (`terminals.rs`): a flag with no decoy spawns one, a decoy
//! with no flag or no card is killed. Keys never reach a decoy or the card
//! under it; `input.rs` swallows everything but Enter and Escape, which
//! unmask. Not saved: a restart shows the cards.
use crate::terminal_body::TerminalBody;
use crate::AppView;
use gpui::Window;
use infiniterm_core::grid::Size;

/// A decoy needs no history; the mask is the last screenful.
const DECOY_SCROLLBACK: usize = 0;

impl AppView {
    /// A decoy for every masked card at that card's size, and none else.
    pub fn reconcile_decoys(&mut self, window: &Window) {
        let masked: Vec<(String, Size, String)> = self
            .model
            .cards
            .iter()
            .filter(|c| c.masked)
            .map(|c| {
                let world = if self.model.selection.maximized
                    && self.model.selection.focused_id.as_deref() == Some(&c.id)
                {
                    self.model.view_size
                } else {
                    Size {
                        w: c.rect.w,
                        h: c.rect.h,
                    }
                };
                (c.id.clone(), world, c.cwd.clone())
            })
            .collect();
        let gone: Vec<String> = self
            .decoys
            .keys()
            .filter(|id| !masked.iter().any(|(m, _, _)| m == *id))
            .cloned()
            .collect();
        for id in gone {
            if let Some(pane) = self.decoys.remove(&id).and_then(|d| d.pane) {
                self.backend.pty.kill(pane);
            }
        }
        if masked.is_empty() {
            return;
        }
        let metrics = self.metrics(window);
        let palette = self.palette.clone();
        let command = self.model.config.terminal.decoy_command.clone();
        for (id, world, cwd) in masked {
            if !self.decoys.contains_key(&id) {
                let mut body = TerminalBody::new(
                    &metrics,
                    palette.clone(),
                    world,
                    DECOY_SCROLLBACK,
                    cwd.clone(),
                );
                match self
                    .backend
                    .pty
                    .spawn_now(std::path::Path::new(&cwd), Some(&command), vec![])
                {
                    Ok(pane) => {
                        body.pane = Some(pane);
                        self.backend.pty.resize_now(pane, body.cols(), body.rows());
                    }
                    Err(e) => body.error = Some(format!("could not start the decoy: {e}")),
                }
                self.decoys.insert(id.clone(), body);
                continue;
            }
            let Some(body) = self.decoys.get_mut(&id) else {
                continue;
            };
            if body.refit_to(world) {
                if let Some(pane) = body.pane {
                    self.backend.pty.resize_now(pane, body.cols(), body.rows());
                }
            }
        }
    }

    /// The decoy that owns `pane`, for the output router.
    pub fn decoy_for_pane(
        &mut self,
        pane: infiniterm_core::backend::PaneId,
    ) -> Option<&mut TerminalBody> {
        self.decoys.values_mut().find(|d| d.pane == Some(pane))
    }
}
