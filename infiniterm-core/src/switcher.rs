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
//! `row_text` words a row (#746); `Model::switcher_row` gathers its facts
//! and infiniterm-ui/src/switcher_view.rs draws it.

use crate::saved_layout::CardKind;

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

/// What the switcher knows about a card to word its row (#746).
pub struct RowFacts<'a> {
    pub kind: CardKind,
    /// `Model::numbered_label`: the number, the lock, the label.
    pub label: &'a str,
    /// The card has a name the user gave it, so its label is not a folder's.
    pub named: bool,
    /// An editor's explorer root.
    pub root: Option<&'a str>,
    /// An editor's active file.
    pub path: Option<&'a str>,
    /// A browser's active tab's page title (`Card::page_title`).
    pub page_title: Option<&'a str>,
}

/// A switcher row's words: the label, a faint detail after it, and the
/// card's kind for the column on the right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowText {
    pub label: String,
    /// What is open inside: an editor's active file, a browser's page title.
    pub detail: Option<String>,
    /// `None` for a terminal: it is what a card is unless it says otherwise.
    pub kind: Option<&'static str>,
}

/// The row's words. A folder editor's label loses its trailing `/`, the
/// same rule as the editor's folder header (#694): the kind column already
/// says editor, and the active file beside it says what is open. Only here:
/// the chip, the palette and the status bar keep the slash, which there is
/// what tells a folder editor from a file. The filesystem root keeps its
/// `/`, having no name without it.
pub fn row_text(f: &RowFacts) -> RowText {
    let folder = f.kind.has_editor_body() && f.root.is_some() && !f.named;
    let label = match f.label.strip_suffix('/') {
        Some(bare) if folder && !bare.is_empty() && !bare.ends_with(' ') => bare.to_string(),
        _ => f.label.to_string(),
    };
    let detail = match f.kind {
        CardKind::Editor if f.root.is_some() => f.path.map(crate::paths::base_name),
        CardKind::Browser => f.page_title.map(str::trim),
        _ => None,
    }
    .filter(|d| !d.is_empty())
    .map(str::to_string);
    let kind = (f.kind != CardKind::Terminal).then(|| f.kind.as_str());
    RowText {
        label,
        detail,
        kind,
    }
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

    fn facts(kind: CardKind, label: &'static str) -> RowFacts<'static> {
        RowFacts {
            kind,
            label,
            named: false,
            root: None,
            path: None,
            page_title: None,
        }
    }

    #[test]
    fn a_folder_editor_drops_its_slash_and_shows_its_file() {
        let row = row_text(&RowFacts {
            root: Some("/Users/me/infiniterm"),
            path: Some("/Users/me/infiniterm/src/main.rs"),
            ..facts(CardKind::Editor, "#3 infiniterm/")
        });
        assert_eq!(row.label, "#3 infiniterm");
        assert_eq!(row.detail.as_deref(), Some("main.rs"));
        assert_eq!(row.kind, Some("editor"));
        // Nothing open yet: the folder alone.
        let row = row_text(&RowFacts {
            root: Some("/Users/me/infiniterm"),
            ..facts(CardKind::Editor, "#3 infiniterm/")
        });
        assert_eq!(row.detail, None);
    }

    #[test]
    fn the_root_and_a_given_name_keep_their_slash() {
        let root = RowFacts {
            root: Some("/"),
            ..facts(CardKind::Editor, "#3 /")
        };
        assert_eq!(row_text(&root).label, "#3 /");
        let named = RowFacts {
            named: true,
            root: Some("/x"),
            ..facts(CardKind::Editor, "#3 notes/")
        };
        assert_eq!(row_text(&named).label, "#3 notes/");
        // A file editor's label is its file: no detail repeats it.
        let file = RowFacts {
            path: Some("/x/a.rs"),
            ..facts(CardKind::Editor, "#2 a.rs")
        };
        assert_eq!(row_text(&file).detail, None);
    }

    #[test]
    fn a_browser_shows_its_page_title_and_a_terminal_no_kind() {
        let row = row_text(&RowFacts {
            page_title: Some(" Example Domain "),
            ..facts(CardKind::Browser, "#4 example.com")
        });
        assert_eq!(row.detail.as_deref(), Some("Example Domain"));
        assert_eq!(row.kind, Some("browser"));
        let blank = RowFacts {
            page_title: Some(""),
            ..facts(CardKind::Browser, "#4 example.com")
        };
        assert_eq!(row_text(&blank).detail, None);
        let term = row_text(&facts(CardKind::Terminal, "#1 zsh"));
        assert_eq!((term.kind, term.detail), (None, None));
        assert_eq!(
            row_text(&facts(CardKind::Diff, "#5 diff")).kind,
            Some("diff")
        );
    }
}
