//! The file tree's git status (`editor.explorer.git`, #798): one answer per
//! tree root, shared by every editor tab and card of the app, asked on a
//! thread and never on the UI thread (#717).
//!
//! `EditorBody::idle` calls `ask` on its disk poll and `get` to pick up a
//! landed answer; the tree painter in `editor_body.rs` reads what `get`
//! returned and `row_style` for each row. Keyed by the TREE ROOT, not the
//! repository, because learning the repository is itself a `git` run: the
//! answer carries its root (`TreeStatus::root`) and `mark_for` handles a
//! tree in a subfolder. Several tabs on one tree therefore share one
//! `git status` per poll. The parsing is `git_tree_status.rs` in core.
use infiniterm_core::git_tree_status::{RowMark, TreeStatus};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Every tree's latest answer, shared across the app.
pub type TreeGitStore = Arc<Mutex<TreeGit>>;

#[derive(Default)]
pub struct TreeGit {
    by_dir: HashMap<String, Slot>,
}

#[derive(Default)]
struct Slot {
    /// `None` before the first answer and outside a repository.
    status: Option<Arc<TreeStatus>>,
    asked_at: Option<f64>,
    in_flight: bool,
}

/// The one store every editor tab shares; a test makes its own.
pub fn shared() -> TreeGitStore {
    static STORE: std::sync::OnceLock<TreeGitStore> = std::sync::OnceLock::new();
    STORE.get_or_init(TreeGitStore::default).clone()
}

/// Asks for `tree_root`'s status on a thread unless an ask is in flight or
/// one started less than `every` ms ago (another tab on the same tree got
/// there first). `run` is the blocking `git` call; `None` means outside a
/// repository and clears the answer. True when it asked.
pub fn ask(
    store: &TreeGitStore,
    tree_root: &str,
    now: f64,
    every: f64,
    run: impl FnOnce() -> Option<TreeStatus> + Send + 'static,
) -> bool {
    {
        let Ok(mut all) = store.lock() else {
            return false;
        };
        let slot = all.by_dir.entry(tree_root.to_string()).or_default();
        if slot.in_flight || slot.asked_at.is_some_and(|at| now - at < every) {
            return false;
        }
        slot.in_flight = true;
        slot.asked_at = Some(now);
    }
    let store = store.clone();
    let root = tree_root.to_string();
    std::thread::spawn(move || {
        let status = run().map(Arc::new);
        if let Ok(mut all) = store.lock() {
            let slot = all.by_dir.entry(root).or_default();
            slot.status = status;
            slot.in_flight = false;
        }
    });
    true
}

/// The latest answer for `tree_root`, if any.
pub fn get(store: &TreeGitStore, tree_root: &str) -> Option<Arc<TreeStatus>> {
    store.lock().ok()?.by_dir.get(tree_root)?.status.clone()
}

/// The git gutter's colours (`editor_body.rs`), here so the tree and the
/// gutter say "added" and "changed" in one colour each. Not agent hues.
pub(crate) const MARK_ADDED: &str = "#4caf50";
pub(crate) const MARK_MODIFIED: &str = "#e3b341";
pub(crate) const MARK_DELETED: &str = "#f85149";
/// How strongly an ignored row is drawn: present, but clearly not part of
/// the work. Tuned on screen against both themes.
pub const IGNORED_ALPHA: f32 = 0.45;

/// How one tree row looks: the letter in the status column, the colour of
/// the letter and the name (`None` keeps the row's own), and its opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowStyle {
    pub letter: &'static str,
    pub tint: Option<&'static str>,
    pub alpha: f32,
}

