//! Cmd+Tab for cards: the order the switcher offers, and the rule that
//! decides which visits are worth remembering.
//!
//! The list is recency, not geometry, because the canvas has no order and
//! twenty cards have no neighbours worth stepping through. What makes it
//! usable is what is NOT in it: Ekin walks the canvas with Cmd+Alt+Arrow,
//! so the cards he crosses on the way would fill a plain most-recent list
//! and the one press that should land on the other card he is working in
//! would land on a card he merely passed. A visit earns its place by
//! being CHOSEN (a click, the palette, `ift`, a new card, this switcher)
//! or by being stayed in: typed into, or held for `TRAIL_DWELL_MS`.
//!
//! Pure: the model owns `focus_trail` and the current visit and calls
//! these (model/mod.rs `land_focus`, `promote_focus`, `note_input`); the
//! ui draws the list and commits on the Ctrl release (keycode.rs).

/// How long a card must keep the focus before an arrow visit counts as a
/// visit at all. Long enough that three cards crossed on the way to a
/// fourth leave no trace, short enough that a glance you actually read
/// does. A second and a half; the number nobody can defend exactly, so it
/// is here with its reason rather than inline.
pub const TRAIL_DWELL_MS: f64 = 1500.;

/// Whether a focus visit belongs in the trail.
pub fn earns_trail(deliberate: bool, typed: bool, dwell_ms: f64) -> bool {
    deliberate || typed || dwell_ms >= TRAIL_DWELL_MS
}

/// The switcher's rows: the card you are in first (so one press and
/// release lands on the previous one, the way Cmd+Tab does), then the
/// trail most recent first, then every other card so the switcher is
/// never a list of one on a fresh canvas.
pub fn order(current: Option<&str>, trail: &[String], all: &[String]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let push = |id: &str, out: &mut Vec<String>| {
        if all.iter().any(|a| a == id) && !out.iter().any(|o| o == id) {
            out.push(id.to_string());
        }
    };
    if let Some(c) = current {
        push(c, &mut out);
    }
    for id in trail.iter().rev() {
        push(id, &mut out);
    }
    for id in all {
        push(id, &mut out);
    }
    out
}

/// Stepping wraps, because a switcher that stops at the end makes you
/// count.
pub fn step(len: usize, index: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (index as isize + delta).rem_euclid(len as isize) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ids(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_visit_earns_its_place_by_choice_by_typing_or_by_staying() {
        assert!(earns_trail(true, false, 0.));
        assert!(earns_trail(false, true, 0.));
        assert!(earns_trail(false, false, TRAIL_DWELL_MS));
        // Crossed on the way somewhere else.
        assert!(!earns_trail(false, false, 200.));
    }

    #[test]
    fn the_current_card_is_first_then_recency_then_the_rest() {
        let all = ids(&["a", "b", "c", "d"]);
        // Trail is oldest first: c was the last card that earned its place.
        let trail = ids(&["b", "c"]);
        assert_eq!(order(Some("a"), &trail, &all), ids(&["a", "c", "b", "d"]));
        // Nothing focused yet, nothing earned: the canvas as it stands.
        assert_eq!(order(None, &[], &all), all);
        // A trail entry for a card that has closed is skipped.
        assert_eq!(
            order(Some("a"), &ids(&["gone", "b"]), &all),
            ids(&["a", "b", "c", "d"])
        );
    }

    #[test]
    fn stepping_wraps_both_ways() {
        assert_eq!(step(3, 0, 1), 1);
        assert_eq!(step(3, 2, 1), 0);
        assert_eq!(step(3, 0, -1), 2);
        assert_eq!(step(0, 0, 1), 0);
    }
}
