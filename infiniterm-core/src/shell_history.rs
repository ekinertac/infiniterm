//! One zsh history file per card, under the data dir, and the line the app
//! writes into it when a card's session died with the machine.
//!
//! Every shell on a Mac starts by reading `~/.zsh_history`, so a new card
//! opens with the history of every other card, which Ekin never liked. The
//! app names a file per card in `INFINITERM_HISTFILE` (`terminals.rs` puts
//! it in the shell's environment) and the user's `.zshrc` takes it:
//! `HISTFILE="${INFINITERM_HISTFILE:-$HOME/.zsh_history}"`. A plain
//! `HISTFILE` in the environment would not do, because macOS's /etc/zshrc
//! sets HISTFILE before .zshrc runs and clobbers it. The card id is stable
//! across restarts, so a card's Up arrow is the card's own past.
//!
//! `append` is for the lost-session notice (`terminal_body::
//! lost_session_tail`): `claude --resume <id>` goes into the card's history
//! before the new shell starts, so Up brings it back with no pasting.
//! Plain lines, not zsh's `: <time>:0;` extended form: zsh reads plain
//! lines under either setting, and the extended form is a literal command
//! starting with `:` under the plain one.
use crate::paths::app_support_dir;
use std::path::PathBuf;

/// Where a card's shell keeps its history. Beside the drafts, by card id.
pub fn history_file(card_id: &str) -> PathBuf {
    app_support_dir().join("history").join(card_id)
}

/// The history file's directory, made if needed, so a shell's first write
/// does not fail on a missing parent (zsh gives up silently on that).
pub fn ensure_dir(path: &std::path::Path) -> std::io::Result<()> {
    match path.parent() {
        Some(dir) => std::fs::create_dir_all(dir),
        None => Ok(()),
    }
}

/// One command onto the end of a history file, as the last thing Up finds.
/// A command with a newline in it would read back as two entries, so it is
/// refused rather than split.
pub fn append(path: &std::path::Path, command: &str) -> std::io::Result<()> {
    if command.contains('\n') || command.trim().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "a history entry is one non-empty line",
        ));
    }
    ensure_dir(path)?;
    let mut text = std::fs::read_to_string(path).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(command);
    text.push('\n');
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("ift-hist-{}-{name}", std::process::id()))
            .join("history")
            .join("card-1")
    }

    #[test]
    fn a_card_has_its_own_file_under_the_data_dir() {
        let p = history_file("abc");
        assert!(p.ends_with("history/abc"), "{p:?}");
        assert!(p.starts_with(app_support_dir()));
    }

    // The resume command must be the LAST line, whatever the file held, and
    // a file that did not end in a newline must not have it glued on.
    #[test]
    fn append_makes_the_command_the_last_entry() {
        let p = scratch("append");
        let _ = std::fs::remove_dir_all(p.parent().unwrap().parent().unwrap());
        append(&p, "claude --resume abc").unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "claude --resume abc\n"
        );
        std::fs::write(&p, "ls\ncd x").unwrap();
        append(&p, "claude --resume abc").unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "ls\ncd x\nclaude --resume abc\n"
        );
        assert!(append(&p, "a\nb").is_err());
        assert!(append(&p, "  ").is_err());
        let _ = std::fs::remove_dir_all(p.parent().unwrap().parent().unwrap());
    }
}
