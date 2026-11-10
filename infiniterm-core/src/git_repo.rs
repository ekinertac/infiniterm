//! Which GitHub repository a directory belongs to, read from the repo's own
//! files (`.git/config`), so a terminal line saying `#349` can be linked to
//! `github.com/<owner>/<repo>/issues/349` (#335). No `git` process: this runs
//! when a card's directory changes, from the paint path, and reading a small
//! file is cheaper than starting a program.
//!
//! Handles a plain checkout (`.git` a directory) and a worktree or submodule
//! (`.git` a file `gitdir: <path>`, whose config is in the common directory
//! when there is a `commondir` file). The remote is `origin` when there is
//! one, else the first remote on github.com. Called by `terminal_body.rs`;
//! the pattern side is `links.rs`.
//!
//! Also `head_stamp` (#717): the modification times that move when a file's
//! text at HEAD can change, so the editor's gutter (`editor_body.rs`) runs
//! `git` only after a commit, checkout, stage or edit, not on every poll.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub owner: String,
    pub name: String,
}

impl Repo {
    /// The page for issue or pull request `number` (GitHub redirects the
    /// issue address to the pull request when it is one).
    pub fn issue_url(&self, number: &str) -> String {
        format!(
            "https://github.com/{}/{}/issues/{number}",
            self.owner, self.name
        )
    }
}

/// `owner` and `repo` of a github.com remote URL, in any of its spellings:
/// `https://github.com/o/r(.git)`, `git@github.com:o/r.git`,
/// `ssh://git@github.com/o/r`.
pub fn parse_remote(url: &str) -> Option<Repo> {
    let url = url.trim();
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git://github.com/"))
        .or_else(|| {
            // https://user@github.com/o/r
            let (_, after) = url.split_once("://")?;
            after.split_once('@')?.1.strip_prefix("github.com/")
        })?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, name) = rest.split_once('/')?;
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (ok(owner) && ok(name)).then(|| Repo {
        owner: owner.into(),
        name: name.into(),
    })
}

/// The remote URLs of a git config file, `origin` first.
fn remotes(config: &str) -> Vec<String> {
    let mut out: Vec<(bool, String)> = vec![];
    let mut current: Option<String> = None;
    for line in config.lines() {
        let line = line.trim();
        if let Some(section) = line.strip_prefix('[') {
            current = section
                .strip_prefix("remote \"")
                .and_then(|s| s.split('"').next())
                .map(str::to_string);
        } else if let (Some(name), Some(value)) = (&current, line.strip_prefix("url")) {
            if let Some(url) = value.trim_start().strip_prefix('=') {
                out.push((name == "origin", url.trim().to_string()));
            }
        }
    }
    out.sort_by_key(|(origin, _)| !*origin);
    out.into_iter().map(|(_, url)| url).collect()
}

/// The git directories of the checkout whose root is `root`: the one with
/// `HEAD` and `index` (`.git`, or where a `.git` file points), and the common
/// one with `config`, `refs` and `packed-refs` (the same, or through
/// `commondir` for a worktree). `None` when `root` has no `.git`.
fn git_dirs(root: &Path) -> Option<(PathBuf, PathBuf)> {
    let dot = root.join(".git");
    if dot.is_dir() {
        return Some((dot.clone(), dot));
    }
    let text = std::fs::read_to_string(&dot).ok()?;
    let gitdir = root.join(text.trim().strip_prefix("gitdir:")?.trim());
    let common = std::fs::read_to_string(gitdir.join("commondir"))
        .ok()
        .map(|c| gitdir.join(c.trim()))
        .unwrap_or_else(|| gitdir.clone());
    Some((gitdir, common))
}

/// The git directory that holds the config for `dir`: its `.git` directory, or
/// where a `.git` file points, through `commondir` for a worktree.
fn config_path(dir: &Path) -> Option<PathBuf> {
    for ancestor in dir.ancestors() {
        if ancestor.join(".git").exists() {
            return git_dirs(ancestor).map(|(_, common)| common.join("config"));
        }
    }
    None
}

