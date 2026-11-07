//! The file list behind the editor's Go to File (Cmd+P, #602): every file
//! under a folder as a path relative to it, from `git ls-files` in a repo
//! (fast, and it leaves out what .gitignore leaves out) and otherwise from a
//! walk with a cap.
//!
//! Called by `infiniterm-ui/src/editors.rs` when the model asks
//! (`EditorAction::GoToFile`), which hands the list to
//! `Model::open_quick_files`. Pure apart from `list_local`'s one `git` run;
//! `walk` takes the folder reader as a closure so a test needs no disk and a
//! host's file source works the same as this machine's.

use std::collections::VecDeque;

/// The most files one list holds: a palette that ranks every row on every
/// keystroke must stay quick, and a folder this big is not "minor edits".
pub const CAP: usize = 20_000;

/// A host's folders each cost a round trip, so its walk stops sooner.
pub const REMOTE_CAP: usize = 3_000;

/// Folders a walk never enters: version control's own, and the build and
/// package output that fills a project with files nobody opens by name.
const SKIPPED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    ".next",
];

pub fn skips_dir(name: &str) -> bool {
    SKIPPED_DIRS.contains(&name)
}

/// The paths `git ls-files -z` printed, sorted, without repeats, and cut at
/// `cap`. A path with a newline in it survives because of the NUL split.
pub fn from_git(stdout: &str, cap: usize) -> Vec<String> {
    let mut files: Vec<String> = stdout
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect();
    files.sort_unstable();
    files.dedup();
    files.truncate(cap);
    files
}

/// Every file under `root`, breadth first so the shallow ones are kept when
/// the cap bites, as paths relative to `root` with `/` separators. `read`
/// answers a folder (absolute path) with its entries as `(name, is_dir)`.
pub fn walk(
    root: &str,
    cap: usize,
    read: &mut dyn FnMut(&str) -> Vec<(String, bool)>,
) -> Vec<String> {
    let root = root.trim_end_matches('/');
    let mut files = Vec::new();
    let mut queue: VecDeque<String> = VecDeque::from([String::new()]);
    while let Some(rel) = queue.pop_front() {
        let dir = if rel.is_empty() {
            root.to_string()
        } else {
            format!("{root}/{rel}")
        };
        let mut entries = read(&dir);
        entries.sort();
        for (name, is_dir) in entries {
            let path = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if is_dir {
                if !skips_dir(&name) {
                    queue.push_back(path);
                }
            } else if name != ".DS_Store" {
                files.push(path);
                if files.len() >= cap {
                    files.sort_unstable();
                    return files;
                }
            }
        }
    }
    files.sort_unstable();
    files
}

/// The top of the git repository `dir` is in, on this machine.
pub fn repo_root(dir: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["-C", dir, "rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!root.is_empty()).then_some(root)
}

/// This machine's `git ls-files` under `root` (tracked and untracked, not
/// ignored), or `None` outside a repository or without git.
pub fn list_local_git(root: &str, cap: usize) -> Option<Vec<String>> {
    let out = std::process::Command::new("git")
        .args([
            "-C",
            root,
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let files = from_git(&String::from_utf8_lossy(&out.stdout), cap);
    (!files.is_empty()).then_some(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_output_is_split_on_nul_sorted_and_cut() {
        let out = "b.rs\0a/x.rs\0a/x.rs\0\0c y.rs\0";
        assert_eq!(from_git(out, 10), vec!["a/x.rs", "b.rs", "c y.rs"]);
        assert_eq!(from_git(out, 2), vec!["a/x.rs", "b.rs"]);
        assert!(from_git("", 10).is_empty());
    }

    fn tree(dir: &str) -> Vec<(String, bool)> {
        let e = |n: &str, d: bool| (n.to_string(), d);
        match dir {
            "/p" => vec![
                e("src", true),
                e(".git", true),
                e("a.md", false),
                e(".DS_Store", false),
            ],
            "/p/src" => vec![e("main.rs", false), e("deep", true), e("target", true)],
            "/p/src/deep" => vec![e("z.rs", false)],
            _ => vec![],
        }
    }

    #[test]
    fn a_walk_lists_relative_paths_and_skips_junk_folders() {
        let files = walk("/p/", CAP, &mut |d| tree(d));
        assert_eq!(files, vec!["a.md", "src/deep/z.rs", "src/main.rs"]);
    }

    #[test]
    fn the_cap_keeps_the_shallow_files() {
        let files = walk("/p", 2, &mut |d| tree(d));
        assert_eq!(files, vec!["a.md", "src/main.rs"]);
    }
}
