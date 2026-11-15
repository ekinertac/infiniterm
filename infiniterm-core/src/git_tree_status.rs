//! Git status for the editor's file tree (`editor.explorer.git`, #798): which
//! files are modified, added, untracked, renamed or in conflict, which
//! folders hold such a file, and what git ignores.
//!
//! One `git status --porcelain=v1 -z --ignored` per tree, asked on a thread
//! by `tree_git.rs` in the ui crate (never on the UI thread, #717) and read
//! by the tree painter in `editor_body.rs` through `TreeStatus::mark_for`.
//! `-z` keeps names with spaces, quotes and non-ASCII exact; `--ignored` in
//! its traditional mode reports an ignored folder once (`node_modules/`),
//! not every file in it, which is what keeps this cheap.
//!
//! Paths in git's output are relative to the repository root; the tree asks
//! with absolute paths, so `mark_for` strips `root` first. A tree rooted in a
//! subfolder of the repo therefore works unchanged. Related: `git.rs` (the
//! diff card's status, a different question: numstat and HEAD's text).
use std::collections::HashMap;
use std::path::Path;

/// What git says about one path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeMark {
    Modified,
    Added,
    Untracked,
    Renamed,
    Conflict,
    Ignored,
}

/// What a tree row shows: its own status, the strongest status somewhere
/// below a folder, or that git ignores it (itself or a folder above it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowMark {
    File(TreeMark),
    Folder(TreeMark),
    Ignored,
}

/// One answer for one repository. Keys are repo-relative, `/`-separated,
/// with no trailing slash.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeStatus {
    /// The repository root, absolute.
    pub root: String,
    files: HashMap<String, TreeMark>,
    /// Folder to the strongest status below it, computed once per answer so
    /// painting a row is a lookup, not a walk.
    folders: HashMap<String, TreeMark>,
    ignored_dirs: Vec<String>,
    /// Folders git reports once as new (`?? new/`): everything inside is
    /// untracked, though git lists none of it.
    untracked_dirs: Vec<String>,
}

/// Whether `rel` is `dir` or inside it (a prefix that ends at a `/`).
fn is_under(rel: &str, dir: &str) -> bool {
    rel == dir || (rel.starts_with(dir) && rel.as_bytes().get(dir.len()) == Some(&b'/'))
}

/// Which status wins when a folder holds several: a conflict needs the
/// reader most, then an edit, then a new file. Ignored never rolls up.
pub fn strength(m: TreeMark) -> u8 {
    match m {
        TreeMark::Conflict => 3,
        TreeMark::Modified => 2,
        TreeMark::Added | TreeMark::Untracked | TreeMark::Renamed => 1,
        TreeMark::Ignored => 0,
    }
}

/// The two-letter codes git uses for an unmerged path.
const CONFLICT_CODES: [&str; 7] = ["DD", "AU", "UD", "UA", "DU", "AA", "UU"];

/// `git status --porcelain=v1 -z --ignored` output, read into marks.
pub fn parse(root: &str, porcelain_z: &str) -> TreeStatus {
    let mut s = TreeStatus {
        root: root.trim_end_matches('/').to_string(),
        ..TreeStatus::default()
    };
    // A deleted file has no row but still changes its folders.
    let mut rolled: Vec<(String, TreeMark)> = Vec::new();
    let mut fields = porcelain_z.split('\0');
    while let Some(rec) = fields.next() {
        // "XY path": two status letters, a space, the path.
        let (Some(code), Some(path)) = (rec.get(..2), rec.get(3..)) else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        let (x, y) = (code.as_bytes()[0], code.as_bytes()[1]);
        // A rename or copy carries its source as the next field.
        if matches!(x, b'R' | b'C') || matches!(y, b'R' | b'C') {
            fields.next();
        }
        if code == "!!" {
            match path.strip_suffix('/') {
                Some(d) => s.ignored_dirs.push(d.to_string()),
                None => {
                    s.files.insert(path.to_string(), TreeMark::Ignored);
                }
            }
            continue;
        }
        if code == "??" {
            if let Some(d) = path.strip_suffix('/') {
                s.untracked_dirs.push(d.to_string());
                s.folders.insert(d.to_string(), TreeMark::Untracked);
                rolled.push((d.to_string(), TreeMark::Untracked));
                continue;
            }
        }
        let path = path.to_string();
        let mark = if CONFLICT_CODES.contains(&code) {
            TreeMark::Conflict
        } else if code == "??" {
            TreeMark::Untracked
        } else {
            // The working tree's letter when there is one, as VS Code.
            let letter = if y != b' ' { y } else { x };
            match letter {
                b'A' => TreeMark::Added,
                b'R' | b'C' => TreeMark::Renamed,
                b'D' => {
                    rolled.push((path, TreeMark::Modified));
                    continue;
                }
                _ => TreeMark::Modified,
            }
        };
        rolled.push((path.clone(), mark));
        s.files.insert(path, mark);
    }
    for (path, mark) in rolled {
        let mut at = path.as_str();
        while let Some(cut) = at.rfind('/') {
            at = &at[..cut];
            let slot = s.folders.entry(at.to_string()).or_insert(mark);
            if strength(mark) > strength(*slot) {
                *slot = mark;
            }
        }
    }
    s
}

