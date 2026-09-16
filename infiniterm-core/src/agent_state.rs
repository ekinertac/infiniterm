//! Hook-driven card activity and stale-working detection.
//!
//! THREE states, not two. `Notification` and `Stop` answer different
//! questions and the reference collapsed both into one "idle": Notification
//! means the agent is BLOCKED ON YOU (it wants permission, or it asked
//! something and cannot go on), Stop means the turn FINISHED and nothing is
//! waiting. Painting those the same made the attention dot count cards that
//! needed nothing, which is the same as having no dot.
//!
//! A plain shell shows no state, a waiting card never expires, and a
//! working card that has gone quiet for a minute stops claiming to work.
//! Hooks and the UI pass timestamps explicitly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentState {
    #[default]
    None,
    /// Mid-turn. Nothing is needed from you.
    Working,
    /// Blocked on you: a permission prompt, a question, or a turn that
    /// ended badly. The only state worth interrupting yourself for.
    Waiting,
    /// The turn finished. Its result is there when you want it.
    Done,
}
/// How long a card may claim to be working with nothing arriving before it
/// is written off as a crashed agent.
///
/// A minute was the reference's, and it is too short now: an agent can think
/// for longer than that without a tool call or a line of output, and the
/// card went colourless mid-thought. Five minutes still catches a crash,
/// and a card that lingers orange is a smaller lie than one that goes blank
/// while the agent is working.
pub const STALE_MS: f64 = 300_000.;

impl AgentState {
    /// For the log, and for `ift ls`.
    pub fn name(self) -> &'static str {
        match self {
            AgentState::None => "none",
            AgentState::Working => "working",
            AgentState::Waiting => "waiting",
            AgentState::Done => "done",
        }
    }
}
pub fn apply_hook_event(prev: AgentState, event: &str) -> AgentState {
    match event {
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => AgentState::Working,
        // A turn that ended in failure is exactly when you want to look, so
        // it waits for you rather than reporting itself done.
        "StopFailure" => AgentState::Waiting,
        // Claude Code sends Notification for two different things: it wants
        // permission MID-TURN, and it has been waiting on your input for a
        // minute. Only the first is the agent being blocked on you; the
        // second arrives 60 s after every Stop, and treating it as a request
        // turned every finished card amber a minute after it went green.
        // A permission prompt can only happen while a turn is running.
        "Notification" if prev == AgentState::Working => AgentState::Waiting,
        "Notification" => prev,
        "Stop" => AgentState::Done,
        "SessionStart" | "SessionEnd" => AgentState::None,
        _ => prev,
    }
}
pub fn staleness(
    state: AgentState,
    last_event_at: f64,
    last_output_at: f64,
    now: f64,
) -> AgentState {
    if state == AgentState::Working && now - last_event_at.max(last_output_at) > STALE_MS {
        AgentState::None
    } else {
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AgentState::*;
    #[test]
    fn mid_turn_is_working() {
        for e in ["UserPromptSubmit", "PreToolUse", "PostToolUse"] {
            assert_eq!(apply_hook_event(None, e), Working);
        }
    }
    // The distinction the whole signal rests on: asking for you is not the
    // same as being finished.
    #[test]
    fn being_blocked_on_you_is_not_the_same_as_being_finished() {
        assert_eq!(apply_hook_event(Working, "Stop"), Done);
        assert_eq!(apply_hook_event(Working, "Notification"), Waiting);
        assert_eq!(apply_hook_event(Working, "StopFailure"), Waiting);
    }

    // Claude Code nags a minute after every turn ends. That is not the agent
    // asking for something, and reading it as one turned every finished card
    // amber sixty seconds after it went green.
    #[test]
    fn the_idle_nag_a_minute_after_a_turn_leaves_the_card_alone() {
        assert_eq!(apply_hook_event(Done, "Notification"), Done);
        assert_eq!(apply_hook_event(None, "Notification"), None);
        // Still blocked if it already was.
        assert_eq!(apply_hook_event(Waiting, "Notification"), Waiting);
    }
    #[test]
    fn session_start_end_show_nothing() {
        assert_eq!(apply_hook_event(None, "SessionStart"), None);
        assert_eq!(apply_hook_event(Done, "SessionEnd"), None);
    }
    #[test]
    fn unknown_event_preserves_state() {
        assert_eq!(apply_hook_event(Working, "PreCompact"), Working);
    }
    #[test]
    fn silence_clears_working() {
        let now = 1000000.;
        let old = now - STALE_MS - 1.;
        assert_eq!(staleness(Working, old, old, now), None);
    }
    #[test]
    fn output_keeps_working_alive() {
        let now = 1000000.;
        assert_eq!(
            staleness(Working, now - STALE_MS - 1., now - 500., now),
            Working
        );
    }
    // Neither settled state expires: a card that asked you something an
    // hour ago is still asking, and a finished turn is still finished.
    #[test]
    fn a_settled_card_never_goes_stale() {
        let now = 1000000.;
        let old = now - STALE_MS * 10.;
        assert_eq!(staleness(Waiting, old, old, now), Waiting);
        assert_eq!(staleness(Done, old, old, now), Done);
    }
}
