//! The terminal bodies' lifecycle, per frame: a terminal card with no shell
//! gets one spawned (INFINITERM_CARD_ID in its environment, which is how the
//! hooks address the card), the scheduler's pieces are fed to their grids
//! under one budget, parsed bytes are acknowledged to the reader (the
//! backpressure), what the programs wrote back goes to the PTYs, and a card
//! whose rect or metrics changed re-counts its grid and tells the PTY.
//! Port of `paneRegistry.ts`'s routing and `TerminalCard.svelte`'s start.
use crate::terminal_body::{weight_of, Metrics, TerminalBody};
use crate::AppView;
use gpui::{font, px, TextRun, Window};
use infiniterm_core::backend::Panes;
use infiniterm_core::grid::Size;
use infiniterm_core::saved_layout::CardKind;

/// The first family in a CSS-style list that is a real font name. The
/// reference's default is `ui-monospace, Menlo, monospace`; the generic
/// names mean nothing to gpui, so Menlo is what every Mac gets.
pub fn family_of(list: &str) -> String {
    list.split(',')
        .map(|f| f.trim().trim_matches('"').trim_matches('\'').to_string())
        .find(|f| {
            !f.is_empty() && !matches!(f.as_str(), "ui-monospace" | "monospace" | "system-ui")
        })
        .unwrap_or_else(|| "Menlo".to_string())
}

