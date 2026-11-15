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
use infiniterm_core::git_tree_status::TreeStatus;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const EVERY: f64 = 2000.;

    fn settle(store: &TreeGitStore, root: &str) {
        let stop = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while store.lock().unwrap().by_dir.get(root).is_some_and(|s| s.in_flight) {
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
        assert!(!ask(&store, "/r", EVERY - 1., EVERY, counted(&runs)), "too soon");
        assert!(ask(&store, "/other", EVERY - 1., EVERY, counted(&runs)), "its own tree");
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