/// What `git show HEAD:<file>` depends on, as modification times, for the
/// editor's gutter (#717): a tab asks git again only when this changes, so
/// an unchanged repo costs a few `stat`s per poll instead of a process.
/// Covers `HEAD` (a checkout, a detached HEAD moving), the branch's ref file
/// and `packed-refs` (a commit, a pull), `index` (a stage or a reset) and
/// the file itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadStamp(Vec<Option<std::time::SystemTime>>);

/// The stamp of `file` in the checkout at `root`. `None` when `HEAD` cannot
/// be read (the repo is gone or the `.git` file points nowhere): then the
/// caller asks git, so a vanished repo is never mistaken for an unchanged one.
pub fn head_stamp(root: &Path, file: &Path) -> Option<HeadStamp> {
    let (gitdir, common) = git_dirs(root)?;
    let head = std::fs::read_to_string(gitdir.join("HEAD")).ok()?;
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let mut times = vec![
        mtime(&gitdir.join("HEAD")),
        mtime(&gitdir.join("index")),
        mtime(&common.join("packed-refs")),
        mtime(file),
    ];
    // A detached HEAD names a commit, not a ref: HEAD's own time covers it.
    if let Some(name) = head.trim().strip_prefix("ref:") {
        times.push(mtime(&common.join(name.trim())));
    }
    Some(HeadStamp(times))
}

