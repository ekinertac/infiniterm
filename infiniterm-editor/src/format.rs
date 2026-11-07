//! Formatting a buffer through an external formatter (#602): Prettier for the
//! web languages, markdown and YAML, `rustfmt`, `gofmt` and `black` for their
//! own. The text goes to the formatter's stdin and the formatted text comes
//! back on its stdout; nothing is written by the formatter itself. The
//! command is the language's entry in the `editor.formatters.<language>`
//! setting, else the built-in default here.
//!
//! Called by `infiniterm-ui/src/editor_body.rs` (`format_buffer`: on save when
//! `editor.formatOnSave` is on, and from `editor.format`). Local files only:
//! the formatter runs on this machine, in the file's folder so a project's own
//! config (`.prettierrc`, `rustfmt.toml`) is found. Pure apart from the
//! process it runs.
//!
//! A command is words separated by spaces, no shell: `{path}` is replaced by
//! the file's path (Prettier needs it to pick a parser), and the program is
//! looked up in the project's `node_modules/.bin` going up from the file,
//! then on `PATH` and the places a Mac app launched from Finder does not see
//! (`/opt/homebrew/bin`, `~/.cargo/bin` ...).

use crate::language::Language;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A formatter that has not answered in this long is killed and the file is
/// saved as it is: a save must never hang.
pub const TIMEOUT: Duration = Duration::from_secs(8);

/// The built-in command for `lang`, `None` where no formatter is assumed.
pub fn default_command(lang: Language) -> Option<&'static str> {
    Some(match lang {
        Language::JavaScript
        | Language::TypeScript
        | Language::Tsx
        | Language::Json
        | Language::Css
        | Language::Scss
        | Language::Html
        | Language::Markdown
        | Language::Yaml => "prettier --stdin-filepath {path}",
        Language::Rust => "rustfmt --edition 2021",
        Language::Go => "gofmt",
        Language::Python => "black -q -",
        _ => return None,
    })
}

/// Where a Mac app started from Finder does not look but a terminal does.
const EXTRA_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    ".cargo/bin",
    ".asdf/shims",
    ".volta/bin",
    ".local/bin",
];

/// The program `name` for a file in `dir`: the nearest `node_modules/.bin`
/// going up, then each `PATH` directory, then `EXTRA_DIRS` (those starting
/// with `.` are under `$HOME`).
pub fn find_program(name: &str, dir: &Path, path_var: &str, home: &Path) -> Option<PathBuf> {
    if name.contains('/') {
        return Path::new(name).is_file().then(|| PathBuf::from(name));
    }
    let local = dir
        .ancestors()
        .map(|d| d.join("node_modules/.bin").join(name))
        .find(|p| p.is_file());
    if local.is_some() {
        return local;
    }
    let from_path = std::env::split_paths(path_var).map(|d| d.join(name));
    let extra = EXTRA_DIRS.iter().map(|d| {
        if d.starts_with('.') {
            home.join(d).join(name)
        } else {
            Path::new(d).join(name)
        }
    });
    from_path.chain(extra).find(|p| p.is_file())
}

/// `command` split into a program and its arguments, `{path}` filled in.
pub fn split_command(command: &str, path: &str) -> Option<(String, Vec<String>)> {
    let mut words = command
        .split_whitespace()
        .map(|w| w.replace("{path}", path));
    let program = words.next()?;
    Some((program, words.collect()))
}

/// The formatted text, or why there is none: the program is not found, it
/// failed, it printed nothing, or it timed out. `path` is the file's path;
/// the formatter runs in its folder.
pub fn run(command: &str, path: &str, text: &str, timeout: Duration) -> Result<String, String> {
    let (program, args) = split_command(command, path).ok_or("no formatter command")?;
    let dir = Path::new(path).parent().unwrap_or(Path::new("."));
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let path_var = std::env::var("PATH").unwrap_or_default();
    let exe = find_program(&program, dir, &path_var, &home)
        .ok_or_else(|| format!("{program} not found"))?;
    let mut child = Command::new(&exe)
        .args(&args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    // Feed stdin from a thread and drain both pipes from others: a formatter
    // that prints before it has read everything must not deadlock with us.
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let input = text.to_string();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let mut stderr = child.stderr.take().ok_or("no stderr")?;
    let out_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut s);
        s
    });
    let err_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut stderr, &mut s);
        s
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} took too long"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(format!("{program}: {e}")),
        }
    };
    let _ = writer.join();
    let out = out_reader.join().unwrap_or_default();
    let err = err_reader.join().unwrap_or_default();
    if !status.success() {
        let first = err
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("failed");
        return Err(format!("{program}: {}", first.trim()));
    }
    if out.is_empty() && !text.is_empty() {
        return Err(format!("{program} printed nothing"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_splits_into_words_and_fills_the_path() {
        assert_eq!(
            split_command("prettier --stdin-filepath {path}", "/p/a.ts"),
            Some((
                "prettier".to_string(),
                vec!["--stdin-filepath".to_string(), "/p/a.ts".to_string()]
            ))
        );
        assert_eq!(split_command("  ", "/p"), None);
    }

    #[test]
    fn the_defaults_cover_the_web_languages_and_a_few_compilers() {
        assert!(default_command(Language::TypeScript)
            .unwrap()
            .contains("--stdin-filepath {path}"));
        assert_eq!(default_command(Language::Go), Some("gofmt"));
        assert_eq!(default_command(Language::Sql), None);
    }

    #[test]
    fn the_projects_own_binary_wins_over_the_path() {
        let root = std::env::temp_dir().join(format!("ift-fmt-{}", std::process::id()));
        let bin = root.join("node_modules/.bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::write(bin.join("prettier"), "").unwrap();
        let found = find_program(
            "prettier",
            &root.join("src/deep"),
            "",
            Path::new("/nonexistent"),
        );
        assert_eq!(found, Some(bin.join("prettier")));
        assert_eq!(
            find_program("not-a-real-tool", &root, "", Path::new("/nonexistent")),
            None
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn text_goes_through_stdin_and_comes_back_and_failures_are_errors() {
        let dir = std::env::temp_dir();
        let path = dir.join("x.txt").to_string_lossy().into_owned();
        let t = Duration::from_secs(5);
        // `tr` is on every unix PATH the test runs with.
        assert_eq!(
            run("tr a-z A-Z", &path, "abc\n", t),
            Ok("ABC\n".to_string())
        );
        assert!(run("false", &path, "abc", t).is_err());
        assert!(run("definitely-not-installed-xyz", &path, "abc", t)
            .unwrap_err()
            .contains("not found"));
        // A formatter that prints nothing for real text is a failure, not a
        // file emptied.
        assert!(run("true", &path, "abc", t)
            .unwrap_err()
            .contains("nothing"));
        assert_eq!(run("true", &path, "", t), Ok(String::new()));
    }

    #[cfg(unix)]
    #[test]
    fn a_formatter_that_hangs_is_killed() {
        let path = std::env::temp_dir()
            .join("x.txt")
            .to_string_lossy()
            .into_owned();
        let started = Instant::now();
        let err = run("sleep 5", &path, "abc", Duration::from_millis(150)).unwrap_err();
        assert!(err.contains("took too long"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
