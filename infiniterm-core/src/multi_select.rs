//! Keyboard selection extension and reversal, with focused-first command order.
//! Port of multiSelect.ts and its tests; selection state stays with the caller.
//! Navigation uses one workspace. Missing ids and duplicate ids are skipped.
use crate::{
    cards::PlacedCard,
    navigate::{nearest_in_direction, Direction},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extended {
    pub focused_id: String,
    pub extra: Vec<String>,
}
pub fn extend_selection(
    cards: &[PlacedCard],
    focused_id: &str,
    extra: &[String],
    dir: Direction,
) -> Option<Extended> {
    let next = nearest_in_direction(cards, focused_id, dir)?;
    let mut selected = extra.to_vec();
    if selected.contains(&next.id) {
        selected.retain(|id| id != &next.id);
    } else {
        selected.push(focused_id.into());
    }
    Some(Extended {
        focused_id: next.id.clone(),
        extra: selected,
    })
}
pub fn selected_cards<'a>(
    cards: &'a [PlacedCard],
    focused_id: Option<&str>,
    extra: &[String],
) -> Vec<&'a PlacedCard> {
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for id in focused_id
        .into_iter()
        .chain(extra.iter().map(String::as_str))
    {
        if seen.insert(id) {
            if let Some(card) = cards.iter().find(|c| c.id == id) {
                out.push(card);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::card;
    fn row() -> Vec<PlacedCard> {
        vec![card("a", 0., 0.), card("b", 200., 0.), card("c", 400., 0.)]
    }
    fn strings(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }
    fn ext(id: &str, extra: &[&str], dir: Direction) -> Option<Extended> {
        extend_selection(&row(), id, &strings(extra), dir)
    }
    #[test]
    fn extension_keeps_card_left_behind() {
        assert_eq!(
            ext("a", &[], Direction::Right),
            Some(Extended {
                focused_id: "b".into(),
                extra: strings(&["a"])
            })
        );
        assert_eq!(
            ext("b", &["a"], Direction::Right),
            Some(Extended {
                focused_id: "c".into(),
                extra: strings(&["a", "b"])
            })
        );
    }
    #[test]
    fn reversing_shrinks_selection() {
        assert_eq!(
            ext("c", &["a", "b"], Direction::Left),
            Some(Extended {
                focused_id: "b".into(),
                extra: strings(&["a"])
            })
        );
        assert_eq!(
            ext("b", &["a"], Direction::Left),
            Some(Extended {
                focused_id: "a".into(),
                extra: vec![]
            })
        );
    }
    #[test]
    fn no_extension_at_edge() {
        assert!(ext("c", &["a", "b"], Direction::Right).is_none());
        assert!(ext("a", &[], Direction::Up).is_none());
    }
    #[test]
    fn focused_first_skipping_gone_and_duplicates() {
        let c = row();
        for (focused, extra, expected) in [
            (Some("c"), vec!["a", "zz", "b"], vec!["c", "a", "b"]),
            (None, vec!["b"], vec!["b"]),
            (Some("a"), vec!["a"], vec!["a"]),
        ] {
            assert_eq!(
                selected_cards(&c, focused, &strings(&extra))
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
}
