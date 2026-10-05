//! Finding and loading the schema a file names. `"$schema"` near the top of
//! a JSON file points at a path (read relative to the file) or an `https`
//! address (fetched once with `curl` on a thread and kept in
//! `<data>/schemas/`, refreshed in the background after a week). Nothing
//! else is fetched: `http://` and other schemes are refused, so a file you
//! open cannot make the app reach anywhere in the clear.
//!
//! A remote schema is not there on the first keystroke: `get` answers
//! `None` while the download runs, and the next keystroke asks again.
//! shortcut: the popup does not open by itself when the download ends; it
//! waits for the next key. Fine for a file you are typing in.
//!
//! The editor's own `settings.json` does not come through here: its schema
//! is built from the settings table (`complete.rs`).
use crate::schema::Schema;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

/// How far into the text `"$schema"` is looked for.
const REF_SCAN_CHARS: usize = 4096;
/// A cached remote schema older than this is used and refreshed.
const CACHE_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
/// After a failed download, how long before the next try.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// A schema bigger than this is not read: a megabyte of schema parsed on a
/// keystroke would be felt.
const MAX_SCHEMA_BYTES: u64 = 4 * 1024 * 1024;

/// The value of the `"$schema"` key near the top of a JSON text.
pub fn schema_ref(text: &str) -> Option<String> {
    let head: String = text.chars().take(REF_SCAN_CHARS).collect();
    let at = head.find("\"$schema\"")? + "\"$schema\"".len();
    let rest = head[at..].trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    let value = &rest[..end];
    (!value.is_empty()).then(|| value.to_string())
}

#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Local(PathBuf),
    Remote(String),
}

/// What a `$schema` value points at, relative to the file's folder; `None`
/// for anything the app will not read (`http://`, other schemes).
pub fn resolve(reference: &str, file_dir: &Path) -> Option<Source> {
    if reference.starts_with("https://") {
        return Some(Source::Remote(reference.to_string()));
    }
    if reference.contains("://") {
        return None;
    }
    let path = match reference.strip_prefix("~/") {
        Some(rest) => crate::paths::home_dir().join(rest),
        None => file_dir.join(reference),
    };
    Some(Source::Local(path))
}

/// Downloads a URL's body, or `None`.
pub type Fetcher = fn(&str) -> Option<String>;