impl TreeStatus {
    /// The mark for an absolute path in the tree, `None` for a clean path
    /// or one outside the repository.
    pub fn mark_for(&self, abs_path: &str, is_dir: bool) -> Option<RowMark> {
        let rel = abs_path
            .strip_prefix(self.root.as_str())?
            .strip_prefix('/')?
            .trim_end_matches('/');
        if rel.is_empty() {
            return None;
        }
        match self.files.get(rel) {
            Some(TreeMark::Ignored) => return Some(RowMark::Ignored),
            Some(m) => return Some(RowMark::File(*m)),
            None => {}
        }
        if is_dir {
            if let Some(m) = self.folders.get(rel) {
                return Some(RowMark::Folder(*m));
            }
        }
        if self.untracked_dirs.iter().any(|d| is_under(rel, d)) {
            return Some(if is_dir {
                RowMark::Folder(TreeMark::Untracked)
            } else {
                RowMark::File(TreeMark::Untracked)
            });
        }
        self.ignored_dirs
            .iter()
            .any(|d| is_under(rel, d))
            .then_some(RowMark::Ignored)
    }
}

/// Runs git for the repository `dir` is in. Blocks: call it on a thread.
pub fn status(dir: &Path) -> Result<TreeStatus, String> {
    let root = crate::git::git(dir, &["rev-parse", "--show-toplevel"])?;
    let root = root.trim();
    let out = crate::git::git(
        Path::new(root),
        &["status", "--porcelain=v1", "-z", "--ignored"],
    )?;
    Ok(parse(root, &out))
}

#[cfg(test)]
mod tests {
    use super::RowMark::{File, Folder, Ignored};
    use super::TreeMark::*;
    use super::*;

    fn file(s: &TreeStatus, rel: &str) -> Option<RowMark> {
        s.mark_for(&format!("/r/{rel}"), false)
    }
    fn dir(s: &TreeStatus, rel: &str) -> Option<RowMark> {
        s.mark_for(&format!("/r/{rel}"), true)
    }

    // VS Code shows the working tree's letter when there is one: a file
    // staged as new and then edited reads M. A rename's source path is not
    // a row of its own.
    #[test]
    fn letters_follow_the_working_tree_column() {
        let s = parse(
            "/r",
            " M a\0AM b\0MM c\0A  d\0M  e\0R  f\0g\0RM h\0i\0?? j\0",
        );
        for p in ["a", "b", "c", "e", "h"] {
            assert_eq!(file(&s, p), Some(File(Modified)), "{p}");
        }
        assert_eq!(file(&s, "d"), Some(File(Added)));
        assert_eq!(file(&s, "f"), Some(File(Renamed)));
        assert_eq!(file(&s, "j"), Some(File(Untracked)));
        assert_eq!(file(&s, "g"), None, "a rename source");
        assert_eq!(file(&s, "i"), None, "a rename source");
    }

    #[test]
    fn conflict_codes() {
        for code in ["DD", "AU", "UD", "UA", "DU", "AA", "UU"] {
            let s = parse("/r", &format!("{code} x\0"));
            assert_eq!(file(&s, "x"), Some(File(Conflict)), "{code}");
        }
    }

