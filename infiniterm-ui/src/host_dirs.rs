//! The folders an editor card's file tree shows, from wherever the card's
//! files are (#560, step 3). On this Mac's disk a folder is listed on the
//! spot, as it always was. On a host's disk a listing takes a round trip, so
//! it is asked on a thread and taken in `tick`; the tree holds the entries
//! (`Tree::set_listing`), this holds what the tree cannot: which folders are
//! being asked, which failed and why, and which are watched.
//!
//! Where it fits: `editor_body.rs` owns one per editor body and passes its
//! `list` as the tree's lister, calls `tick` from `idle`, and draws
//! `status` as the tree's one line of text. The wire is file_source.rs's
//! `FileSource` (`HostSource` over ssh, in a viewer).
//!
//! Constraints: the lister answers an unlisted host folder with nothing and
//! starts the ask, so the tree shows it empty until `tick` hands over the
//! entries; the root says "Loading from the host..." meanwhile. A watch
//! replaces the periodic re-list of a folder; a folder with no watch (an
//! older helper, a lost connection) is re-listed every `POLL_MS` instead.
//! A folder that vanished (`not_found`) lists as empty and its parent's
//! next listing drops it; any other failure keeps the old entries and is
//! said once. Related: infiniterm-editor's explorer.rs, backend/file_client.rs.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use infiniterm_core::file_source::{Code, FileError, FileSource, ListEntry, Watch};
use infiniterm_editor::explorer::Entry;

/// How often a folder with no watch is listed again: the editor's disk poll.
pub const POLL_MS: f64 = 2000.;

/// What the tree's root says while it has nothing to show.
pub const LOADING: &str = "Loading from the host...";

enum WatchSlot {
    Starting,
    On(Watch),
    /// The helper does not watch (an older host): the folder is polled, and
    /// a watch is not asked for again.
    Unsupported,
    /// A watch could not be set now (the link is down): the folder is polled
    /// and a watch is tried again with the next poll.
    Polled,
}

/// What `tick` found, for the body to apply.
#[derive(Default)]
pub struct Updates {
    /// Folders whose entries arrived or changed.
    pub listings: Vec<(String, Vec<Entry>)>,
    /// Sentences for the card to say.
    pub notices: Vec<String>,
}

/// What the tree's root has to say instead of rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Loading,
    Failed(String),
}

impl Status {
    pub fn text(&self) -> String {
        match self {
            Status::Loading => LOADING.to_string(),
            Status::Failed(why) => format!("Could not list this folder on the host: {why}"),
        }
    }
}

type Answer = (String, Result<Vec<Entry>, FileError>);

pub struct HostDirs {
    source: Arc<dyn FileSource>,
    /// Folders with an ask in flight.
    asking: HashSet<String>,
    /// Folders listed at least once.
    listed: HashSet<String>,
    /// Why a folder's last listing failed; the text stays until one works.
    failed: HashMap<String, String>,
    /// Folders to ask again as soon as the answer in flight lands.
    again: HashSet<String>,
    answers: Receiver<Answer>,
    answer_tx: Sender<Answer>,
    watches: HashMap<String, WatchSlot>,
    started: Receiver<(String, Result<Watch, FileError>)>,
    started_tx: Sender<(String, Result<Watch, FileError>)>,
    last_poll: f64,
}

/// A path inside a host folder. Joined as text: the host's separator is
/// not this machine's.
pub fn join(dir: &str, name: &str) -> String {
    format!("{}/{name}", dir.trim_end_matches('/'))
}

fn entries(dir: &str, list: Vec<ListEntry>) -> Vec<Entry> {
    list.into_iter()
        .map(|e| Entry {
            path: join(dir, &e.name),
            name: e.name,
            is_dir: e.is_dir,
        })
        .collect()
}

impl HostDirs {
    pub fn new(source: Arc<dyn FileSource>) -> HostDirs {
        let (answer_tx, answers) = channel();
        let (started_tx, started) = channel();
        HostDirs {
            source,
            asking: HashSet::new(),
            listed: HashSet::new(),
            failed: HashMap::new(),
            again: HashSet::new(),
            answers,
            answer_tx,
            watches: HashMap::new(),
            started,
            started_tx,
            last_poll: 0.,
        }
    }

    /// Whether this was made for `source`.
    pub fn is_for(&self, source: &Arc<dyn FileSource>) -> bool {
        Arc::ptr_eq(&self.source, source)
    }

    /// The tree's lister. Local: the folder, now. A host's: nothing yet, and
    /// the folder is asked for.
    pub fn list(&mut self, dir: &str) -> Vec<Entry> {
        if self.source.is_local() {
            return crate::editor_body::list_dir(dir);
        }
        self.ask(dir);
        vec![]
    }