/// `curl`, as the omnibox suggestions do: no HTTP crate, and `--max-time`
/// is why a stalled connection cannot hold the thread for long.
pub fn curl_fetch(url: &str) -> Option<String> {
    let out = std::process::Command::new("curl")
        .args(["-sL", "--fail", "--max-time", "15", url])
        .output()
        .ok()?;
    (out.status.success() && out.stdout.len() as u64 <= MAX_SCHEMA_BYTES)
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

enum Entry {
    Loaded {
        schema: Arc<Schema>,
        stamp: Option<SystemTime>,
        /// A remote one: a refresh has been started this run.
        refreshed: bool,
    },
    Fetching,
    Failed(SystemTime),
}

pub struct Store {
    cache_dir: PathBuf,
    fetch: Fetcher,
    entries: Arc<Mutex<HashMap<String, Entry>>>,
}

/// FNV-1a, for a cache file name from a URL: stable across runs, which
/// `DefaultHasher` does not promise.
fn fnv(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn read_schema(path: &Path) -> Option<Arc<Schema>> {
    if std::fs::metadata(path).ok()?.len() > MAX_SCHEMA_BYTES {
        return None;
    }
    Schema::parse(&std::fs::read_to_string(path).ok()?).map(Arc::new)
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

impl Store {
    pub fn new(cache_dir: PathBuf, fetch: Fetcher) -> Store {
        Store {
            cache_dir,
            fetch,
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The schema for a source, if it is ready now.
    pub fn get(&self, source: &Source) -> Option<Arc<Schema>> {
        match source {
            Source::Local(path) => self.get_local(path),
            Source::Remote(url) => self.get_remote(url),
        }
    }

    /// A file on disk, read again when its modification time changes.
    fn get_local(&self, path: &Path) -> Option<Arc<Schema>> {
        let key = path.to_string_lossy().into_owned();
        let stamp = mtime(path);
        let mut entries = self.entries.lock().ok()?;
        if let Some(Entry::Loaded {
            schema, stamp: s, ..
        }) = entries.get(&key)
        {
            if *s == stamp {
                return Some(schema.clone());
            }
        }
        let schema = read_schema(path)?;
        entries.insert(
            key,
            Entry::Loaded {
                schema: schema.clone(),
                stamp,
                refreshed: true,
            },
        );
        Some(schema)
    }

    fn get_remote(&self, url: &str) -> Option<Arc<Schema>> {
        let mut entries = self.entries.lock().ok()?;
        match entries.get_mut(url) {
            Some(Entry::Loaded {
                schema, refreshed, ..
            }) => {
                let schema = schema.clone();
                if !*refreshed {
                    *refreshed = true;
                    self.spawn_fetch(url);
                }
                return Some(schema);
            }
            Some(Entry::Fetching) => return None,
            Some(Entry::Failed(at)) if at.elapsed().unwrap_or_default() < RETRY_AFTER => {
                return None;
            }
            Some(Entry::Failed(_)) | None => {}
        }
        let cache = self.cache_dir.join(format!("{}.json", fnv(url)));
        if let Some(schema) = read_schema(&cache) {
            let stale = mtime(&cache)
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > CACHE_TTL);
            entries.insert(
                url.to_string(),
                Entry::Loaded {
                    schema: schema.clone(),
                    stamp: None,
                    refreshed: !stale,
                },
            );
            if stale {
                if let Some(Entry::Loaded { refreshed, .. }) = entries.get_mut(url) {
                    *refreshed = true;
                }
                self.spawn_fetch(url);
            }
            return Some(schema);
        }
        entries.insert(url.to_string(), Entry::Fetching);
        self.spawn_fetch(url);
        None
    }

    /// Downloads on a thread, writes the cache file, and swaps the entry in.
    /// A failed refresh leaves the schema that is already loaded.
    fn spawn_fetch(&self, url: &str) {
        let (url, fetch) = (url.to_string(), self.fetch);
        let entries = self.entries.clone();
        let cache = self.cache_dir.join(format!("{}.json", fnv(&url)));
        std::thread::spawn(move || {
            let body = fetch(&url).filter(|b| Schema::parse(b).is_some());
            let Ok(mut entries) = entries.lock() else {
                return;
            };
            match body {
                Some(body) => {
                    if let Some(dir) = cache.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let _ = std::fs::write(&cache, &body);
                    if let Some(schema) = Schema::parse(&body) {
                        entries.insert(
                            url,
                            Entry::Loaded {
                                schema: Arc::new(schema),
                                stamp: None,
                                refreshed: true,
                            },
                        );
                    }
                }
                None => {
                    if !matches!(entries.get(&url), Some(Entry::Loaded { .. })) {
                        entries.insert(url, Entry::Failed(SystemTime::now()));
                    }
                }
            }
        });
    }
}

/// The app's own store: downloads kept in the data folder.
pub fn global() -> &'static Store {
    static STORE: OnceLock<Store> = OnceLock::new();
    STORE.get_or_init(|| Store::new(crate::paths::app_support_dir().join("schemas"), curl_fetch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("infiniterm-schema-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_schema_key_is_found_near_the_top() {
        assert_eq!(
            schema_ref("{\n  // c\n  \"$schema\" : \"./s.json\",\n \"a\": 1}"),
            Some("./s.json".into())
        );
        assert_eq!(schema_ref("{\"a\": 1}"), None);
        assert_eq!(schema_ref("{\"$schema\": \"\"}"), None);
        assert_eq!(schema_ref("{\"$schema\": 3}"), None);
    }

    #[test]
    fn a_reference_is_a_path_beside_the_file_or_an_https_address() {
        let dir = Path::new("/p/q");
        assert_eq!(
            resolve("./s.json", dir),
            Some(Source::Local("/p/q/./s.json".into()))
        );
        assert_eq!(
            resolve("/abs/s.json", dir),
            Some(Source::Local("/abs/s.json".into()))
        );
        assert_eq!(
            resolve("https://x.org/s.json", dir),
            Some(Source::Remote("https://x.org/s.json".into()))
        );
        assert_eq!(
            resolve("http://x.org/s.json", dir),
            None,
            "not in the clear"
        );
        assert_eq!(resolve("ftp://x.org/s.json", dir), None);
    }

    #[test]
    fn a_local_schema_is_read_again_when_the_file_changes() {
        let dir = temp("local");
        let file = dir.join("s.json");
        std::fs::write(&file, "{\"properties\": {\"a\": {}}}").unwrap();
        let store = Store::new(dir.join("cache"), |_| None);
        let s1 = store.get(&Source::Local(file.clone())).unwrap();
        assert_eq!(s1.properties(&s1.nodes_at(&[])).len(), 1);
        // The same file again: the same schema, not a second parse.
        assert!(Arc::ptr_eq(
            &s1,
            &store.get(&Source::Local(file.clone())).unwrap()
        ));
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&file, "{\"properties\": {\"a\": {}, \"b\": {}}}").unwrap();
        let s2 = store.get(&Source::Local(file)).unwrap();
        assert_eq!(s2.properties(&s2.nodes_at(&[])).len(), 2);
        assert!(store
            .get(&Source::Local(dir.join("missing.json")))
            .is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    static FETCHES: AtomicUsize = AtomicUsize::new(0);

    fn fake_fetch(url: &str) -> Option<String> {
        FETCHES.fetch_add(1, Ordering::SeqCst);
        url.contains("good")
            .then(|| "{\"properties\": {\"k\": {}}}".to_string())
    }

    fn wait_for(store: &Store, src: &Source) -> Option<Arc<Schema>> {
        for _ in 0..200 {
            if let Some(s) = store.get(src) {
                return Some(s);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn a_remote_schema_is_fetched_once_cached_and_a_failure_is_not_hammered() {
        let dir = temp("remote");
        let store = Store::new(dir.clone(), fake_fetch);
        let good = Source::Remote("https://example.org/good.json".into());
        // Not there on the first ask; there soon after.
        assert!(store.get(&good).is_none());
        let s = wait_for(&store, &good).expect("downloaded");
        assert_eq!(s.properties(&s.nodes_at(&[])).len(), 1);
        let after_first = FETCHES.load(Ordering::SeqCst);
        // A new store (a new run) reads the file instead of downloading.
        let again = Store::new(dir.clone(), |_| panic!("must not download"));
        assert!(again.get(&good).is_some());
        assert_eq!(FETCHES.load(Ordering::SeqCst), after_first);
        // A bad address: no schema, and asking again at once does not retry.
        let bad = Source::Remote("https://example.org/bad.json".into());
        assert!(store.get(&bad).is_none());
        std::thread::sleep(Duration::from_millis(100));
        let n = FETCHES.load(Ordering::SeqCst);
        assert!(store.get(&bad).is_none());
        assert!(store.get(&bad).is_none());
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(FETCHES.load(Ordering::SeqCst), n);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