impl AppView {
    /// The cell metrics the config asks for, measured on the font at scale 1.
    pub fn metrics(&self, window: &Window) -> Metrics {
        let t = &self.model.config.terminal;
        let family = family_of(&t.font_family);
        let weight = weight_of(&t.font_weight);
        let bold_weight = weight_of(&t.font_weight_bold);
        let mut f = font(family.clone());
        f.weight = weight;
        let probe = window.text_system().shape_line(
            "M".into(),
            px(t.font_size as f32),
            &[TextRun {
                len: 1,
                font: f,
                color: gpui::black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        // The cell is the glyph advance. `letterSpacing` is NOT applied:
        // gpui shapes a run with the font's own advances, and drawing each
        // glyph on its own to honour a spacing would cost the batching that
        // holds 120 fps. The reference all but ignored it too (xterm snapped
        // the cell to whole pixels, so -2 came out as -0.16 px).
        Metrics {
            family,
            font_px: t.font_size,
            line_height: t.line_height,
            cell_w: f32::from(probe.width) as f64,
            weight,
            bold_weight,
        }
    }

    /// Every terminal card gets a body and a shell once; closed cards lose
    /// theirs (the model already killed the shell). Rect and metric changes
    /// re-count the grid.
    pub fn reconcile_terminals(&mut self, window: &Window) {
        let ids: Vec<String> = self.model.cards.iter().map(|c| c.id.clone()).collect();
        self.bodies.retain(|id, _| ids.contains(id));
        let metrics = self.metrics(window);
        let scrollback = self.model.config.terminal.scrollback as usize;
        let shell = Some(self.model.config.terminal.shell.clone()).filter(|s| !s.is_empty());
        let palette = self.palette.clone();
        let blink = self.model.config.terminal.cursor_blink;
        let inactive_dim = self.model.config.ui.inactive_dim;
        let cards: Vec<_> = self
            .model
            .cards
            .iter()
            .filter(|c| c.kind == CardKind::Terminal)
            .cloned()
            .collect();
        let maximized = self
            .model
            .selection
            .maximized
            .then(|| self.model.selection.focused_id.clone())
            .flatten();
        for card in cards {
            let world = fit_size(
                &card.id,
                card.rect,
                maximized.as_deref(),
                self.model.view_size,
            );
            let is_terminal = self
                .bodies
                .get_mut(&card.id)
                .and_then(|b| b.as_any_mut().downcast_mut::<TerminalBody>())
                .is_some();
            if !is_terminal {
                self.bodies.insert(
                    card.id.clone(),
                    Box::new(TerminalBody::new(
                        &metrics,
                        palette.clone(),
                        world,
                        scrollback,
                        card.cwd.clone(),
                    )),
                );
            }
            let Some(body) = self
                .bodies
                .get_mut(&card.id)
                .and_then(|b| b.as_any_mut().downcast_mut::<TerminalBody>())
            else {
                continue;
            };
            if body.palette != palette {
                body.set_palette(palette.clone());
            }
            body.blink = blink;
            body.inactive_dim = inactive_dim;
            let metrics_changed = body.font_family != metrics.family
                || body.weight != metrics.weight
                || body.bold_weight != metrics.bold_weight
                || body.font_px != metrics.font_px
                || body.line_height != metrics.line_height
                || body.cell_w != metrics.cell_w;
            let refit = if metrics_changed {
                body.set_metrics(&metrics, world)
            } else {
                body.refit_to(world)
            };
            if card.pane_id.is_none() && body.error.is_none() {
                // A card that was here before and whose session is still
                // running takes it back rather than starting a second shell
                // beside it. This is the whole point of the tmux and daemon
                // backends and the only reason `session` is in the save file.
                let adopt = card
                    .session
                    .clone()
                    .filter(|s| self.live_sessions.iter().any(|live| live == s))
                    .and_then(|s| self.backend.pty.adopt(&s));
                if let Some(pane) = adopt {
                    body.pane = Some(pane);
                    // The program in an adopted session announced whether it
                    // speaks the kitty keyboard protocol at ITS startup,
                    // which is long out of the ring we are about to replay,
                    // and Claude Code was measured never re-announcing. The
                    // save file is the only thing that still remembers.
                    if card.kitty_keys {
                        body.assume_kitty_keys();
                    }
                    self.backend.pty.resize_now(pane, body.cols(), body.rows());
                    if let Some(c) = self.model.card_mut(&card.id) {
                        c.pane_id = Some(pane);
                    }
                    continue;
                }
                // The session is gone (a reboot, a power cut) but its daemon
                // may have left the ring on disk: what was on screen comes
                // back above the new shell, with a line saying so, instead
                // of 23 cards of a bare prompt. `card.session` is overwritten
                // below once the new pane reports its id.
                if let Some((ring, when)) = card
                    .session
                    .as_deref()
                    .and_then(|s| self.backend.pty.take_ring(s))
                {
                    body.feed_lost_session(&ring, &when, card.agent_session.as_deref());
                }
                // `terminal.shell` when set, else the backend's $SHELL; a card
                // made to run one program runs that instead.
                let command = card.command.clone().or_else(|| shell.clone());
                let env = vec![("INFINITERM_CARD_ID".to_string(), card.id.clone())];
                match self.backend.pty.spawn_now(
                    std::path::Path::new(&card.cwd),
                    command.as_deref(),
                    env,
                ) {
                    Ok(pane) => {
                        body.pane = Some(pane);
                        // The real size before the shell draws its first prompt,
                        // or it wraps at 80x24 until something resizes it.
                        self.backend.pty.resize_now(pane, body.cols(), body.rows());
                        if let Some(c) = self.model.card_mut(&card.id) {
                            c.pane_id = Some(pane);
                        }
                    }
                    Err(e) => {
                        let text = format!("could not start a shell in {}: {e}", card.cwd);
                        body.error = Some(text.clone());
                        if let Some(c) = self.model.card_mut(&card.id) {
                            c.error = Some(text);
                        }
                    }
                }
            } else if refit {
                if let Some(pane) = body.pane {
                    self.backend.pty.resize_now(pane, body.cols(), body.rows());
                }
            }
            body.cwd = card.cwd.clone();
            // The title the program set through the terminal. Kept apart
            // from `card.title`, which is a name somebody chose; the label
            // uses this only while an agent is in the card.
            // What the backend calls this card's session, learned a frame or
            // two after the card was made. Saved, so the next launch can
            // find it.
            if let Some(pane) = card.pane_id {
                let session = self.backend.pty.session_id(pane);
                if session.is_some() && card.session != session {
                    if let Some(c) = self.model.card_mut(&card.id) {
                        c.session = session;
                        self.model.dirty_layout = true;
                    }
                }
            }
            // Mirrored so a relaunch can seed it back; see `assume_kitty_keys`.
            let kitty = body.kitty_keys();
            if card.kitty_keys != kitty {
                if let Some(c) = self.model.card_mut(&card.id) {
                    c.kitty_keys = kitty;
                    self.model.dirty_layout = true;
                }
            }
            let osc = body.title.clone();
            if card.osc_title != osc {
                if let Some(c) = self.model.card_mut(&card.id) {
                    c.osc_title = osc;
                }
                // The label was drawn before this ran.
                self.redraw = true;
            }
        }
    }

    /// One budget of output across the panes, then the acks and the replies.
    pub fn feed_terminals(&mut self, now: f64, cx: &mut gpui::App) {
        let pieces = self.scheduler.take(now);
        for piece in pieces {
            let pane = piece.pane;
            let n = piece.data().len();
            if let Some(body) = self.terminal_for_pane(pane) {
                body.feed(piece.data());
                // OSC 52: a program (tmux, a remote vim) put text on the
                // clipboard through the terminal.
                if let Some(text) = body.clipboard_out.take() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
            }
            let owed = self.ledger.note(pane, n);
            if owed > 0 {
                self.backend.pty.ack_now(pane, owed);
            }
        }
        // Small outputs must not wait for a batch to fill.
        for (pane, bytes) in self.ledger.drain() {
            self.backend.pty.ack_now(pane, bytes);
        }
        // What the emulator answered a program with (a colour query, device
        // attributes, a cursor report) is sent ONLY when we are the
        // terminal. Under tmux we are not: tmux answers the pane itself,
        // and a second answer arrives at the program as keystrokes. It was
        // measured landing in the shell as `10;rgb:5050/9e9e/3131` after
        // the program that asked had already gone, and Claude Code, which
        // re-queries as it redraws, took a steady drip of it into its input.
        // Under our own daemon nothing between us and the pty answers
        // anything, so our replies MUST go: `we_are_the_terminal()` is true
        // there and false only under tmux. Inverting this line reproduces
        // tmux bug 9 exactly, so it is not spelled as `!is_tmux()` any more.
        let mut per_pane: Vec<PaneWrites> = vec![];
        for body in self.bodies.values_mut() {
            if let Some(t) = body.as_any_mut().downcast_mut::<TerminalBody>() {
                let replies = std::mem::take(&mut t.replies);
                if let Some(pane) = t.pane {
                    per_pane.push((pane, std::mem::take(&mut t.outgoing), replies));
                }
            }
        }
        // A decoy's program may query its terminal too (decoys.rs).
        for t in self.decoys.values_mut() {
            let replies = std::mem::take(&mut t.replies);
            if let Some(pane) = t.pane {
                per_pane.push((pane, std::mem::take(&mut t.outgoing), replies));
            }
        }
        let writes = writes_for(&self.backend.pty, per_pane);
        for (pane, bytes) in writes {
            self.backend.pty.write_now(pane, &bytes);
        }
    }

    /// Anything a body's outgoing queue holds right now (after a key), so a
    /// keystroke reaches the shell in this event, not the next frame.
    pub fn flush_writes(&mut self) {
        let mut writes: Vec<(u32, Vec<u8>)> = vec![];
        for body in self.bodies.values_mut() {
            if let Some(t) = body.as_any_mut().downcast_mut::<TerminalBody>() {
                if let Some(pane) = t.pane {
                    for bytes in std::mem::take(&mut t.outgoing) {
                        writes.push((pane, bytes));
                    }
                }
            }
        }
        for (pane, bytes) in writes {
            self.backend.pty.write_now(pane, &bytes);
        }
    }
}

/// The size a card's grid is fitted to.
///
/// A maximized card is PAINTED into the whole canvas at scale 1 (paint.rs
/// returns early and hands the body `bounds`), so that is the size its grid
/// has to be. Fitting it to `card.rect` instead left the pty at the size
/// the card is on the canvas: it filled the window and the shell still
/// believed it had the old columns and rows, so nothing reflowed and
/// maximising appeared to change nothing.
fn fit_size(
    id: &str,
    rect: infiniterm_core::grid::Rect,
    maximized: Option<&str>,
    view: Size,
) -> Size {
    if maximized == Some(id) {
        return view;
    }
    Size {
        w: rect.w,
        h: rect.h,
    }
}

/// One pane's pending writes: its own keystrokes/pastes, and the emulator's
/// replies to a program's query, still separate because only the second
/// group is gated (see `gated_writes`).
type PaneWrites = (u32, Vec<Vec<u8>>, Vec<Vec<u8>>);

/// The one place the gate's POLARITY is written down.
///
/// `gated_writes` below takes a bool, which a test can pin from both sides
/// but which anyone can also negate by accident at a call site, where no
/// test reaches: `feed_terminals` needs a live `AppView`, a gpui window and
/// a real backend, and nothing in this crate stands one up. So the call
/// site is given no bool to negate. It hands over the backend, and the
/// question is asked here, once, in an expression with no room for a `!`.
/// Structural, because the alternative was a comment asking the next person
/// to be careful, and tmux bug 9 is what being careful already cost.
fn writes_for(terminal: &Panes, per_pane: Vec<PaneWrites>) -> Vec<(u32, Vec<u8>)> {
    gated_writes(terminal.we_are_the_terminal(), per_pane)
}

/// The `feed_terminals` write gate, pulled out of the gpui-shaped loop so it
/// can be tested without a window or a live pty: keystrokes and pastes
/// (`outgoing`) always reach the pane, but the emulator's own replies to a
/// program's query go only when `answer` says we are the terminal. Getting
/// `answer` backwards is tmux bug 9; see `writes_for` for why no caller
/// passes this argument by hand.
fn gated_writes(answer: bool, per_pane: Vec<PaneWrites>) -> Vec<(u32, Vec<u8>)> {
    let mut writes = vec![];
    for (pane, outgoing, replies) in per_pane {
        for bytes in outgoing {
            writes.push((pane, bytes));
        }
        if answer {
            for bytes in replies {
                writes.push((pane, bytes));
            }
        }
    }
    writes
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cmd+Shift+Enter fills the window with the card, and the grid has to
    // follow or the shell keeps the columns it had on the canvas.
    #[test]
    fn a_maximized_card_is_fitted_to_the_view_and_the_others_to_their_rects() {
        let rect = infiniterm_core::grid::Rect {
            x: 0.,
            y: 0.,
            w: 400.,
            h: 300.,
        };
        let view = Size { w: 1920., h: 1080. };
        let own = Size { w: 400., h: 300. };
        assert_eq!(
            fit_size("term-1", rect, Some("term-1"), view),
            view,
            "the maximized one fills the window"
        );
        assert_eq!(
            fit_size("term-1", rect, None, view),
            own,
            "nothing maximized: its own rect"
        );
        assert_eq!(
            fit_size("term-1", rect, Some("another-card"), view),
            own,
            "a card behind the maximized one keeps its own size"
        );
    }

    // Through a REAL backend, not a bool somebody passed in: this is the
    // half of the gate that the pure tests below cannot reach. A local
    // shell is our own terminal, so a program's query gets our answer.
    // (The false half needs a tmux to build `Panes::Tmux`, which these
    // tests must not require; `we_are_the_terminal` covers it in core.)
    #[test]
    fn a_local_backend_sends_the_emulators_answers() {
        use infiniterm_core::config::TerminalBackend;
        let (panes, _rx, _fell_back) = Panes::start(TerminalBackend::Pty, 4);
        let writes = writes_for(
            &panes,
            vec![(7, vec![b"ls\n".to_vec()], vec![b"\x1b[0n".to_vec()])],
        );
        assert_eq!(
            writes,
            vec![(7, b"ls\n".to_vec()), (7, b"\x1b[0n".to_vec())],
            "a local shell has nothing in front of it to answer for us"
        );
    }

    // The call site under test: `we_are_the_terminal()` becomes `answer`,
    // and this is the boundary between "true" (local shells, our daemon)
    // and "false" (tmux). Inverted, our replies reach a program that tmux
    // already answered, landing in its input as keystrokes (tmux bug 9).
    #[test]
    fn replies_are_forwarded_only_when_we_are_the_terminal() {
        let per_pane = vec![(1, vec![b"keys".to_vec()], vec![b"\x1b[0c".to_vec()])];

        let writes = gated_writes(true, per_pane.clone());
        assert_eq!(
            writes,
            vec![(1, b"keys".to_vec()), (1, b"\x1b[0c".to_vec())],
            "we are the terminal: our reply must reach the pane"
        );

        let writes = gated_writes(false, per_pane);
        assert_eq!(
            writes,
            vec![(1, b"keys".to_vec())],
            "under tmux tmux already answered; a second answer is corruption"
        );
    }

    // Outgoing bytes are a card's own keystrokes/pastes, not an emulator's
    // reply to a query; they must never be gated by the terminal-identity
    // question, or a paste under tmux would silently vanish.
    #[test]
    fn outgoing_bytes_ignore_the_gate_entirely() {
        let per_pane = vec![(7, vec![b"echo hi\n".to_vec()], vec![])];
        assert_eq!(
            gated_writes(false, per_pane.clone()),
            vec![(7, b"echo hi\n".to_vec())]
        );
        assert_eq!(
            gated_writes(true, per_pane),
            vec![(7, b"echo hi\n".to_vec())]
        );
    }

    #[test]
    fn several_panes_are_each_gated_independently() {
        let per_pane = vec![
            (1, vec![], vec![b"reply-1".to_vec()]),
            (2, vec![], vec![b"reply-2".to_vec()]),
        ];
        assert_eq!(
            gated_writes(true, per_pane),
            vec![(1, b"reply-1".to_vec()), (2, b"reply-2".to_vec())]
        );
    }
}
