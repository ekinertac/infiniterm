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
        for card in cards {
            let world = Size {
                w: card.rect.w,
                h: card.rect.h,
            };
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
                // A card that was here before and whose tmux window is still
                // running takes it back rather than starting a second shell
                // beside it. This is the whole point of the tmux backend and
                // the only reason `tmux_window` is in the save file.
                let adopt = card
                    .tmux_window
                    .clone()
                    .filter(|w| self.live_windows.iter().any(|live| live == w))
                    .and_then(|w| self.backend.pty.adopt(&w));
                if let Some(pane) = adopt {
                    body.pane = Some(pane);
                    self.backend.pty.resize_now(pane, body.cols(), body.rows());
                    if let Some(c) = self.model.card_mut(&card.id) {
                        c.pane_id = Some(pane);
                    }
                    continue;
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
            // What tmux called this card's window, learned a frame or two
            // after the card was made. Saved, so the next launch can find it.
            if let Some(pane) = card.pane_id {
                let window = self.backend.pty.window_id(pane);
                if window.is_some() && card.tmux_window != window {
                    if let Some(c) = self.model.card_mut(&card.id) {
                        c.tmux_window = window;
                        self.model.dirty_layout = true;
                    }
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
        let answer = !self.backend.pty.is_tmux();
        let mut writes: Vec<(u32, Vec<u8>)> = vec![];
        for body in self.bodies.values_mut() {
            if let Some(t) = body.as_any_mut().downcast_mut::<TerminalBody>() {
                let replies = std::mem::take(&mut t.replies);
                if let Some(pane) = t.pane {
                    for bytes in std::mem::take(&mut t.outgoing) {
                        writes.push((pane, bytes));
                    }
                    if answer {
                        for bytes in replies {
                            writes.push((pane, bytes));
                        }
                    }
                }
            }
        }
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
