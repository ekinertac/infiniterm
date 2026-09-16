//! Where you have been, so the omnibox can rank it. One JSON array beside
//! workspace.json, loaded once and written back debounced through the same
//! write-then-rename the save file uses.
//!
//! NOT Chromium's history. The CEF profile keeps a real SQLite `History` in
//! <data>/browser/, but reading it means a SQLite dependency and a lock
//! fight with the browser process that owns it, and we already see every
//! navigation: browsers.rs receives an address change per frame.
//!
//! Sorted on WRITE, never on read: `list` hands out a borrow that the model
//! holds while it builds an OmniCtx, and a sort at read time would need
//! &mut there for no reason anybody could see from the call site.
//!
//! Called by `omni::providers` through OmniCtx, and by the ui, which records
//! navigations and titles. Related: omni/rank.rs, paths.rs.

use serde_json::{json, Value};
use std::path::Path;

/// Enough that a month of browsing is all there, small enough that the file
/// stays a file you can read.
pub const MAX_ENTRIES: usize = 1000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Visit {
    pub url: String,
    pub title: String,
    pub visits: u32,
    /// Milliseconds, the clock the rest of the model uses.
    pub at: f64,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    /// Always in ranked order: every mutation re-sorts.
    entries: Vec<Visit>,
    dirty: bool,
}

/// An address that is the app's own furniture, never somewhere you went.
fn internal(url: &str) -> bool {
    url.is_empty()
        || url.starts_with("about:")
        || url.starts_with("data:")
        || url.starts_with("chrome://")
        || url.starts_with("devtools://")
}

impl History {
    pub fn load(path: &Path) -> History {
        let Ok(text) = std::fs::read_to_string(path) else {
            return History::default();
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            return History::default();
        };
        let Some(array) = value.as_array() else {
            return History::default();
        };
        let mut history = History {
            entries: array
                .iter()
                .filter_map(|v| {
                    let url = v.get("url")?.as_str()?.to_string();
                    if internal(&url) {
                        return None;
                    }
                    Some(Visit {
                        url,
                        title: v
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        // A file written before visits existed counts as one.
                        visits: v.get("visits").and_then(Value::as_u64).unwrap_or(1) as u32,
                        at: v.get("at").and_then(Value::as_f64).unwrap_or(0.),
                    })
                })
                .collect(),
            // A load is not a change: setting this would rewrite the file on
            // every launch for nothing.
            dirty: false,
        };
        history.settle();
        history
    }

    pub fn record(&mut self, url: &str, now: f64) {
        if internal(url) {
            return;
        }
        self.dirty = true;
        if let Some(entry) = self.entries.iter_mut().find(|e| e.url == url) {
            entry.visits += 1;
            entry.at = now;
        } else {
            self.entries.push(Visit {
                url: url.to_string(),
                title: String::new(),
                visits: 1,
                at: now,
            });
        }
        self.settle();
    }

    pub fn set_title(&mut self, url: &str, title: &str) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.url == url) {
            if entry.title != title {
                entry.title = title.to_string();
                self.dirty = true;
            }
        }
    }

    /// Ranked order, then the cap. Frequency first, recency as the tiebreak:
    /// a page opened daily beats a redirect seen once a minute ago.
    fn settle(&mut self) {
        self.entries
            .sort_by(|a, b| b.visits.cmp(&a.visits).then(b.at.total_cmp(&a.at)));
        self.entries.truncate(MAX_ENTRIES);
    }

    pub fn list(&self) -> &[Visit] {
        &self.entries
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    pub fn save(&self, path: &Path) {
        let array: Vec<Value> = self
            .entries
            .iter()
            .map(|e| json!({"url": e.url, "title": e.title, "visits": e.visits, "at": e.at}))
            .collect();
        let Ok(text) = serde_json::to_string_pretty(&Value::Array(array)) else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Write then rename, so a force-quit mid-write leaves the old file
        // rather than half of this one.
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_revisit_bumps_the_count_and_the_time() {
        let mut h = History::default();
        h.record("https://a.example", 100.);
        h.record("https://b.example", 200.);
        h.record("https://a.example", 300.);
        let a = h
            .list()
            .iter()
            .find(|v| v.url == "https://a.example")
            .unwrap();
        assert_eq!(a.visits, 2);
        assert_eq!(a.at, 300.);
        assert_eq!(h.list().len(), 2);
    }

    // Frequency first, recency as the tiebreak: a page opened daily must beat
    // the redirect URL that was seen once ten seconds ago.
    #[test]
    fn the_list_ranks_by_visits_then_recency() {
        let mut h = History::default();
        for _ in 0..5 {
            h.record("https://daily.example", 1.);
        }
        h.record("https://once.example", 999.);
        assert_eq!(h.list()[0].url, "https://daily.example");
    }

    #[test]
    fn a_title_lands_on_the_entry_it_belongs_to() {
        let mut h = History::default();
        h.record("https://a.example", 1.);
        h.set_title("https://a.example", "A");
        assert_eq!(h.list()[0].title, "A");
        // A title for something never visited is dropped, not inserted.
        h.set_title("https://gone.example", "G");
        assert_eq!(h.list().len(), 1);
    }

    #[test]
    fn internal_addresses_are_never_recorded() {
        let mut h = History::default();
        h.record("about:blank", 1.);
        h.record("", 1.);
        h.record("data:text/html,x", 1.);
        assert!(h.list().is_empty());
    }

    #[test]
    fn the_file_round_trips_and_a_broken_one_reads_as_empty() {
        let dir = std::env::temp_dir().join(format!("omni-hist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.json");
        let mut h = History::default();
        h.record("https://a.example", 7.);
        h.set_title("https://a.example", "A");
        h.save(&path);
        let back = History::load(&path);
        assert_eq!(back.list()[0].title, "A");
        assert_eq!(back.list()[0].at, 7.);
        std::fs::write(&path, "{not json").unwrap();
        assert!(History::load(&path).list().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_cap_drops_the_least_useful_entries() {
        let mut h = History::default();
        for i in 0..(MAX_ENTRIES + 10) {
            h.record(&format!("https://{i}.example"), i as f64);
        }
        assert_eq!(h.list().len(), MAX_ENTRIES);
        // What survived is what was ranked highest, not what arrived last.
        assert_eq!(h.list()[0].at, (MAX_ENTRIES + 9) as f64);
    }

    // The dirty flag is what the poll task debounces on; a load must not set
    // it or every launch would rewrite the file for nothing.
    #[test]
    fn only_a_change_makes_it_dirty() {
        let mut h = History::default();
        assert!(!h.is_dirty());
        h.record("https://a.example", 1.);
        assert!(h.is_dirty());
        h.clear_dirty();
        h.set_title("https://a.example", "A");
        assert!(h.is_dirty());
        h.clear_dirty();
        h.set_title("https://a.example", "A");
        assert!(!h.is_dirty(), "the same title again changes nothing");
    }
}
