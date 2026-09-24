//! What arrives from the backend's threads, applied to cards: hook reports
//! (authoritative agent state), pane statuses from the process table, pane
//! events (output timestamps, exits), and the staleness sweep. Port of the
//! corresponding blocks of `App.svelte`.
use super::Model;
use crate::agent_state::{apply_hook_event, staleness};
use crate::backend::{PaneEvent, PaneId};
use crate::hooks::HookReport;
use crate::inspect::PaneStatus;

impl Model {
    /// Hook reports are AUTHORITATIVE: they set agent state directly and hold
    /// until the next event. The activity heuristic only ever applies to
    /// cards still at `None`, so a card running an agent never flickers.
    pub fn apply_hook(&mut self, report: &HookReport) {
        let now = self.now_ms;
        let Some(card) = self.card_mut(&report.card_id) else {
            return;
        };
        if let Some(t) = &report.transcript {
            card.transcript_path = Some(t.clone());
        }
        let mut layout_changed = false;
        if let Some(s) = &report.session {
            if card.agent_session.as_deref() != Some(s) {
                card.agent_session = Some(s.clone());
                layout_changed = true;
            }
        }
        let before = card.agent;
        card.agent = apply_hook_event(card.agent, &report.event);
        if let Some((_, track)) = self.programs.get_mut(&report.card_id) {
            track.hook();
        }
        let Some(card) = self.card_mut(&report.card_id) else {
            return;
        };
        card.last_event_at = now;
        // Notification means Claude has been waiting on input for a minute:
        // worth a nudge, unlike every other state change here.
        if report.event == "Notification" {
            card.notified_at = now;
        }
        let line = format!(
            "[hook] {} {before:?}->{:?} {}",
            report.event,
            card.agent,
            if card.title.is_empty() {
                &card.id[..8.min(card.id.len())]
            } else {
                &card.title
            }
        );
        self.log(line);
        if layout_changed {
            self.dirty_layout = true;
        }
    }

    /// The whole list arrives each time and is applied wholesale: a card
    /// missing from the report has left its session. `cwd` is only
    /// overwritten when the report has one; lsof can fail for a process that
    /// exits mid-poll.
    pub fn apply_pane_statuses(&mut self, statuses: &[PaneStatus]) {
        for card in &mut self.cards {
            let status = card
                .pane_id
                .and_then(|p| statuses.iter().find(|s| s.pane == p));
            card.remote = status.and_then(|s| s.remote.clone());
            card.proc = status.and_then(|s| s.proc.clone());
            if let Some(cwd) = status.and_then(|s| s.cwd.clone()) {
                if cwd != card.cwd {
                    card.cwd = cwd;
                    self.dirty_layout = true;
                }
            }
        }
    }

    /// A pane event's effect on the MODEL: the output timestamp for the
    /// activity heuristic, and an exit, which closes the card the same way
    /// the close command does. The bytes themselves go to the terminal body.
    pub fn apply_pane_event(&mut self, pane: PaneId, event: &PaneEvent) {
        let now = self.now_ms;
        let card_id = self
            .cards
            .iter()
            .find(|c| c.pane_id == Some(pane))
            .map(|c| c.id.clone());
        let Some(id) = card_id else { return };
        match event {
            PaneEvent::Output(bytes) => {
                if let Some(c) = self.card_mut(&id) {
                    c.last_output_at = now;
                }
                self.scan_program(&id, bytes);
            }
            // Never scanned: a replay is old output, and its marks would
            // relight cards with commands that finished long ago.
            PaneEvent::Replay(_) => {
                if let Some(c) = self.card_mut(&id) {
                    c.last_output_at = now;
                }
            }
            PaneEvent::Exited { .. } => {
                self.programs.remove(&id);
                self.close_card(&id, true)
            }
            // Not an exit: the pane id stays, so nothing spawns a second
            // shell under the one that is still running.
            PaneEvent::Detached => {
                if let Some(c) = self.card_mut(&id) {
                    c.displaced = true;
                }
                self.log(format!(
                    "displaced {} by an outside attach",
                    &id[..8.min(id.len())]
                ));
            }
            PaneEvent::TitleChanged(_) | PaneEvent::CwdChanged(_) => {}
        }
    }

    /// Reads a card's live output for command marks, progress and
    /// notifications (`program_state`) and applies what it finds.
    fn scan_program(&mut self, id: &str, bytes: &[u8]) {
        let now = self.now_ms;
        let entry = self.programs.entry(id.to_string()).or_default();
        let signals = entry.0.feed(bytes);
        if signals.is_empty() {
            return;
        }
        let Some(card) = self.cards.iter_mut().find(|c| c.id == id) else {
            return;
        };
        let track = &mut entry.1;
        let agent_card = card.agent_session.is_some();
        let mut lines = vec![];
        for sig in &signals {
            let before = card.agent;
            card.agent = track.apply(card.agent, agent_card, sig, now);
            if before != card.agent {
                lines.push(format!(
                    "[shell] {sig:?} {before:?}->{:?} {}",
                    card.agent,
                    short(card)
                ));
            }
        }
        for line in lines {
            self.log(line);
        }
    }

    /// A command that has run long enough turns its card working.
    pub(super) fn promote_programs(&mut self) {
        let now = self.now_ms;
        let mut lines = vec![];
        for card in &mut self.cards {
            if let Some((_, track)) = self.programs.get(&card.id) {
                let before = card.agent;
                card.agent = track.tick(card.agent, now);
                if before != card.agent {
                    lines.push(format!(
                        "[shell] running {before:?}->{:?} {}",
                        card.agent,
                        short(card)
                    ));
                }
            }
        }
        for line in lines {
            self.log(line);
        }
    }

    /// Catches a crashed agent or a missed Stop hook, which would otherwise
    /// leave a card claiming to work forever. Run every 5 s by the ui.
    pub fn sweep_stale(&mut self) {
        let now = self.now_ms;
        let mut lines = vec![];
        for c in &mut self.cards {
            // A quiet build is still running; the sweep is for agents.
            if self
                .programs
                .get(&c.id)
                .is_some_and(|(_, t)| t.owns_working())
            {
                continue;
            }
            let before = c.agent;
            c.agent = staleness(c.agent, c.last_event_at, c.last_output_at, now);
            if before != c.agent {
                lines.push(format!(
                    "{}  {:<18} {:<7} -> {}  (no event {}s, no output {}s)",
                    &c.id[..8.min(c.id.len())],
                    "stale sweep",
                    before.name(),
                    c.agent.name(),
                    ((now - c.last_event_at) / 1000.).round(),
                    ((now - c.last_output_at) / 1000.).round()
                ));
            }
        }
        for line in lines {
            self.effects.push(super::Effect::AgentLog(line));
        }
    }
}

/// The card's name for the log, as the hook lines have it.
fn short(card: &super::Card) -> &str {
    if card.title.is_empty() {
        &card.id[..8.min(card.id.len())]
    } else {
        &card.title
    }
}
