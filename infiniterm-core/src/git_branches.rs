//! The branches behind the palette's "Git: switch branch…" (#750): every
//! branch of a repository as a row, and the `git switch` a pick runs.
//!
//! Called by `infiniterm-ui/src/runtime.rs` for `Effect::ListBranches` (the
//! rows go to `Model::open_branches`) and `Effect::SwitchBranch`. Pure apart
//! from `list` and `switch`, which shell out to `git` the way `git.rs` does;
//! `parse` and `switch_args` are what the tests hold.
//!
//! "Switch", not "checkout": `git switch` (git 2.23) is the command that only
//! changes branches, where `checkout` also restores files. A remote branch
//! with no local one is listed too, and picking it runs `--track`, which
//! makes the local branch and switches to it in one step.

use std::path::Path;
use std::process::Command;

const LOCAL: &str = "refs/heads/";
const REMOTE: &str = "refs/remotes/";

/// What `for-each-ref` prints per ref: `*` on the checked-out one, a tab,
/// the full ref name. A ref name never holds a control character, so the
/// tab splits it right.
const FORMAT: &str = "--format=%(HEAD)\t%(refname)";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// The full ref name (`refs/heads/main`, `refs/remotes/origin/x`): the
    /// palette row's id, unambiguous where a short name is not.
    pub refname: String,
    /// As a person reads it: `main`, `origin/x`.
    pub name: String,
    pub remote: bool,
    pub current: bool,
}

/// The rows of `for-each-ref`'s output: the local branches, the current one
/// first, then the remote branches that have no local branch of the same
/// name (switching to one would only switch to the local). A remote's `HEAD`
/// pointer is not a branch. Otherwise git's order, newest commit first.
pub fn parse(text: &str) -> Vec<Branch> {
    let mut local = vec![];
    let mut remote = vec![];
    for line in text.lines() {
        let Some((head, refname)) = line.split_once('\t') else {
            continue;
        };
        if let Some(name) = refname.strip_prefix(LOCAL) {
            local.push(Branch {
                refname: refname.into(),
                name: name.into(),
                remote: false,
                current: head == "*",
            });
        } else if let Some(name) = refname.strip_prefix(REMOTE) {
            if name.ends_with("/HEAD") {
                continue;
            }
            remote.push(Branch {
                refname: refname.into(),
                name: name.into(),
                remote: true,
                current: false,
            });
        }
    }
    remote.retain(|r| {
        let short = local_name(&r.name);
        !local.iter().any(|l| l.name == short)
    });
    local.sort_by_key(|b| !b.current);
    local.extend(remote);
    local
}

/// The local branch `git switch --track` makes from a remote one: the name
/// after the remote's (`origin/feature/x` is `feature/x`).
fn local_name(remote_name: &str) -> &str {
    remote_name
        .split_once('/')
        .map(|(_, rest)| rest)
        .unwrap_or(remote_name)
}

/// The arguments after `git switch` for a picked row, or None for a ref that
/// is not a branch.
pub fn switch_args(refname: &str) -> Option<Vec<String>> {
    if let Some(name) = refname.strip_prefix(LOCAL) {
        return Some(vec![name.into()]);
    }
    let name = refname.strip_prefix(REMOTE)?;
    Some(vec!["--track".into(), name.into()])
}

/// What a person reads for a ref name: the name without `refs/heads/` or
/// `refs/remotes/`.
pub fn short_name(refname: &str) -> &str {
    refname
        .strip_prefix(LOCAL)
        .or_else(|| refname.strip_prefix(REMOTE))
        .unwrap_or(refname)
}

/// The notice after a switch: the branch now checked out, which for a remote
/// pick is the local one `--track` made, not the remote's name.
pub fn switched_notice(refname: &str) -> String {
    match refname.strip_prefix(REMOTE) {
        Some(name) => format!("switched to {} (tracks {name})", local_name(name)),
        None => format!("switched to {}", short_name(refname)),
    }
}

