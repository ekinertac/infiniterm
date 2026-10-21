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

/// The git directory that holds the config for `dir`: its `.git` directory, or
/// where a `.git` file points, through `commondir` for a worktree.
fn config_path(dir: &Path) -> Option<PathBuf> {
    for ancestor in dir.ancestors() {
        let dot = ancestor.join(".git");
        if dot.is_dir() {
            return Some(dot.join("config"));
        }
        if dot.is_file() {
            let text = std::fs::read_to_string(&dot).ok()?;
            let gitdir = text.trim().strip_prefix("gitdir:")?.trim();
            let gitdir = ancestor.join(gitdir);
            let common = std::fs::read_to_string(gitdir.join("commondir"))
                .ok()
                .map(|c| gitdir.join(c.trim()))
                .unwrap_or(gitdir);
            return Some(common.join("config"));
        }
    }
    None
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
}
