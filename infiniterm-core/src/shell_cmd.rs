//! Which shell a card gets, and how to tell it to run one command.
//!
//! Two callers build the same command line and must not drift: this
//! process (`backend/local_pty.rs`) and `iftd` (`infiniterm-session`),
//! which spawns a card's shell on the daemon backend. So the decision is a
//! pure function here rather than an argument list written out twice.
//!
//! The split that matters is per shell, not per OS: `-lc` is what a POSIX
//! shell takes, `-NoLogo -Command` is PowerShell's, `/C` is cmd's, and a
//! Windows box with Git Bash on it can have all three. The shell's file
//! name is the only thing the choice is made from.
//!
//! Related: `backend/local_pty.rs` (the caller, and the env scrubbing that
//! goes with the spawn), `config.rs` (`terminal.decoyCommand`, which is run
//! through the same path), docs/windows-handoff.md.

/// The shell a card starts when nothing else says otherwise.
///
/// unix: `$SHELL`, else zsh, which is macOS's default.
///
/// Windows: PowerShell 7 if it is installed, else the PowerShell that ships
/// with the OS, else whatever `%COMSPEC%` names. `$SHELL` is ignored here on
/// purpose: a Git Bash or MSYS session sets it to a path like `/usr/bin/bash`
/// that only makes sense inside that environment, and a card is not inside it.
pub fn default_shell() -> String {
    #[cfg(windows)]
    {
        if let Some(pwsh) = on_path("pwsh.exe") {
            return pwsh;
        }
        if let Some(ps) = on_path("powershell.exe") {
            return ps;
        }
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    }
    #[cfg(unix)]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string())
    }
}

/// The first directory on `PATH` holding `exe`, as a full path. A full path
/// rather than the bare name so the spawn cannot pick up a different one
/// later, and so a card's label says which shell it really is.
#[cfg(windows)]
fn on_path(exe: &str) -> Option<String> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(exe))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

/// What kind of command line `shell` speaks, from its file name alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    /// zsh, bash, sh, fish, dash: `-lc <cmd>`.
    Posix,
    /// pwsh and powershell: `-NoLogo -Command <cmd>`.
    PowerShell,
    /// cmd.exe: `/C <cmd>`.
    Cmd,
}

/// Matched on the file stem, lowercased, so a full path and a `.exe`
/// suffix both land on the right answer. Anything unrecognised is POSIX:
/// that is the overwhelming majority of shells, and it is also what every
/// canvas saved before Windows existed assumed.
pub fn kind_of(shell: &str) -> ShellKind {
    let name = shell.rsplit(['/', '\\']).next().unwrap_or(shell);
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    match stem.to_ascii_lowercase().as_str() {
        "pwsh" | "powershell" => ShellKind::PowerShell,
        "cmd" => ShellKind::Cmd,
        _ => ShellKind::Posix,
    }
}

/// The arguments that make `shell` run `cmd`, or start interactively when
/// there is no command.
///
/// `-lc` on the POSIX side is deliberate: a login shell so the user's own
/// profile runs first, which is what a terminal app does and what makes a
/// card's PATH match the one in their other terminal.
pub fn shell_args(shell: &str, cmd: Option<&str>) -> Vec<String> {
    let kind = kind_of(shell);
    match (kind, cmd) {
        (ShellKind::Posix, None) => vec![],
        (ShellKind::Posix, Some(c)) => vec!["-lc".into(), c.into()],
        // -NoLogo for the interactive case too: the copyright banner is
        // noise in a card, and PowerShell prints it on every start.
        (ShellKind::PowerShell, None) => vec!["-NoLogo".into()],
        (ShellKind::PowerShell, Some(c)) => {
            vec!["-NoLogo".into(), "-Command".into(), c.into()]
        }
        (ShellKind::Cmd, None) => vec![],
        (ShellKind::Cmd, Some(c)) => vec!["/C".into(), c.into()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_path_and_an_exe_suffix_reach_the_same_answer() {
        assert_eq!(kind_of("/bin/zsh"), ShellKind::Posix);
        assert_eq!(kind_of("zsh"), ShellKind::Posix);
        assert_eq!(
            kind_of(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            ShellKind::PowerShell
        );
        assert_eq!(kind_of("pwsh"), ShellKind::PowerShell);
        assert_eq!(kind_of("PowerShell.EXE"), ShellKind::PowerShell);
        assert_eq!(kind_of(r"C:\Windows\System32\cmd.exe"), ShellKind::Cmd);
    }

    // Git Bash on Windows is a POSIX shell at a Windows path; the OS is not
    // what decides.
    #[test]
    fn a_posix_shell_at_a_windows_path_is_still_posix() {
        assert_eq!(
            kind_of(r"C:\Program Files\Git\bin\bash.exe"),
            ShellKind::Posix
        );
        assert_eq!(
            shell_args(r"C:\Program Files\Git\bin\bash.exe", Some("echo hi")),
            vec!["-lc", "echo hi"]
        );
    }

    #[test]
    fn an_unknown_shell_is_treated_as_posix() {
        assert_eq!(kind_of("/usr/local/bin/nu"), ShellKind::Posix);
        assert_eq!(kind_of(""), ShellKind::Posix);
    }

    #[test]
    fn each_shell_gets_its_own_run_flags() {
        assert_eq!(shell_args("/bin/zsh", Some("ls")), vec!["-lc", "ls"]);
        assert_eq!(
            shell_args("pwsh.exe", Some("ls")),
            vec!["-NoLogo", "-Command", "ls"]
        );
        assert_eq!(shell_args("cmd.exe", Some("ls")), vec!["/C", "ls"]);
    }

    // No command means an interactive shell; only PowerShell needs a flag,
    // to keep its banner out of the card.
    #[test]
    fn an_interactive_shell_takes_no_command_flags() {
        assert!(shell_args("/bin/zsh", None).is_empty());
        assert!(shell_args("cmd.exe", None).is_empty());
        assert_eq!(shell_args("pwsh.exe", None), vec!["-NoLogo"]);
    }

    // The command is one argument, never split: a card's command has spaces,
    // quotes and pipes in it and the shell is what parses them.
    #[test]
    fn the_command_stays_one_argument() {
        let args = shell_args("/bin/zsh", Some("echo 'a b' | wc -l"));
        assert_eq!(args.len(), 2);
        assert_eq!(args[1], "echo 'a b' | wc -l");
    }

    #[test]
    fn the_default_shell_is_something_this_machine_has() {
        let shell = default_shell();
        assert!(!shell.is_empty());
        #[cfg(windows)]
        assert!(
            matches!(kind_of(&shell), ShellKind::PowerShell | ShellKind::Cmd),
            "{shell}"
        );
    }
}