    fn ask(&mut self, dir: &str) {
        if !self.asking.insert(dir.to_string()) {
            self.again.insert(dir.to_string());
            return;
        }
        let (source, tx, dir) = (self.source.clone(), self.answer_tx.clone(), dir.to_string());
        std::thread::spawn(move || {
            let result = source.list(&dir).map(|l| entries(&dir, l));
            let _ = tx.send((dir, result));
        });
    }

    /// Folders a tree brought along from another tab's body, already listed.
    pub fn adopt(&mut self, dirs: &[String]) {
        self.listed.extend(dirs.iter().cloned());
    }

    /// What the root shows in place of rows: loading, or why it could not be
    /// listed. `None` once it has been listed.
    pub fn status(&self, root: &str) -> Option<Status> {
        if self.source.is_local() {
            return None;
        }
        if let Some(why) = self.failed.get(root) {
            return Some(Status::Failed(why.clone()));
        }
        (!self.listed.contains(root)).then_some(Status::Loading)
    }

    /// Takes the answers that came, keeps watches on `shown`, re-lists what a
    /// watch reported (or what has no watch, every `POLL_MS`).
    pub fn tick(&mut self, shown: &[String], root: &str, now: f64) -> Updates {
        let mut out = Updates::default();
        if self.source.is_local() {
            return out;
        }
        while let Ok((dir, result)) = self.answers.try_recv() {
            self.asking.remove(&dir);
            let again = self.again.remove(&dir);
            match result {
                Ok(list) => {
                    self.failed.remove(&dir);
                    self.listed.insert(dir.clone());
                    out.listings.push((dir.clone(), list));
                }
                // A folder that is gone lists as empty; its parent's own
                // listing drops it from the tree.
                Err(e) if e.code == Code::NotFound && dir != root => {
                    self.failed.remove(&dir);
                    self.listed.insert(dir.clone());
                    out.listings.push((dir.clone(), vec![]));
                }
                Err(e) => {
                    if self.failed.get(&dir) != Some(&e.message) {
                        out.notices
                            .push(format!("could not list {dir} on the host: {e}"));
                    }
                    self.failed.insert(dir.clone(), e.message);
                }
            }
            if again {
                self.ask(&dir);
            }
        }
        while let Ok((dir, result)) = self.started.try_recv() {
            match result {
                Ok(watch) => {
                    self.watches.insert(dir.clone(), WatchSlot::On(watch));
                    // What changed before the watch was set is not an event.
                    self.ask(&dir);
                }
                Err(e) if e.code == Code::Unsupported => {
                    self.watches.insert(dir, WatchSlot::Unsupported);
                }
                Err(_) => {
                    self.watches.insert(dir, WatchSlot::Polled);
                }
            }
        }
        self.watches.retain(|d, _| shown.contains(d));
        for dir in shown {
            if !self.watches.contains_key(dir) {
                self.watches.insert(dir.clone(), WatchSlot::Starting);
                let (source, tx, d) = (self.source.clone(), self.started_tx.clone(), dir.clone());
                std::thread::spawn(move || {
                    let _ = tx.send((d.clone(), source.watch(&d)));
                });
            }
        }
        let (mut stale, mut closed) = (vec![], vec![]);
        for (dir, slot) in &mut self.watches {
            if let WatchSlot::On(w) = slot {
                let seen = w.poll();
                if seen.closed {
                    // The connection ended: list what may have been missed;
                    // the next tick sets a new watch.
                    closed.push(dir.clone());
                    stale.push(dir.clone());
                } else if seen.changed {
                    stale.push(dir.clone());
                }
            }
        }
        for dir in closed {
            self.watches.remove(&dir);
        }
        for dir in stale {
            self.ask(&dir);
        }
        if now - self.last_poll >= POLL_MS {
            let polled: Vec<(String, bool)> = shown
                .iter()
                .filter_map(|d| match self.watches.get(d) {
                    Some(WatchSlot::Polled) => Some((d.clone(), true)),
                    Some(WatchSlot::Unsupported) => Some((d.clone(), false)),
                    _ => None,
                })
                .collect();
            // The interval is used up only by a poll that happened, so a
            // folder whose watch is still being set up is not made to wait.
            if !polled.is_empty() {
                self.last_poll = now;
            }
            for (dir, retry_watch) in polled {
                self.ask(&dir);
                if retry_watch {
                    self.watches.remove(&dir);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use infiniterm_core::file_source::{EntryKind, FileData, HostPaths, Stat};
    use std::sync::mpsc::Sender;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// A host in memory: folders with entries, a switch that makes every list
    /// fail, and watches the test fires by hand.
    #[derive(Default)]
    struct Fake {
        dirs: Mutex<HashMap<String, Vec<ListEntry>>>,
        fail: Mutex<Option<Code>>,
        no_watch: Mutex<bool>,
        fire: Mutex<HashMap<String, Sender<()>>>,
        lists: Mutex<Vec<String>>,
    }

    fn entry(name: &str, is_dir: bool) -> ListEntry {
        ListEntry {
            name: name.into(),
            is_dir,
        }
    }

    impl FileSource for Fake {
        fn is_local(&self) -> bool {
            false
        }
        fn home(&self) -> Result<HostPaths, FileError> {
            unreachable!()
        }
        fn stat(&self, _: &str) -> Result<Stat, FileError> {
            unreachable!()
        }
        fn exists(&self, _: &[String]) -> Result<Vec<Option<EntryKind>>, FileError> {
            unreachable!()
        }
        fn list(&self, path: &str) -> Result<Vec<ListEntry>, FileError> {
            self.lists.lock().unwrap().push(path.to_string());
            if let Some(code) = *self.fail.lock().unwrap() {
                return Err(FileError::new(code, format!("{path}: no")));
            }
            self.dirs
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| FileError::new(Code::NotFound, format!("{path}: not found")))
        }
        fn read(&self, _: &str) -> Result<FileData, FileError> {
            unreachable!()
        }
        fn write(&self, _: &str, _: &[u8], _: Option<u64>) -> Result<Option<u64>, FileError> {
            unreachable!()
        }
        fn watch(&self, path: &str) -> Result<Watch, FileError> {
            if *self.no_watch.lock().unwrap() {
                return Err(FileError::new(Code::Unsupported, "no watching"));
            }
            let (tx, rx) = channel();
            self.fire.lock().unwrap().insert(path.to_string(), tx);
            Ok(Watch::new(rx, Box::new(|| {})))
        }
    }

    fn host() -> Arc<Fake> {
        let fake = Arc::new(Fake::default());
        let mut d = fake.dirs.lock().unwrap();
        d.insert("/r".into(), vec![entry("src", true), entry("a.md", false)]);
        d.insert("/r/src".into(), vec![entry("main.rs", false)]);
        drop(d);
        fake
    }

    /// Ticks until `done` says so (the asks run on threads).
    fn until(
        dirs: &mut HostDirs,
        shown: &[&str],
        mut now: f64,
        mut done: impl FnMut(&Updates, &mut HostDirs) -> bool,
    ) -> Updates {
        let shown: Vec<String> = shown.iter().map(|s| s.to_string()).collect();
        let stop = Instant::now() + Duration::from_secs(5);
        loop {
            let up = dirs.tick(&shown, "/r", now);
            if done(&up, dirs) {
                return up;
            }
            assert!(Instant::now() < stop, "timed out");
            now += 1.;
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn the_root_says_loading_until_the_host_lists_it_then_the_entries_have_host_paths() {
        let fake = host();
        let mut dirs = HostDirs::new(fake.clone());
        assert!(dirs.list("/r").is_empty(), "nothing yet");
        assert_eq!(dirs.status("/r"), Some(Status::Loading));
        assert_eq!(Status::Loading.text(), "Loading from the host...");

        let up = until(&mut dirs, &["/r"], 0., |u, _| !u.listings.is_empty());
        let (dir, list) = &up.listings[0];
        assert_eq!(dir, "/r");
        assert_eq!(
            list.iter()
                .map(|e| (e.name.as_str(), e.path.as_str(), e.is_dir))
                .collect::<Vec<_>>(),
            vec![("src", "/r/src", true), ("a.md", "/r/a.md", false)]
        );
        assert_eq!(dirs.status("/r"), None);
    }

    #[test]
    fn a_host_error_is_text_for_the_root_and_a_notice_once() {
        let fake = host();
        *fake.fail.lock().unwrap() = Some(Code::Denied);
        // No watch, so the poll is what asks again.
        *fake.no_watch.lock().unwrap() = true;
        let mut dirs = HostDirs::new(fake.clone());
        dirs.list("/r");
        let up = until(&mut dirs, &["/r"], 0., |u, _| !u.notices.is_empty());
        assert!(up.notices[0].contains("/r: no"), "{:?}", up.notices);
        let Some(Status::Failed(why)) = dirs.status("/r") else {
            panic!("the root failed");
        };
        assert!(Status::Failed(why)
            .text()
            .starts_with("Could not list this folder on the host"));

        // Asked again (the poll) and failing the same way: not said again.
        let mut said = 0;
        until(&mut dirs, &["/r"], 10_000., |u, d| {
            said += u.notices.len();
            d.status("/r").is_some() && fake.lists.lock().unwrap().len() >= 2
        });
        assert_eq!(said, 0);

        // The host recovers: the next poll lists it.
        *fake.fail.lock().unwrap() = None;
        until(&mut dirs, &["/r"], 20_000., |u, _| !u.listings.is_empty());
        assert_eq!(dirs.status("/r"), None);
    }

    #[test]
    fn a_folder_that_vanishes_lists_as_empty_but_a_vanished_root_is_an_error() {
        let fake = host();
        let mut dirs = HostDirs::new(fake.clone());
        dirs.list("/r");
        until(&mut dirs, &["/r"], 0., |u, _| !u.listings.is_empty());
        dirs.list("/r/gone");
        let up = until(&mut dirs, &["/r", "/r/gone"], 100., |u, _| {
            u.listings.iter().any(|(d, _)| d == "/r/gone")
        });
        assert!(up
            .listings
            .iter()
            .any(|(d, l)| d == "/r/gone" && l.is_empty()));

        fake.dirs.lock().unwrap().remove("/r");
        dirs.list("/r");
        let up = until(&mut dirs, &["/r"], 200., |u, _| !u.notices.is_empty());
        assert!(up.notices[0].contains("not found"), "{:?}", up.notices);
        assert!(matches!(dirs.status("/r"), Some(Status::Failed(_))));
    }

    #[test]
    fn a_watch_event_lists_the_folder_again_without_a_poll() {
        let fake = host();
        let mut dirs = HostDirs::new(fake.clone());
        dirs.list("/r");
        until(&mut dirs, &["/r"], 0., |u, _| !u.listings.is_empty());
        // The watch is set (and lists once, for what it missed).
        until(&mut dirs, &["/r"], 1., |_, _| {
            fake.fire.lock().unwrap().contains_key("/r")
        });
        until(&mut dirs, &["/r"], 2., |u, _| !u.listings.is_empty());

        fake.dirs
            .lock()
            .unwrap()
            .insert("/r".into(), vec![entry("new.md", false)]);
        fake.fire.lock().unwrap()["/r"].send(()).unwrap();
        // `now` far below POLL_MS: only the event can have caused this.
        let up = until(&mut dirs, &["/r"], 3., |u, _| !u.listings.is_empty());
        assert_eq!(up.listings[0].1[0].name, "new.md");
    }

    #[test]
    fn a_lost_watch_is_polled_and_a_host_without_watching_is_polled_too() {
        let fake = host();
        *fake.no_watch.lock().unwrap() = true;
        let mut dirs = HostDirs::new(fake.clone());
        dirs.list("/r");
        until(&mut dirs, &["/r"], 0., |u, _| !u.listings.is_empty());
        // The host's refusal to watch has been taken, so the folder is polled.
        until(&mut dirs, &["/r"], 1., |_, d| {
            matches!(d.watches.get("/r"), Some(WatchSlot::Unsupported))
        });
        fake.dirs
            .lock()
            .unwrap()
            .insert("/r".into(), vec![entry("later.md", false)]);
        // Past the poll interval: the change is found without any event.
        let up = until(&mut dirs, &["/r"], 3_000., |u, _| !u.listings.is_empty());
        assert_eq!(up.listings[0].1[0].name, "later.md");

        // Dropping the sender of a watch closes it, as a lost connection does.
        let fake = host();
        let mut dirs = HostDirs::new(fake.clone());
        dirs.list("/r");
        until(&mut dirs, &["/r"], 0., |u, _| !u.listings.is_empty());
        until(&mut dirs, &["/r"], 1., |_, _| {
            fake.fire.lock().unwrap().contains_key("/r")
        });
        fake.fire.lock().unwrap().clear();
        fake.dirs
            .lock()
            .unwrap()
            .insert("/r".into(), vec![entry("after.md", false)]);
        let up = until(&mut dirs, &["/r"], 2., |u, _| {
            u.listings
                .iter()
                .any(|(_, l)| l.iter().any(|e| e.name == "after.md"))
        });
        assert!(!up.listings.is_empty());
    }

    #[test]
    fn a_local_source_lists_on_the_spot_and_has_no_status() {
        let mut dirs = HostDirs::new(Arc::new(infiniterm_core::file_source::LocalSource));
        let dir = std::env::temp_dir().join(format!("hd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.txt"), "x").unwrap();
        let names: Vec<_> = dirs
            .list(&dir.to_string_lossy())
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["x.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(dirs.status("/anything"), None);
        let up = dirs.tick(&["/".into()], "/", 5_000.);
        assert!(up.listings.is_empty() && up.notices.is_empty());
    }

    #[test]
    fn a_tree_brought_from_another_tab_is_not_loading_again() {
        let mut dirs = HostDirs::new(host());
        assert_eq!(dirs.status("/r"), Some(Status::Loading));
        dirs.adopt(&["/r".to_string(), "/r/src".to_string()]);
        assert_eq!(dirs.status("/r"), None);
    }

    #[test]
    fn host_paths_join_as_text() {
        assert_eq!(join("/a/b", "c"), "/a/b/c");
        assert_eq!(join("/", "c"), "/c");
        assert_eq!(join("C:\\x/", "c"), "C:\\x/c");
    }
}