/// The GitHub repository `dir` belongs to, if it is a checkout with a
/// github.com remote.
pub fn github_repo(dir: &str) -> Option<Repo> {
    if dir.is_empty() {
        return None;
    }
    let config = std::fs::read_to_string(config_path(Path::new(dir))?).ok()?;
    remotes(&config).iter().find_map(|u| parse_remote(u))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spelling_of_a_github_remote_is_read() {
        let want = Some(Repo {
            owner: "curiousgamesdev".into(),
            name: "CG-RGS".into(),
        });
        for url in [
            "https://github.com/curiousgamesdev/CG-RGS",
            "https://github.com/curiousgamesdev/CG-RGS.git",
            "https://user@github.com/curiousgamesdev/CG-RGS.git",
            "git@github.com:curiousgamesdev/CG-RGS.git",
            "ssh://git@github.com/curiousgamesdev/CG-RGS",
            "git://github.com/curiousgamesdev/CG-RGS.git",
            " https://github.com/curiousgamesdev/CG-RGS/ ",
        ] {
            assert_eq!(parse_remote(url), want, "{url}");
        }
        assert_eq!(parse_remote("https://gitlab.com/o/r.git"), None);
        assert_eq!(parse_remote("git@github.com:onlyowner"), None);
        assert_eq!(parse_remote("/some/local/path"), None);
    }

    #[test]
    fn the_issue_address_is_the_repos_issues_page() {
        let r = Repo {
            owner: "ekinertac".into(),
            name: "infiniterm".into(),
        };
        assert_eq!(
            r.issue_url("349"),
            "https://github.com/ekinertac/infiniterm/issues/349"
        );
    }

    #[test]
    fn origin_wins_over_another_remote() {
        let config = "[core]\n\turl = not-a-remote\n[remote \"upstream\"]\n\turl = https://github.com/a/up.git\n[remote \"origin\"]\n\turl = git@github.com:b/mine.git\n";
        assert_eq!(remotes(config)[0], "git@github.com:b/mine.git");
        let repo = remotes(config).iter().find_map(|u| parse_remote(u));
        assert_eq!(repo.unwrap().name, "mine");
    }

    #[test]
    fn a_checkout_and_a_worktree_both_find_the_repo() {
        let dir = std::env::temp_dir().join(format!("ift-repo-{}", std::process::id()));
        let main = dir.join("main");
        std::fs::create_dir_all(main.join(".git")).unwrap();
        std::fs::create_dir_all(main.join("src/deep")).unwrap();
        std::fs::write(
            main.join(".git/config"),
            "[remote \"origin\"]\n\turl = git@github.com:o/r.git\n",
        )
        .unwrap();
        // A worktree: a `.git` file, a gitdir with a commondir back to main.
        let wt = dir.join("wt");
        let wtgit = main.join(".git/worktrees/wt");
        std::fs::create_dir_all(&wtgit).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", wtgit.display())).unwrap();
        std::fs::write(wtgit.join("commondir"), "../..\n").unwrap();
        let want = Some(Repo {
            owner: "o".into(),
            name: "r".into(),
        });
        assert_eq!(github_repo(main.join("src/deep").to_str().unwrap()), want);
        assert_eq!(github_repo(wt.to_str().unwrap()), want);
        assert_eq!(github_repo("/definitely/not/a/dir"), None);
        assert_eq!(github_repo(""), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Sets `path`'s modification time `secs` after an old fixed time, so two
    /// bumps in the same second still differ.
    fn bump(path: &Path, secs: u64) {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000 + secs);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    /// A checkout at `root` on branch main, with a file `a.rs`.
    fn fake_checkout(gitdir: &Path, common: &Path, root: &Path) {
        std::fs::create_dir_all(common.join("refs/heads")).unwrap();
        std::fs::create_dir_all(gitdir).unwrap();
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(gitdir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(gitdir.join("index"), "").unwrap();
        std::fs::write(common.join("refs/heads/main"), "abc\n").unwrap();
        std::fs::write(common.join("refs/heads/other"), "def\n").unwrap();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    }

    #[test]
    fn the_head_stamp_moves_on_a_commit_a_checkout_a_stage_and_an_edit_only() {
        let dir = std::env::temp_dir().join(format!("ift-stamp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let git = dir.join(".git");
        fake_checkout(&git, &git, &dir);
        let file = dir.join("a.rs");
        let stamp = || head_stamp(&dir, &file).expect("a checkout");
        let first = stamp();
        assert_eq!(stamp(), first, "nothing changed");
        bump(&git.join("refs/heads/main"), 1);
        let committed = stamp();
        assert_ne!(committed, first, "a commit moves the branch's ref");
        bump(&git.join("refs/heads/other"), 2);
        assert_eq!(stamp(), committed, "another branch's ref is not ours");
        std::fs::write(git.join("HEAD"), "ref: refs/heads/other\n").unwrap();
        bump(&git.join("HEAD"), 3);
        let switched = stamp();
        assert_ne!(switched, committed, "a checkout moves HEAD");
        bump(&git.join("index"), 4);
        let staged = stamp();
        assert_ne!(staged, switched, "a stage moves the index");
        bump(&file, 5);
        assert_ne!(stamp(), staged, "an edit of the file");
        // Gone: no stamp, so the caller asks git and learns there is no repo.
        std::fs::remove_dir_all(&git).unwrap();
        assert_eq!(head_stamp(&dir, &file), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_worktree_stamps_its_own_head_and_the_common_refs_and_detached_is_head_alone() {
        let dir = std::env::temp_dir().join(format!("ift-stamp-wt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let common = dir.join("main/.git");
        let gitdir = common.join("worktrees/wt");
        let wt = dir.join("wt");
        fake_checkout(&gitdir, &common, &wt);
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
        std::fs::write(gitdir.join("commondir"), "../..\n").unwrap();
        let file = wt.join("a.rs");
        let first = head_stamp(&wt, &file).expect("a worktree");
        bump(&common.join("refs/heads/main"), 1);
        let committed = head_stamp(&wt, &file).unwrap();
        assert_ne!(committed, first, "the branch ref lives in the common dir");
        bump(&gitdir.join("index"), 2);
        assert_ne!(head_stamp(&wt, &file).unwrap(), committed, "its own index");
        // Detached: HEAD holds a commit id.
        std::fs::write(gitdir.join("HEAD"), "0123abcd\n").unwrap();
        bump(&gitdir.join("HEAD"), 3);
        let detached = head_stamp(&wt, &file).unwrap();
        bump(&common.join("refs/heads/main"), 4);
        assert_eq!(
            head_stamp(&wt, &file).unwrap(),
            detached,
            "no branch to follow"
        );
        bump(&gitdir.join("HEAD"), 5);
        assert_ne!(head_stamp(&wt, &file).unwrap(), detached);
        std::fs::remove_dir_all(&dir).ok();
    }
}
