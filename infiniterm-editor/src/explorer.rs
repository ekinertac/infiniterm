//! The file tree beside an editor, rooted at a directory. Port of
//! `Explorer.svelte`'s state and keys, with the filesystem behind a
//! closure so the tree is tested without one.
//!
//! Lazy: a directory is listed when it is first expanded, never before, so
//! opening `~/Code` costs one listing and not a walk. Arrows move, Right
//! expands and Left collapses (or climbs to the parent), Enter opens a
//! file or toggles a directory, Escape hands focus back to the editor. A
//! click does what Enter does. The rows are the tree flattened in display
//! order, so the cursor is one index.
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub entry: Entry,
    pub depth: usize,
}

/// What a key asked the editor to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeAction {
    None,
    Open(String),
    /// The cursor moved onto a file without opening it. The editor shows a
    /// picture at once on this, so a folder of screenshots is walked with
    /// the arrows alone; text waits for Enter, since a preview of it would
    /// mean a buffer swapped under the reader every keystroke.
    Landed(String),
    /// Escape: back to the buffer.
    Close,
}

#[derive(Clone, Debug, Default)]
pub struct Tree {
    pub root: String,
    listed: HashMap<String, Vec<Entry>>,
    expanded: HashMap<String, bool>,
    pub cursor: usize,
    /// First visible row, for scrolling the panel.
    pub scroll: usize,
}

impl Tree {
    pub fn new(root: &str) -> Tree {
        Tree {
            root: root.to_string(),
            ..Default::default()
        }
    }

    /// Lists a directory the first time it is needed; `list` is the
    /// filesystem (`files::dir_list`), asked once per directory.
    pub fn load(&mut self, dir: &str, list: &mut dyn FnMut(&str) -> Vec<Entry>) {
        if !self.listed.contains_key(dir) {
            let entries = list(dir);
            self.listed.insert(dir.to_string(), entries);
        }
    }

    pub fn ensure_root(&mut self, list: &mut dyn FnMut(&str) -> Vec<Entry>) {
        let root = self.root.clone();
        self.load(&root, list);
    }

    /// Re-lists every directory the tree is SHOWING (the root and the
    /// expanded ones; a collapsed directory is re-listed when it opens
    /// again, by `load` after it is forgotten here), for the editor's
    /// disk poll: a file made or removed outside the app appears without
    /// reopening the tree. Expansion and the cursor are kept, the cursor
    /// clamped to the rows that remain. True when anything changed.
    pub fn refresh(&mut self, list: &mut dyn FnMut(&str) -> Vec<Entry>) -> bool {
        let shown: Vec<String> = self
            .listed
            .keys()
            .filter(|d| **d == self.root || self.is_expanded(d))
            .cloned()
            .collect();
        let mut changed = false;
        for dir in shown {
            let fresh = list(&dir);
            if self.listed.get(&dir) != Some(&fresh) {
                self.listed.insert(dir, fresh);
                changed = true;
            }
        }
        if changed {
            // Directories no longer shown are re-listed when next opened.
            let expanded: Vec<String> = self.expanded.keys().cloned().collect();
            for dir in expanded {
                if !self.listed.values().flatten().any(|e| e.is_dir && e.path == dir) && dir != self.root {
                    self.listed.remove(&dir);
                    self.expanded.remove(&dir);
                }
            }
            let last = self.rows().len().saturating_sub(1);
            self.cursor = self.cursor.min(last);
        }
        changed
    }

    pub fn rows(&self) -> Vec<Row> {
        let mut out = vec![];
        self.walk(&self.root, 0, &mut out);
        out
    }

    fn walk(&self, dir: &str, depth: usize, out: &mut Vec<Row>) {
        for entry in self.listed.get(dir).into_iter().flatten() {
            out.push(Row {
                entry: entry.clone(),
                depth,
            });
            if entry.is_dir && self.expanded.get(&entry.path).copied().unwrap_or(false) {
                self.walk(&entry.path, depth + 1, out);
            }
        }
    }

    pub fn is_expanded(&self, path: &str) -> bool {
        self.expanded.get(path).copied().unwrap_or(false)
    }

    /// Enter or a click on a row: a file opens, a directory toggles.
    pub fn toggle(&mut self, index: usize, list: &mut dyn FnMut(&str) -> Vec<Entry>) -> TreeAction {
        let rows = self.rows();
        let Some(row) = rows.get(index) else {
            return TreeAction::None;
        };
        self.cursor = index;
        if !row.entry.is_dir {
            return TreeAction::Open(row.entry.path.clone());
        }
        let open = !self.is_expanded(&row.entry.path);
        self.expanded.insert(row.entry.path.clone(), open);
        if open {
            self.load(&row.entry.path.clone(), list);
        }
        TreeAction::None
    }