    #[test]
    fn names_with_spaces_and_unicode() {
        let s = parse("/r", "?? my file.txt\0 M ü/ç d.rs\0");
        assert_eq!(file(&s, "my file.txt"), Some(File(Untracked)));
        assert_eq!(file(&s, "ü/ç d.rs"), Some(File(Modified)));
    }

    #[test]
    fn folders_roll_up_strongest() {
        let s = parse("/r", " M src/a/x.rs\0?? src/b.rs\0UU lib/y.rs\0");
        assert_eq!(dir(&s, "src"), Some(Folder(Modified)));
        assert_eq!(dir(&s, "src/a"), Some(Folder(Modified)));
        assert_eq!(dir(&s, "lib"), Some(Folder(Conflict)));
        assert_eq!(dir(&s, "docs"), None);
    }

    // A deleted file has no row, but its folder still changed.
    #[test]
    fn a_deletion_marks_its_folders() {
        let s = parse("/r", " D gone/x.rs\0");
        assert_eq!(dir(&s, "gone"), Some(Folder(Modified)));
        assert_eq!(file(&s, "gone/x.rs"), None);
    }

    #[test]
    fn ignored_folder_dims_its_subtree() {
        let s = parse("/r", "!! node_modules/\0!! .env\0");
        assert_eq!(dir(&s, "node_modules"), Some(Ignored));
        assert_eq!(file(&s, "node_modules/x/y.js"), Some(Ignored));
        assert_eq!(dir(&s, "node_modules/x"), Some(Ignored));
        assert_eq!(file(&s, ".env"), Some(Ignored));
        assert_eq!(dir(&s, "node_modules_x"), None, "a prefix ends at /");
    }

    // Git reports a new folder once, as it does an ignored one: everything
    // inside it is untracked too, and the folder reads as holding new files.
    #[test]
    fn untracked_folder_marks_its_subtree() {
        let s = parse("/r", "?? new/\0");
        assert_eq!(dir(&s, "new"), Some(Folder(Untracked)));
        assert_eq!(file(&s, "new/a/b.rs"), Some(File(Untracked)));
        assert_eq!(dir(&s, "new/a"), Some(Folder(Untracked)));
        assert_eq!(file(&s, "newer"), None);
    }

    #[test]
    fn tracked_change_beats_ignored_folder() {
        let s = parse("/r", "!! build/\0 M build/keep.txt\0");
        assert_eq!(file(&s, "build/keep.txt"), Some(File(Modified)));
    }

    #[test]
    fn outside_root_is_none() {
        let s = parse("/r", " M a\0");
        assert_eq!(s.mark_for("/other/a", false), None);
        assert_eq!(s.mark_for("/rx/a", false), None);
        assert_eq!(s.mark_for("/r", true), None, "the root itself");
    }

    #[test]
    fn tree_in_a_subfolder() {
        let s = parse("/r", " M sub/a.rs\0");
        assert_eq!(file(&s, "sub/a.rs"), Some(File(Modified)));
        assert_eq!(dir(&s, "sub"), Some(Folder(Modified)));
    }

    #[test]
    fn strength_orders_conflict_first() {
        assert!(strength(Conflict) > strength(Modified));
        assert!(strength(Modified) > strength(Added));
        assert_eq!(strength(Added), strength(Untracked));
        assert_eq!(strength(Added), strength(Renamed));
        assert_eq!(strength(TreeMark::Ignored), 0);
    }

    // Runs the real `git`, with the user's config shut out.
    #[test]
    fn real_repo() {
        let dir = std::env::temp_dir().join(format!(
            "ift-git-tree-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("out")).unwrap();
        let ok = std::process::Command::new("git")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        std::fs::write(dir.join(".gitignore"), "out/\n").unwrap();
        std::fs::write(dir.join("out/a"), "x").unwrap();
        std::fs::write(dir.join("new.txt"), "x").unwrap();
        let s = status(&dir).unwrap();
        let root = s.root.clone();
        assert_eq!(
            s.mark_for(&format!("{root}/new.txt"), false),
            Some(File(Untracked))
        );
        assert_eq!(s.mark_for(&format!("{root}/out/a"), false), Some(Ignored));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