/// git's refusal on one line, for the notice bar, which draws one line:
/// the lines joined, the `error:` and `fatal:` labels and the closing
/// `Aborting` left out.
pub fn one_line(stderr: &str) -> String {
    stderr
        .lines()
        .map(|l| {
            let l = l.trim();
            l.strip_prefix("error: ")
                .or_else(|| l.strip_prefix("fatal: "))
                .unwrap_or(l)
        })
        .filter(|l| !l.is_empty() && *l != "Aborting")
        .collect::<Vec<_>>()
        .join(" ")
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The repository root `path` (a folder, or a file in one) is in and its
/// branches, or git's error (not a repository, no git).
pub fn list(path: &Path) -> Result<(String, Vec<Branch>), String> {
    let dir = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    let root = git(dir, &["rev-parse", "--show-toplevel"])
        .map_err(|e| one_line(&e))?
        .trim()
        .to_string();
    let text = git(
        Path::new(&root),
        &[
            "for-each-ref",
            "--sort=-committerdate",
            FORMAT,
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    Ok((root, parse(&text)))
}

/// Switch the repository at `repo` to the branch `refname`. The error is
/// git's own message ("Your local changes … would be overwritten").
pub fn switch(repo: &str, refname: &str) -> Result<(), String> {
    let args = switch_args(refname).ok_or_else(|| format!("{refname} is not a branch"))?;
    let mut all = vec!["switch"];
    all.extend(args.iter().map(String::as_str));
    git(Path::new(repo), &all)
        .map(|_| ())
        .map_err(|e| one_line(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(rows: &[Branch]) -> Vec<&str> {
        rows.iter().map(|b| b.name.as_str()).collect()
    }

    #[test]
    fn current_first_then_remotes_without_a_local() {
        let text = " \trefs/heads/feature\n\
                    *\trefs/heads/main\n\
                    \x20\trefs/remotes/origin/HEAD\n\
                    \x20\trefs/remotes/origin/main\n\
                    \x20\trefs/remotes/origin/fix/login\n\
                    \x20\trefs/remotes/upstream/feature\n";
        let rows = parse(text);
        assert_eq!(names(&rows), ["main", "feature", "origin/fix/login"]);
        assert!(rows[0].current && !rows[1].current);
        assert!(rows[2].remote);
        assert_eq!(rows[2].refname, "refs/remotes/origin/fix/login");
    }

    #[test]
    fn detached_head_marks_nothing() {
        let rows = parse(" \trefs/heads/a\n \trefs/heads/b\n");
        assert!(rows.iter().all(|b| !b.current));
        assert_eq!(names(&rows), ["a", "b"]);
    }

    #[test]
    fn switch_args_track_a_remote_branch() {
        assert_eq!(switch_args("refs/heads/main"), Some(vec!["main".into()]));
        assert_eq!(
            switch_args("refs/remotes/origin/feature/x"),
            Some(vec!["--track".into(), "origin/feature/x".into()])
        );
        assert_eq!(switch_args("refs/tags/v1"), None);
        assert_eq!(short_name("refs/remotes/origin/x"), "origin/x");
        assert_eq!(switched_notice("refs/heads/dev"), "switched to dev");
        assert_eq!(
            switched_notice("refs/remotes/origin/feature/x"),
            "switched to feature/x (tracks origin/feature/x)"
        );
    }

    #[test]
    fn a_refusal_reads_on_one_line() {
        let stderr =
            "error: Your local changes to the following files would be overwritten by checkout:\n\
                      \ta.txt\n\
                      Please commit your changes or stash them before you switch branches.\n\
                      Aborting\n";
        assert_eq!(
            one_line(stderr),
            "Your local changes to the following files would be overwritten by checkout: \
             a.txt Please commit your changes or stash them before you switch branches."
        );
        assert_eq!(
            one_line("fatal: not a git repository"),
            "not a git repository"
        );
    }

    /// A real repository: a branch made, listed, switched to.
    #[test]
    fn lists_and_switches_in_a_repository() {
        let dir = std::env::temp_dir().join(format!("ift-branches-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| git(&dir, args).unwrap();
        run(&["init", "-q", "-b", "main"]);
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "one",
        ]);
        run(&["branch", "other"]);
        let (root, rows) = list(&dir).unwrap();
        assert_eq!(rows[0].name, "main");
        assert!(rows[0].current);
        assert!(rows.iter().any(|b| b.name == "other"));
        switch(&root, "refs/heads/other").unwrap();
        assert_eq!(run(&["branch", "--show-current"]).trim(), "other");
        assert!(switch(&root, "refs/heads/missing").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
