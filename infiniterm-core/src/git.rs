//! What git says has changed, for the diff cards.
//!
//! Shells out to `git` rather than linking a git library: the CLI is on every
//! machine this runs on, its output formats are stable, and a library would be
//! the largest dependency in the tree for two questions — which files changed,
//! and what a file looked like at HEAD.
//!
//! Everything is relative to the working tree against HEAD: staged and
//! unstaged together, plus untracked files, which is "what I have done since
//! the last commit" — the diff a person means when they type `ift diff`.
//!
//! From the Tauri app's git.rs unchanged but for the Tauri glue. The diff
//! card element consumes `Changes`; `blame.rs` formats a `BlameLine`.

use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFile {
    /// Relative to the repository root.
    pub path: String,
    pub added: u32,
    pub removed: u32,
    /// "modified", "added", "deleted" or "untracked".
    pub status: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changes {
    /// The repository root, absolute.
    pub repo: String,
    pub files: Vec<ChangedFile>,
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

/// The repository a path is in, or an error saying it is in none.
fn repo_of(path: &Path) -> Result<std::path::PathBuf, String> {
    let dir = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    let root = git(dir, &["rev-parse", "--show-toplevel"])?;
    Ok(std::path::PathBuf::from(root.trim()))
}

/// Parses `git diff --numstat HEAD` output: `added<TAB>removed<TAB>path`, with
/// `-` for binary files.
fn parse_numstat(text: &str) -> Vec<(String, u32, u32)> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let added = parts.next()?.parse().unwrap_or(0);
            let removed = parts.next()?.parse().unwrap_or(0);
            let path = parts.next()?.to_string();
            Some((path, added, removed))
        })
        .collect()
}

