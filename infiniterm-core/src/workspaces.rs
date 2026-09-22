//! Workspace naming, navigation, and the agent dots a workspace tab wears.
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
/// The dots a workspace's tab wears: ONE PER AGENT CARD in that card's own
/// hue, so a tab with three Claudes shows three dots and you can see how
/// many are asking. `cards` is `(state, when its last hook event arrived)`;
/// the order is waiting, working, done, so what needs you is nearest the
/// name. A Done card counts only when its Stop arrived after you LEFT the
/// workspace (`left_at`) and never on the active tab (`active`), because
/// a Done card stays Done until its next turn and counting every one lit
/// the tab for good, which is the same as having no dot. A plain shell
/// (`None`) is no dot.
pub fn tab_dots(cards: &[(AgentState, f64)], left_at: f64, active: bool) -> Vec<AgentState> {
    let mut dots: Vec<AgentState> = cards
        .iter()
        .filter_map(|&(s, at)| match s {
            AgentState::Waiting | AgentState::Working => Some(s),
            AgentState::Done if !active && at > left_at => Some(s),
            _ => None,
        })
        .collect();
    dots.sort_by_key(|s| match s {
        AgentState::Waiting => 0,
        AgentState::Working => 1,
        _ => 2,
    });
    dots
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
    fn one_dot_per_agent_card_waiting_first() {
        use AgentState::*;
        let cards = [
            (Working, 1.),
            (None, 2.),
            (Waiting, 3.),
            (Working, 4.),
            (Waiting, 5.),
        ];
        assert_eq!(
            tab_dots(&cards, 0., false),
            vec![Waiting, Waiting, Working, Working]
        );
        assert_eq!(tab_dots(&[(None, 1.)], 0., false), vec![]);
        assert_eq!(tab_dots(&[], 0., false), vec![]);
    }
    #[test]
    fn done_counts_only_turns_finished_since_the_workspace_was_left() {
        use AgentState::*;
        let cards = [(Done, 10.), (Done, 30.), (Working, 40.)];
        assert_eq!(tab_dots(&cards, 20., false), vec![Working, Done]);
        assert_eq!(tab_dots(&cards, 0., false), vec![Working, Done, Done]);
        // Seen already: nothing new to report.
        assert_eq!(tab_dots(&cards, 30., false), vec![Working]);
        // The active workspace shows the cards themselves.
        assert_eq!(tab_dots(&cards, 0., true), vec![Working]);
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
}