    /// A key by name (`down`, `up`, `left`, `right`, `enter`, `escape`).
    pub fn key(&mut self, key: &str, list: &mut dyn FnMut(&str) -> Vec<Entry>) -> TreeAction {
        let rows = self.rows();
        let last = rows.len().saturating_sub(1);
        let row = rows.get(self.cursor).cloned();
        let was = self.cursor;
        match key {
            "down" => self.cursor = (self.cursor + 1).min(last),
            "up" => self.cursor = self.cursor.saturating_sub(1),
            "right" => match &row {
                Some(r) if r.entry.is_dir && !self.is_expanded(&r.entry.path) => {
                    return self.toggle(self.cursor, list);
                }
                _ => self.cursor = (self.cursor + 1).min(last),
            },
            "left" => match &row {
                Some(r) if r.entry.is_dir && self.is_expanded(&r.entry.path) => {
                    self.expanded.insert(r.entry.path.clone(), false);
                }
                Some(r) => {
                    // To the parent row: the nearest row above with a smaller depth.
                    if let Some(i) = (0..self.cursor).rev().find(|&i| rows[i].depth < r.depth) {
                        self.cursor = i;
                    }
                }
                None => {}
            },
            "enter" => return self.toggle(self.cursor, list),
            "escape" => return TreeAction::Close,
            _ => {}
        }
        match rows.get(self.cursor) {
            Some(r) if self.cursor != was && !r.entry.is_dir => {
                TreeAction::Landed(r.entry.path.clone())
            }
            _ => TreeAction::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs() -> impl FnMut(&str) -> Vec<Entry> {
        |dir: &str| {
            let e = |name: &str, is_dir: bool| Entry {
                name: name.into(),
                path: format!("{dir}/{name}"),
                is_dir,
            };
            match dir {
                "/r" => vec![e("src", true), e("README.md", false)],
                "/r/src" => vec![e("main.rs", false)],
                _ => vec![],
            }
        }
    }

    #[test]
    fn directories_list_lazily_and_expand_on_right() {
        let mut list = fs();
        let mut t = Tree::new("/r");
        t.ensure_root(&mut list);
        assert_eq!(t.rows().len(), 2);
        assert_eq!(t.key("right", &mut list), TreeAction::None);
        let rows = t.rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].entry.name, "main.rs");
        assert_eq!(rows[1].depth, 1);
        assert_eq!(
            t.key("down", &mut list),
            TreeAction::Landed("/r/src/main.rs".into())
        );
        assert_eq!(
            t.key("enter", &mut list),
            TreeAction::Open("/r/src/main.rs".into())
        );
        // Left from a child climbs to the parent; left on it collapses.
        t.key("left", &mut list);
        assert_eq!(t.cursor, 0);
        t.key("left", &mut list);
        assert_eq!(t.rows().len(), 2);
        assert_eq!(t.key("escape", &mut list), TreeAction::Close);
    }

    // A file appearing or vanishing on disk shows on the next refresh; a
    // directory that vanished takes its expansion with it, and the cursor
    // stays inside the rows that remain. Nothing changed, nothing said.
    #[test]
    fn refresh_follows_the_disk_and_keeps_expansion() {
        let mut t = Tree::new("/r");
        let mut list = fs();
        t.ensure_root(&mut list);
        t.key("right", &mut list); // expand src
        assert_eq!(t.rows().len(), 3);
        assert!(!t.refresh(&mut list), "same disk, no change");
        // A new file in src, and README gone.
        let mut list2 = |dir: &str| -> Vec<Entry> {
            let e = |name: &str, is_dir: bool| Entry {
                name: name.into(),
                path: format!("{dir}/{name}"),
                is_dir,
            };
            match dir {
                "/r" => vec![e("src", true)],
                "/r/src" => vec![e("lib.rs", false), e("main.rs", false)],
                _ => vec![],
            }
        };
        t.key("down", &mut list);
        t.key("down", &mut list);
        assert_eq!(t.cursor, 2);
        assert!(t.refresh(&mut list2));
        let rows = t.rows();
        assert_eq!(rows.len(), 3, "src, lib.rs, main.rs");
        assert!(rows.iter().any(|r| r.entry.name == "lib.rs"));
        assert!(t.is_expanded("/r/src"), "expansion kept");
        // src itself vanishes: its expansion goes and the cursor clamps.
        let mut list3 = |_dir: &str| -> Vec<Entry> { vec![] };
        assert!(t.refresh(&mut list3));
        assert_eq!(t.rows().len(), 0);
        assert_eq!(t.cursor, 0);
        assert!(!t.is_expanded("/r/src"));
    }

    #[test]
    fn the_cursor_stays_inside_the_rows() {
        let mut list = fs();
        let mut t = Tree::new("/r");
        t.ensure_root(&mut list);
        assert_eq!(t.key("up", &mut list), TreeAction::None);
        assert_eq!(t.cursor, 0);
        t.key("down", &mut list);
        // At the end already: the cursor did not move, so nothing landed.
        assert_eq!(t.key("down", &mut list), TreeAction::None);
        t.key("down", &mut list);
        assert_eq!(t.cursor, 1);
        assert_eq!(
            t.key("enter", &mut list),
            TreeAction::Open("/r/README.md".into())
        );
    }
}