/// Changed files under `path` (a directory, or one file), against HEAD.
pub fn git_changes(path: &str) -> Result<Changes, String> {
    let p = Path::new(path);
    let repo = repo_of(p)?;
    // Scope to what was asked for: a file shows only itself, a directory only
    // what is beneath it. Relative to the repo, as git prints paths.
    let scope = p
        .strip_prefix(&repo)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut files: Vec<ChangedFile> = Vec::new();

    let numstat = git(
        &repo,
        &[
            "diff",
            "--numstat",
            "HEAD",
            "--",
            if scope.is_empty() { "." } else { &scope },
        ],
    )?;
    let status = git(
        &repo,
        &[
            "status",
            "--porcelain",
            "--",
            if scope.is_empty() { "." } else { &scope },
        ],
    )?;
    // Status letters per path, to tell an added file from a modified one.
    let mut kinds = std::collections::HashMap::new();
    for line in status.lines() {
        if line.len() < 4 {
            continue;
        }
        let code = &line[..2];
        let file = line[3..].trim_end().to_string();
        let kind = match code.trim() {
            "??" => "untracked",
            c if c.contains('D') => "deleted",
            c if c.contains('A') => "added",
            _ => "modified",
        };
        kinds.insert(file, kind);
    }
    for (file, added, removed) in parse_numstat(&numstat) {
        let status = kinds.get(&file).copied().unwrap_or("modified");
        files.push(ChangedFile {
            path: file,
            added,
            removed,
            status,
        });
    }
    // Untracked files are not in a diff against HEAD; count their lines as
    // additions so the tree shows them the way a first commit would.
    for (file, kind) in &kinds {
        if *kind == "untracked" && !files.iter().any(|f| &f.path == file) {
            let full = repo.join(file);
            if full.is_dir() {
                continue;
            }
            let added = std::fs::read_to_string(&full)
                .map(|s| s.lines().count() as u32)
                .unwrap_or(0);
            files.push(ChangedFile {
                path: file.clone(),
                added,
                removed: 0,
                status: "untracked",
            });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Changes {
        repo: repo.to_string_lossy().into_owned(),
        files,
    })
}

/// A file as it was at HEAD, or empty for one HEAD did not have.
pub fn git_show_head(repo: &str, path: &str) -> Result<String, String> {
    if path.contains("..") {
        return Err("invalid path".into());
    }
    match git(Path::new(repo), &["show", &format!("HEAD:{path}")]) {
        Ok(text) => Ok(text),
        // "exists on disk, but not in 'HEAD'" — a new file; the diff is all of it.
        Err(e) if e.contains("does not exist") || e.contains("exists on disk") => Ok(String::new()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numstat_parses_and_tolerates_binary() {
        let got = parse_numstat("12\t5\tsrc/a.rs\n-\t-\timg.png\n");
        assert_eq!(
            got,
            vec![("src/a.rs".into(), 12, 5), ("img.png".into(), 0, 0)]
        );
    }

    #[test]
    fn this_repository_answers() {
        let here = env!("CARGO_MANIFEST_DIR");
        let changes = git_changes(here).unwrap();
        // The workspace root, whatever the checkout is called.
        assert!(std::path::Path::new(&changes.repo)
            .join("Cargo.toml")
            .is_file());
        assert!(here.starts_with(&changes.repo));
        // HEAD has a Cargo.toml; a file that never existed does not.
        assert!(git_show_head(&changes.repo, "infiniterm-core/Cargo.toml")
            .unwrap()
            .contains("[package]"));
        assert_eq!(
            git_show_head(&changes.repo, "no/such/file.txt").unwrap(),
            ""
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    /// 1-based line in the working tree file.
    pub line: u32,
    /// Short hash, or "0000000" for a line not committed yet.
    pub hash: String,
    pub author: String,
    /// Author time, seconds since the epoch; 0 for uncommitted.
    pub time: u64,
    pub summary: String,
}

/// Parses `git blame --line-porcelain` output. Each line's block starts with
/// `<hash> <orig> <final> [count]`, then headers, then a tab-prefixed line.
fn parse_blame(text: &str) -> Vec<BlameLine> {
    let mut out = Vec::new();
    let mut cur: Option<BlameLine> = None;
    // Headers repeat only for the first line of each commit group; remember
    // what each hash said so later lines can fill in.
    let mut seen: std::collections::HashMap<String, (String, u64, String)> =
        std::collections::HashMap::new();
    for raw in text.lines() {
        if let Some(rest) = raw.strip_prefix('\t') {
            let _ = rest;
            if let Some(mut b) = cur.take() {
                if let Some((a, t, s)) = seen.get(&b.hash) {
                    if b.author.is_empty() {
                        b.author = a.clone();
                        b.time = *t;
                        b.summary = s.clone();
                    }
                }
                seen.entry(b.hash.clone())
                    .or_insert((b.author.clone(), b.time, b.summary.clone()));
                out.push(b);
            }
            continue;
        }
        if cur.is_none() {
            let mut parts = raw.split(' ');
            let hash = parts.next().unwrap_or("").to_string();
            let _orig = parts.next();
            let line: u32 = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            if hash.len() >= 7 && line > 0 {
                cur = Some(BlameLine {
                    line,
                    hash: hash[..7].to_string(),
                    author: String::new(),
                    time: 0,
                    summary: String::new(),
                });
            }
            continue;
        }
        let Some(b) = cur.as_mut() else { continue };
        if let Some(v) = raw.strip_prefix("author ") {
            b.author = v.to_string();
        } else if let Some(v) = raw.strip_prefix("author-time ") {
            b.time = v.parse().unwrap_or(0);
        } else if let Some(v) = raw.strip_prefix("summary ") {
            b.summary = v.to_string();
        }
    }
    out
}

/// Who last touched each line of the working-tree file. Uncommitted lines
/// come back as hash 0000000 with author "Not Committed Yet", which is what
/// git says and what a person wants to see.
pub fn git_blame(repo: &str, path: &str) -> Result<Vec<BlameLine>, String> {
    if path.contains("..") {
        return Err("invalid path".into());
    }
    let text = git(Path::new(repo), &["blame", "--line-porcelain", "--", path])?;
    Ok(parse_blame(&text))
}

#[cfg(test)]
mod blame_tests {
    use super::*;

    #[test]
    fn porcelain_parses_and_fills_repeated_commits() {
        let text = "\
abc1234567 1 1 2
author Ekin
author-time 1700000000
summary first
\tline one
abc1234567 2 2
\tline two
0000000000000000000000000000000000000000 3 3 1
author Not Committed Yet
author-time 0
summary Version of x from x
\tline three
";
        let got = parse_blame(text);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].hash, "abc1234");
        assert_eq!(got[0].author, "Ekin");
        assert_eq!(got[1].line, 2);
        assert_eq!(got[1].author, "Ekin"); // filled from the first block
        assert_eq!(got[1].summary, "first");
        assert_eq!(got[2].hash, "0000000");
        assert_eq!(got[2].author, "Not Committed Yet");
    }

    #[test]
    fn blames_a_tracked_file_here() {
        let here = env!("CARGO_MANIFEST_DIR");
        let repo = git_changes(here).unwrap().repo;
        let got = git_blame(&repo, "infiniterm-core/Cargo.toml").unwrap();
        assert!(got.len() > 5);
        assert!(got.iter().all(|b| b.line > 0 && b.hash.len() == 7));
    }
}
