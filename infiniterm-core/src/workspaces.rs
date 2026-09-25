//! Workspace naming, navigation, and the agent dots a workspace tab wears.
//! Port of workspaces.ts and its tests; callers supply ids and states explicitly.
//! Switching workspaces never removes cards or their PTYs and never follows activity.
use crate::{agent_state::AgentState, grid::Rect, viewport::Viewport};
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
/// The dots a workspace's tab wears: ONE PER CARD, in the card's order, a
/// grey one for a card with nothing to say and the card's own hue when an
/// agent in it is working, waiting or freshly done, so the tab is a row of
/// lamps and you watch them light one at a time. `cards` is `(state, when
/// its last hook event arrived)`; a card's dot keeps its place whatever it
/// does, which is what makes the row readable. A Done card lights only
/// when its Stop arrived after you LEFT the workspace (`left_at`) and never
/// on the active tab (`active`), because a Done card stays Done until its
/// next turn and lighting every one lit the tab for good, which is the
/// same as having no dot; otherwise it is grey like a shell.
pub fn tab_dots(cards: &[(AgentState, f64)], left_at: f64, active: bool) -> Vec<AgentState> {
    cards
        .iter()
        .map(|&(s, at)| match s {
            AgentState::Waiting | AgentState::Failed | AgentState::Working => s,
            AgentState::Done if !active && at > left_at => s,
            _ => AgentState::None,
        })
        .collect()
}
/// The order the tab's dots follow: the cards as they lie on the canvas,
/// read like a page, top row first and left to right within it. Two cards
/// are on one row when their tops are within `ROW_SLACK` of each other,
/// which absorbs the odd pixel a drag left behind; slots keep tops aligned
/// otherwise. Returns indexes into `rects`. Number order was tried first
/// and a dot said nothing about WHERE its card was.
pub fn reading_order(rects: &[Rect]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..rects.len()).collect();
    order.sort_by(|&a, &b| {
        let (ra, rb) = (&rects[a], &rects[b]);
        let same_row = (ra.y - rb.y).abs() <= ROW_SLACK;
        if same_row {
            ra.x.total_cmp(&rb.x)
        } else {
            ra.y.total_cmp(&rb.y)
        }
    });
    order
}
/// World units within which two cards' tops count as one row.
/// shortcut: the comparison is not transitive across a chain of tops each
/// within the slack of the next; slots snap tops to the same values, so no
/// such chain exists on a real canvas.
const ROW_SLACK: f64 = 24.;
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
    fn one_dot_per_card_in_place_grey_when_nothing_to_say() {
        use AgentState::*;
        let cards = [(Working, 1.), (None, 2.), (Waiting, 3.), (None, 4.)];
        assert_eq!(
            tab_dots(&cards, 0., false),
            vec![Working, None, Waiting, None]
        );
        assert_eq!(tab_dots(&[], 0., false), vec![]);
    }
    #[test]
    fn done_lights_only_turns_finished_since_the_workspace_was_left() {
        use AgentState::*;
        let cards = [(Done, 10.), (Done, 30.), (Working, 40.)];
        assert_eq!(tab_dots(&cards, 20., false), vec![None, Done, Working]);
        assert_eq!(tab_dots(&cards, 0., false), vec![Done, Done, Working]);
        // Seen already: grey like a shell.
        assert_eq!(tab_dots(&cards, 30., false), vec![None, None, Working]);
        // The active workspace shows the cards themselves.
        assert_eq!(tab_dots(&cards, 0., true), vec![None, None, Working]);
    }
    #[test]
    fn dots_read_the_canvas_like_a_page() {
        let r = |x, y| Rect {
            x,
            y,
            w: 100.,
            h: 100.,
        };
        // A row of two, a quarter tucked under the first, one far right on
        // the top row, then a second row.
        let rects = [
            r(200., 0.),
            r(0., 0.),
            r(0., 400.),
            r(1000., 3.),
            r(600., 400.),
        ];
        assert_eq!(reading_order(&rects), vec![1, 0, 3, 2, 4]);
        assert_eq!(reading_order(&[]), Vec::<usize>::new());
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
