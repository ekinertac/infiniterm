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
use crate::shell_cmd::ShellKind;

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

/// A path as a POSIX shell can be given it. Anything outside the safe set is
/// single-quoted, and a single quote inside ends the quoting, escapes itself
/// and starts again, which is the only way a POSIX shell accepts one.
///
/// POSIX ONLY. `quote_for` is what a dropped path goes through, because a
/// card's shell is not always one of these; the callers left here are the
/// unix-only ones that write `sh` themselves (tmux's command line, the
/// updater's swap script).
pub fn posix_quote(path: &str) -> String {
    if is_plain(path) {
        return path.to_string();
    }
    format!("'{}'", path.replace('\'', r"'\''"))
}

/// Characters that need no quoting in any shell here.
fn is_plain(path: &str) -> bool {
    let safe = |c: char| c.is_ascii_alphanumeric() || "-_./:@%+=,".contains(c);
    !path.is_empty() && path.chars().all(safe)
}

/// A path as THE CARD'S shell can be given it.
///
/// This is the whole security story of the drop: a file called
/// `; rm -rf ~` has to reach the prompt as text and not as a command. Which
/// means the quoting has to match the shell actually running there, and
/// POSIX quoting does not survive PowerShell — `'it\''s'` ends the string
/// at the second quote and hands the rest to the parser, so a name
/// containing a quote could carry a `;` out of the string with it.
///
/// PowerShell's literal form is the same single quotes with a quote inside
/// DOUBLED, and nothing else is special in it: not `$`, not a backtick, not
/// `;` or `&`. cmd has no literal form at all, so it gets double quotes;
/// see `ShellKind::Cmd` below for what that does not cover.
pub fn quote_for(path: &str, kind: ShellKind) -> String {
    match kind {
        ShellKind::Posix => posix_quote(path),
        ShellKind::PowerShell => {
            if is_plain(path) {
                return path.to_string();
            }
            format!("'{}'", path.replace('\'', "''"))
        }
        // shortcut: double quotes make everything literal to cmd except
        // `%VAR%`, which it still expands, so a file named `%PATH%.txt`
        // pastes as its value. Windows forbids `"` in a file name, so there
        // is nothing to escape and nothing to execute; the ceiling is one
        // wrong-looking paste. cmd is only ever the card's shell when
        // neither PowerShell is installed (`shell_cmd::default_shell`).
        ShellKind::Cmd => format!("\"{path}\""),
    }
}