/// VS Code's letters. A conflict is red, the gutter's deleted colour: the
/// chrome's `warn` is the same yellow as a modified file. A folder shows a
/// dot in the colour of the strongest change below it.
pub fn row_style(mark: Option<RowMark>) -> RowStyle {
    use infiniterm_core::git_tree_status::TreeMark::*;
    let colour = |m| match m {
        Modified => Some(MARK_MODIFIED),
        Added | Untracked | Renamed => Some(MARK_ADDED),
        Conflict => Some(MARK_DELETED),
        Ignored => None,
    };
    let plain = RowStyle {
        letter: " ",
        tint: None,
        alpha: 1.,
    };
    match mark {
        None => plain,
        Some(RowMark::Ignored) => RowStyle {
            alpha: IGNORED_ALPHA,
            ..plain
        },
        Some(RowMark::Folder(m)) => RowStyle {
            letter: "•",
            tint: colour(m),
            alpha: 1.,
        },
        Some(RowMark::File(m)) => RowStyle {
            letter: match m {
                Modified => "M",
                Added => "A",
                Untracked => "U",
                Renamed => "R",
                Conflict => "!",
                Ignored => " ",
            },
            tint: colour(m),
            alpha: 1.,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use infiniterm_core::git_tree_status::TreeMark;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn row_styles() {
        let s = |m| {
            let r = row_style(m);
            (r.letter, r.tint, r.alpha)
        };
        assert_eq!(s(None), (" ", None, 1.));
        assert_eq!(
            s(Some(RowMark::File(TreeMark::Modified))),
            ("M", Some("#e3b341"), 1.)
        );
        assert_eq!(
            s(Some(RowMark::File(TreeMark::Added))),
            ("A", Some("#4caf50"), 1.)
        );
        assert_eq!(
            s(Some(RowMark::File(TreeMark::Untracked))),
            ("U", Some("#4caf50"), 1.)
        );
        assert_eq!(
            s(Some(RowMark::File(TreeMark::Renamed))),
            ("R", Some("#4caf50"), 1.)
        );
        assert_eq!(
            s(Some(RowMark::File(TreeMark::Conflict))),
            ("!", Some("#f85149"), 1.)
        );
        assert_eq!(
            s(Some(RowMark::Folder(TreeMark::Conflict))),
            ("•", Some("#f85149"), 1.)
        );
        assert_eq!(
            s(Some(RowMark::Folder(TreeMark::Modified))),
            ("•", Some("#e3b341"), 1.)
        );
        assert_eq!(s(Some(RowMark::Ignored)), (" ", None, IGNORED_ALPHA));
    }

    const EVERY: f64 = 2000.;

    fn settle(store: &TreeGitStore, root: &str) {
        let stop = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while store
            .lock()
            .unwrap()
            .by_dir
            .get(root)
            .is_some_and(|s| s.in_flight)
        {
            assert!(std::time::Instant::now() < stop, "never answered");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn answer(root: &str) -> Option<TreeStatus> {
        Some(infiniterm_core::git_tree_status::parse(root, " M a\0"))
    }

    // Two tabs on one tree, or one tab polling fast, run git once per poll.
    #[test]
    fn one_ask_per_root_per_poll() {
        let store = TreeGitStore::default();
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = |runs: &Arc<AtomicUsize>| {
            let runs = runs.clone();
            move || {
                runs.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(20));
                answer("/r")
            }
        };
        assert!(ask(&store, "/r", 0., EVERY, counted(&runs)));
        assert!(!ask(&store, "/r", 0., EVERY, counted(&runs)), "in flight");
        settle(&store, "/r");
        assert!(get(&store, "/r").is_some());
        assert!(
            !ask(&store, "/r", EVERY - 1., EVERY, counted(&runs)),
            "too soon"
        );
        assert!(
            ask(&store, "/other", EVERY - 1., EVERY, counted(&runs)),
            "its own tree"
        );
        assert!(ask(&store, "/r", EVERY, EVERY, counted(&runs)));
        settle(&store, "/r");
        settle(&store, "/other");
        assert_eq!(runs.load(Ordering::SeqCst), 3);
    }

    // A tree whose repository went away (or a `git init` undone) loses its
    // marks on the next answer.
    #[test]
    fn none_clears() {
        let store = TreeGitStore::default();
        ask(&store, "/r", 0., EVERY, || answer("/r"));
        settle(&store, "/r");
        assert!(get(&store, "/r").is_some());
        ask(&store, "/r", EVERY, EVERY, || None);
        settle(&store, "/r");
        assert!(get(&store, "/r").is_none());
    }
}
