//! What a file dragged from the Finder onto the canvas does, decided here
//! so the rule is one pure function rather than four branches in a paint
//! file.
//!
//! The rule is what the thing under the pointer IS. A terminal is a
//! terminal: it gets the path as text, shell-escaped, the way iTerm2 and
//! Terminal.app have always behaved, and the drop is useful in the middle
//! of a half-typed command. Everything else follows the app's own rule that
//! a path means a card, so the file opens as one.
//!
//! Called by `infiniterm-ui/src/input.rs` from gpui's `on_drop`. Related:
//! ift.rs, which owns `open_plan` and is the same decision reached by
//! typing instead of dragging.
use crate::ift::{open_plan, OpenPlan, PathKind};
use crate::saved_layout::CardKind;

/// What the ui should do with a drop.
#[derive(Clone, Debug, PartialEq)]
pub enum DropAction {
    /// Into a terminal, as if pasted: already escaped and space-separated.
    Type(String),
    /// A browser card goes to the file itself.
    Navigate(String),
    /// One card per path, beside the card dropped on or in the free slot
    /// under the pointer.
    Open(Vec<OpenPlan>),
    /// Nothing droppable: no paths, or none that survived.
    None,
}

/// A path as a shell can be given it. Anything outside the safe set is
/// single-quoted, and a single quote inside ends the quoting, escapes
/// itself and starts again, which is the only way a shell accepts one.
///
/// This is the whole security story of the feature: a file called
/// `; rm -rf ~` reaches the prompt as text and not as a command.
pub fn shell_quote(path: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "-_./:@%+=,".contains(c);
    if !path.is_empty() && path.chars().all(safe) {
        return path.to_string();
    }
    format!("'{}'", path.replace('\'', r"'\''"))
}

/// `items` is each dropped path with what the filesystem says it is; the ui
/// asks, because only it can. `target` is the kind of card under the
/// pointer, or `None` for bare canvas.
pub fn drop_plan(items: &[(String, PathKind)], target: Option<CardKind>) -> DropAction {
    if items.is_empty() {
        return DropAction::None;
    }
    match target {
        // A terminal is a terminal. One line, so dropping three files into
        // a `tar` command works the way it does everywhere else.
        Some(CardKind::Terminal) => DropAction::Type(
            items
                .iter()
                .map(|(p, _)| shell_quote(p))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        // The page goes to the file, as it does in a browser.
        Some(CardKind::Browser) => {
            let (path, _) = &items[0];
            DropAction::Navigate(format!("file://{path}"))
        }
        // An editor, a diff or a transcript is showing something already:
        // the drop opens BESIDE it rather than replacing what is on screen.
        _ => DropAction::Open(
            items
                .iter()
                .map(|(p, kind)| open_plan(p, *kind, None))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_path_needs_no_quoting() {
        assert_eq!(
            shell_quote("/Users/ekinertac/Code/api"),
            "/Users/ekinertac/Code/api"
        );
        assert_eq!(shell_quote("a-b_c.1/x"), "a-b_c.1/x");
    }

    // The whole security story: a filename is text at the prompt, never a
    // command, however it is spelled.
    #[test]
    fn anything_a_shell_would_read_is_quoted() {
        assert_eq!(shell_quote("/tmp/my file"), "'/tmp/my file'");
        assert_eq!(shell_quote("/tmp/; rm -rf ~"), "'/tmp/; rm -rf ~'");
        assert_eq!(shell_quote("/tmp/$(whoami)"), "'/tmp/$(whoami)'");
        assert_eq!(shell_quote("/tmp/a`b`"), "'/tmp/a`b`'");
        assert_eq!(shell_quote(""), "''");
    }

    // A single quote cannot appear inside single quotes: the quoting stops,
    // an escaped quote goes in, and the quoting starts again.
    #[test]
    fn a_quote_in_a_filename_closes_and_reopens_the_quoting() {
        assert_eq!(shell_quote("/tmp/it's"), r"'/tmp/it'\''s'");
    }

    #[test]
    fn a_terminal_gets_the_paths_as_one_escaped_line() {
        let items = vec![
            ("/tmp/a".to_string(), PathKind::File),
            ("/tmp/my file".to_string(), PathKind::File),
        ];
        assert_eq!(
            drop_plan(&items, Some(CardKind::Terminal)),
            DropAction::Type("/tmp/a '/tmp/my file'".into())
        );
    }

    #[test]
    fn a_browser_goes_to_the_file() {
        let items = vec![("/tmp/a.html".to_string(), PathKind::File)];
        assert_eq!(
            drop_plan(&items, Some(CardKind::Browser)),
            DropAction::Navigate("file:///tmp/a.html".into())
        );
    }

    // Everywhere else a path means a card, which is the rule `ift <path>`
    // already follows: a file opens as an editor, a directory as one with
    // its tree.
    #[test]
    fn everywhere_else_a_path_becomes_a_card() {
        let items = vec![
            ("/tmp/a.rs".to_string(), PathKind::File),
            ("/tmp/dir".to_string(), PathKind::Directory),
        ];
        let DropAction::Open(plans) = drop_plan(&items, None) else {
            panic!("bare canvas opens cards");
        };
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0], open_plan("/tmp/a.rs", PathKind::File, None));
        assert_eq!(plans[1], open_plan("/tmp/dir", PathKind::Directory, None));
        // An editor card is showing something; the drop lands beside it.
        assert!(matches!(
            drop_plan(&items, Some(CardKind::Editor)),
            DropAction::Open(_)
        ));
    }

    #[test]
    fn nothing_dropped_is_nothing_done() {
        assert_eq!(drop_plan(&[], None), DropAction::None);
        assert_eq!(drop_plan(&[], Some(CardKind::Terminal)), DropAction::None);
    }
}
