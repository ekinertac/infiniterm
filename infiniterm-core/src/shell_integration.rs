//! The zsh integration that makes a card's shell report its commands: two
//! files written under the data dir at launch (`install`), and the
//! environment a card's zsh is started with to load them (`zsh_env`).
//!
//! Why: a card's colour came only from Claude's and Pi's hooks, so a build
//! finishing said nothing. zsh sends no command marks by itself; the
//! integration adds OSC 133 (command started, command finished with its
//! status), which program_state.rs reads off the card's output. It loads
//! through ZDOTDIR, the way Ghostty and VS Code do, so the user's dotfiles
//! are read exactly as before (shell/zshenv.zsh says how).
//!
//! Called from the ui: `install` once at startup (runtime.rs), `zsh_env`
//! when a card's shell is spawned (terminals.rs). Only zsh: it is the macOS
//! default and the only shell anybody here uses; bash and fish can follow
//! when missed. A card made to run one program gets nothing.
use std::path::{Path, PathBuf};

const ZSHENV: &str = include_str!("shell/zshenv.zsh");
const MARKS: &str = include_str!("shell/infiniterm.zsh");

/// Writes the integration into `dir` (created if missing), rewritten every
/// launch so an update's version wins. Returns `dir`.
pub fn install(dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(".zshenv"), ZSHENV)?;
    std::fs::write(dir.join("infiniterm.zsh"), MARKS)?;
    Ok(dir.to_path_buf())
}

/// Whether `shell` (a path or a name) is zsh.
pub fn is_zsh(shell: &str) -> bool {
    Path::new(shell.trim()).file_name().and_then(|n| n.to_str()) == Some("zsh")
}

/// The variables that load the integration into a card's zsh: ZDOTDIR at
/// `dir`, and the user's own ZDOTDIR (`original`, the app's environment)
/// handed over so the shim can put it back. Empty for any other shell.
pub fn zsh_env(shell: &str, dir: &Path, original: Option<String>) -> Vec<(String, String)> {
    if !is_zsh(shell) {
        return vec![];
    }
    let mut env = vec![("ZDOTDIR".to_string(), dir.to_string_lossy().to_string())];
    if let Some(z) = original {
        env.push(("INFINITERM_ZDOTDIR".to_string(), z));
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[test]
    fn only_zsh_gets_the_integration() {
        let d = Path::new("/x");
        assert!(is_zsh("/bin/zsh") && is_zsh("zsh") && is_zsh("/opt/homebrew/bin/zsh"));
        assert!(zsh_env("/bin/bash", d, None).is_empty());
        assert!(zsh_env("fish", d, None).is_empty());
        let e = zsh_env("/bin/zsh", d, Some("/home/me/zd".into()));
        assert_eq!(e[0], ("ZDOTDIR".into(), "/x".into()));
        assert_eq!(e[1], ("INFINITERM_ZDOTDIR".into(), "/home/me/zd".into()));
    }

    /// A real interactive zsh: the user's .zshenv and .zshrc still load,
    /// each command is bracketed with its status, a bare Enter reports
    /// nothing, and a precmd the user added after ours cannot change the
    /// status we report.
    #[test]
    fn a_real_zsh_marks_its_commands_and_keeps_the_users_dotfiles() {
        if !Path::new("/bin/zsh").exists() {
            return;
        }
        let root = std::env::temp_dir().join(format!("ift-zsh-{}", std::process::id()));
        let shim = install(&root.join("shim")).unwrap();
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".zshenv"), "echo USER_ENV\n").unwrap();
        std::fs::write(
            home.join(".zshrc"),
            "echo USER_RC\nother() { return 7 }\nprecmd_functions+=(other)\n",
        )
        .unwrap();
        let mut child = Command::new("/bin/zsh")
            .arg("-i")
            .env_clear()
            .env("HOME", &home)
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", "dumb")
            .envs(zsh_env("/bin/zsh", &shim, None))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"true\nfalse\n\nexit\n")
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            text.contains("USER_ENV") && text.contains("USER_RC"),
            "{text}"
        );
        let marks: Vec<&str> = text
            .split("\x1b]133;")
            .skip(1)
            .map(|s| s.split('\x07').next().unwrap_or(""))
            .collect();
        // `exit` starts a command too, and never reports an end.
        assert_eq!(marks, ["C", "D;0", "C", "D;1", "C"], "{text:?}");
    }
}
