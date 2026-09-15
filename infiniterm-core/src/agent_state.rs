//! Hook-driven card activity and stale-working detection.
//! Port of agentState.ts and its tests; hooks and the UI pass timestamps explicitly.
//! Plain shells show no state, failed turns are idle, and idle never expires.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentState {
    #[default]
    None,
    Working,
    Idle,
}
pub const STALE_MS: f64 = 60000.;
pub fn apply_hook_event(prev: AgentState, event: &str) -> AgentState {
    match event {
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => AgentState::Working,
        "Stop" | "StopFailure" | "Notification" => AgentState::Idle,
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
    #[test]
    fn finished_turn_is_idle() {
        for e in ["Stop", "StopFailure", "Notification"] {
            assert_eq!(apply_hook_event(Working, e), Idle);
        }
    }
    #[test]
    fn session_start_end_show_nothing() {
        assert_eq!(apply_hook_event(None, "SessionStart"), None);
        assert_eq!(apply_hook_event(Idle, "SessionEnd"), None);
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
    #[test]
    fn idle_never_stale() {
        let now = 1000000.;
        let old = now - STALE_MS * 10.;
        assert_eq!(staleness(Idle, old, old, now), Idle);
    }
}