/// `items` is each dropped path with what the filesystem says it is; the ui
/// asks, because only it can. `target` is the kind of card under the
/// pointer, or `None` for bare canvas.
pub fn drop_plan(
    items: &[(String, PathKind)],
    target: Option<CardKind>,
    shell: ShellKind,
) -> DropAction {
    if items.is_empty() {
        return DropAction::None;
    }
    match target {
        // A terminal is a terminal. One line, so dropping three files into
        // a `tar` command works the way it does everywhere else.
        Some(CardKind::Terminal) => DropAction::Type(
            items
                .iter()
                .map(|(p, _)| quote_for(p, shell))
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

    // The drop's whole security story, per shell. A file name is attacker
    // controlled in the sense that matters: somebody hands you a zip, you
    // drag a file out of it into a card, and whatever the quoting did not
    // cover runs as you.
    //
    // POSIX quoting does NOT survive PowerShell, which is why `quote_for`
    // exists: `'it\''s'` ends PowerShell's string at the second quote and
    // hands the rest to the parser.
    #[test]
    fn a_quote_in_a_name_cannot_carry_a_command_out_of_the_string() {
        // The shape of the attack: close the quoting, run something, reopen.
        let nasty = r"C:\tmp\a'; Remove-Item -Recurse ~; 'b.txt";
        let ps = quote_for(nasty, ShellKind::PowerShell);
        // One string from start to end: every quote inside is doubled, so
        // none of them ends it.
        assert!(ps.starts_with('\'') && ps.ends_with('\''), "{ps}");
        assert_eq!(ps, r"'C:\tmp\a''; Remove-Item -Recurse ~; ''b.txt'");
        // And the POSIX answer for the same name is a different string, which
        // is the entire reason this function takes a shell.
        assert_ne!(quote_for(nasty, ShellKind::Posix), ps);

        let unix = "/tmp/a'; rm -rf ~; 'b.txt";
        assert_eq!(
            quote_for(unix, ShellKind::Posix),
            posix_quote(unix),
            "the posix arm is the posix quoter"
        );
    }

    // What a PowerShell literal string does NOT interpret. Each of these
    // would be a command substitution or a separator unquoted.
    #[test]
    fn powershell_single_quotes_leave_every_other_metacharacter_alone() {
        for name in [
            r"C:\tmp\$(Remove-Item -Recurse ~).txt",
            "C:\\tmp\\`whoami`.txt",
            r"C:\tmp\a; Remove-Item x.txt",
            r"C:\tmp\a & calc.exe",
            r"C:\tmp\a | calc.exe",
            r"C:\tmp\$env:USERPROFILE.txt",
        ] {
            let got = quote_for(name, ShellKind::PowerShell);
            // Quoted whole, and nothing inside was rewritten, so the shell
            // sees exactly the file name.
            assert_eq!(got, format!("'{name}'"), "{name}");
        }
    }

    // A plain path is left bare in either shell, so a drop into a half-typed
    // command still reads as something a person typed.
    #[test]
    fn a_plain_path_is_not_quoted_at_all() {
        assert_eq!(quote_for("a-b_c.1/x", ShellKind::PowerShell), "a-b_c.1/x");
        assert_eq!(quote_for("a-b_c.1/x", ShellKind::Posix), "a-b_c.1/x");
        // A Windows path is not plain: the separator is not in the safe set,
        // so it gets quoted, which is what makes the backslashes literal.
        assert_eq!(
            quote_for(r"C:\Users\PC\a.txt", ShellKind::PowerShell),
            r"'C:\Users\PC\a.txt'"
        );
    }

    // cmd has no literal form; double quotes are the best it offers, and the
    // one thing they do not cover is named where it happens.
    #[test]
    fn cmd_gets_double_quotes_and_the_gap_is_written_down() {
        assert_eq!(
            quote_for(r"C:\tmp\a; calc.exe.txt", ShellKind::Cmd),
            "\"C:\\tmp\\a; calc.exe.txt\""
        );
        // Windows forbids a double quote in a file name, so there is never
        // one to escape.
        assert!(!r"C:\tmp\a.txt".contains('\"'));
    }

    // The dropped line as a card would receive it: several paths, one line.
    #[test]
    fn a_windows_drop_into_a_terminal_quotes_every_path_for_that_shell() {
        let items = vec![
            (r"C:\tmp\a.txt".to_string(), PathKind::File),
            (r"C:\tmp\my file.txt".to_string(), PathKind::File),
        ];
        assert_eq!(
            drop_plan(&items, Some(CardKind::Terminal), ShellKind::PowerShell),
            DropAction::Type(r"'C:\tmp\a.txt' 'C:\tmp\my file.txt'".into())
        );
    }
    use super::*;

    #[test]
    fn a_plain_path_needs_no_quoting() {
        assert_eq!(
            posix_quote("/Users/ekinertac/Code/api"),
            "/Users/ekinertac/Code/api"
        );
        assert_eq!(posix_quote("a-b_c.1/x"), "a-b_c.1/x");
    }

    // The whole security story: a filename is text at the prompt, never a
    // command, however it is spelled.
    #[test]
    fn anything_a_shell_would_read_is_quoted() {
        assert_eq!(posix_quote("/tmp/my file"), "'/tmp/my file'");
        assert_eq!(posix_quote("/tmp/; rm -rf ~"), "'/tmp/; rm -rf ~'");
        assert_eq!(posix_quote("/tmp/$(whoami)"), "'/tmp/$(whoami)'");
        assert_eq!(posix_quote("/tmp/a`b`"), "'/tmp/a`b`'");
        assert_eq!(posix_quote(""), "''");
    }

    // A single quote cannot appear inside single quotes: the quoting stops,
    // an escaped quote goes in, and the quoting starts again.
    #[test]
    fn a_quote_in_a_filename_closes_and_reopens_the_quoting() {
        assert_eq!(posix_quote("/tmp/it's"), r"'/tmp/it'\''s'");
    }

    #[test]
    fn a_terminal_gets_the_paths_as_one_escaped_line() {
        let items = vec![
            ("/tmp/a".to_string(), PathKind::File),
            ("/tmp/my file".to_string(), PathKind::File),
        ];
        assert_eq!(
            drop_plan(&items, Some(CardKind::Terminal), ShellKind::Posix),
            DropAction::Type("/tmp/a '/tmp/my file'".into())
        );
    }

    #[test]
    fn a_browser_goes_to_the_file() {
        let items = vec![("/tmp/a.html".to_string(), PathKind::File)];
        assert_eq!(
            drop_plan(&items, Some(CardKind::Browser), ShellKind::Posix),
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
        let DropAction::Open(plans) = drop_plan(&items, None, ShellKind::Posix) else {
            panic!("bare canvas opens cards");
        };
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0], open_plan("/tmp/a.rs", PathKind::File, None));
        assert_eq!(plans[1], open_plan("/tmp/dir", PathKind::Directory, None));
        // An editor card is showing something; the drop lands beside it.
        assert!(matches!(
            drop_plan(&items, Some(CardKind::Editor), ShellKind::Posix),
            DropAction::Open(_)
        ));
    }

    #[test]
    fn nothing_dropped_is_nothing_done() {
        assert_eq!(drop_plan(&[], None, ShellKind::Posix), DropAction::None);
        assert_eq!(
            drop_plan(&[], Some(CardKind::Terminal), ShellKind::Posix),
            DropAction::None
        );
    }
}
