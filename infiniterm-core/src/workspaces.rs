//! Workspace naming, navigation, and independent waiting/working counts.
//! Port of workspaces.ts and its tests; callers supply ids and states explicitly.
//! Switching workspaces never removes cards or their PTYs and never follows activity.
use crate::{agent_state::AgentState, viewport::Viewport};
#[derive(Clone, Debug, PartialEq)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub viewport: Viewport,
    /// The card that was focused when this workspace was last left, so
    /// coming back lands where you were. Without it a switch focused the
    /// first card in the list, which is whichever one happened to be made
    /// first and rarely the one you were working in.
    pub focused: Option<String>,
}
pub const INITIAL_VIEWPORT: Viewport = Viewport {
    x: 0.,
    y: 0.,
    scale: 1.,
};
/// Cards blocked on you. NOT cards that finished: counting those lit the
/// dot after every turn, and a dot that is always on says nothing; the
/// ones that finished while you were away are `fresh_done_count`.
pub fn waiting_count(states: &[AgentState]) -> usize {
    states.iter().filter(|&&s| s == AgentState::Waiting).count()
}
pub fn working_count(states: &[AgentState]) -> usize {
    states.iter().filter(|&&s| s == AgentState::Working).count()
}
/// Cards that FINISHED after you left the workspace: `(state, when its
/// last hook event arrived)` against the moment the workspace was last
/// shown. A Done card stays Done until its next turn, so counting every
/// one lit the tab for good; counting the ones you have not been back to
/// see says "something finished while you were away" and clears itself on
/// the visit.
pub fn fresh_done_count(states: &[(AgentState, f64)], left_at: f64) -> usize {
    states
        .iter()
        .filter(|(s, at)| *s == AgentState::Done && *at > left_at)
        .count()
}
pub fn next_name(existing: &[String], stem: &str) -> String {
    for i in 1..=existing.len() + 1 {
        let name = format!("{stem} {i}");
        if !existing.contains(&name) {
            return name;
        }
    }
    unreachable!("one more candidate than existing names")
}
pub fn step_workspace<'a>(
    ids: &'a [String],
    current_id: Option<&str>,
    step: isize,
) -> Option<&'a str> {
    if ids.is_empty() {
        return None;
    }
    let Some(i) = current_id
        .filter(|id| !id.is_empty())
        .and_then(|id| ids.iter().position(|v| v == id))
    else {
        return Some(ids[0].as_str());
    };
    let n = ids.len() as isize;
    Some(&ids[(i as isize + step % n).rem_euclid(n) as usize])
}
pub fn after_closing<'a>(ids: &'a [String], closing_id: &str) -> Option<&'a str> {
    let i = ids.iter().position(|id| id == closing_id)?;
    ids.iter()
        .filter(|id| id.as_str() != closing_id)
        .nth(i.saturating_sub(1))
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ids(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn waiting_counts_only_what_is_asking_for_you() {
        use AgentState::*;
        assert_eq!(waiting_count(&[Waiting, Working, None, Waiting]), 2);
        assert_eq!(waiting_count(&[Working, None]), 0);
        // A finished turn is not a request.
        assert_eq!(waiting_count(&[Done, Done]), 0);
        assert_eq!(waiting_count(&[]), 0);
    }
    #[test]
    fn fresh_done_counts_turns_finished_since_the_workspace_was_left() {
        use AgentState::*;
        let cards = [(Done, 10.), (Done, 30.), (Working, 40.), (Waiting, 50.)];
        assert_eq!(fresh_done_count(&cards, 20.), 1);
        assert_eq!(fresh_done_count(&cards, 0.), 2);
        // Seen already: nothing new to report.
        assert_eq!(fresh_done_count(&cards, 30.), 0);
        assert_eq!(fresh_done_count(&[], 0.), 0);
    }
    #[test]
    fn names_number_from_one_skip_taken() {
        assert_eq!(next_name(&[], "workspace"), "workspace 1");
        assert_eq!(
            next_name(&ids(&["workspace 1"]), "workspace"),
            "workspace 2"
        );
        assert_eq!(
            next_name(&ids(&["workspace 1", "workspace 3"]), "workspace"),
            "workspace 2"
        );
    }
    #[test]
    fn custom_names_ignored() {
        assert_eq!(
            next_name(&ids(&["humbl.ai", "side project"]), "workspace"),
            "workspace 1"
        );
    }
    #[test]
    fn steps_forward_back() {
        let c = ids(&["a", "b", "c"]);
        assert_eq!(step_workspace(&c, Some("a"), 1), Some("b"));
        assert_eq!(step_workspace(&c, Some("b"), -1), Some("a"));
    }
    #[test]
    fn steps_wrap() {
        let c = ids(&["a", "b", "c"]);
        assert_eq!(step_workspace(&c, Some("c"), 1), Some("a"));
        assert_eq!(step_workspace(&c, Some("a"), -1), Some("c"));
    }
    #[test]
    fn unknown_current_enters_first() {
        let c = ids(&["a", "b", "c"]);
        assert_eq!(step_workspace(&c, None, 1), Some("a"));
        assert_eq!(step_workspace(&c, Some("gone"), 1), Some("a"));
    }
    #[test]
    fn empty_workspace_list() {
        assert_eq!(step_workspace(&[], Some("a"), 1), None);
    }
    #[test]
    fn closing_falls_left() {
        let c = ids(&["a", "b", "c"]);
        assert_eq!(after_closing(&c, "c"), Some("b"));
        assert_eq!(after_closing(&c, "b"), Some("a"));
    }
    #[test]
    fn closing_first_falls_right() {
        assert_eq!(after_closing(&ids(&["a", "b", "c"]), "a"), Some("b"));
    }
    #[test]
    fn closing_last_or_unknown_has_no_target() {
        assert_eq!(after_closing(&ids(&["only"]), "only"), None);
        assert_eq!(after_closing(&ids(&["a", "b", "c"]), "gone"), None);
    }
    #[test]
    fn working_counts_only_working() {
        use AgentState::*;
        assert_eq!(working_count(&[Working, Done, None, Working]), 2);
    }
}
