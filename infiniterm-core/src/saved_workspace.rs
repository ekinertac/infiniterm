//! A workspace saved under a name (#715): one workspace's cards and groups
//! as a file that `workspace.openSaved` opens again later, as a new
//! workspace.
//!
//! The file is `workspace.json`'s own format (`saved_layout.rs`) holding one
//! workspace, so the tolerant parser there reads it and a field a card gains
//! later is saved here with no change to this file. What a snapshot leaves
//! out is what belongs to the shells of the canvas it was taken from: the
//! daemon session, the agent session and its title, and the card number.
//! A snapshot can outlive all of them, so an opened card is a fresh shell in
//! the saved directory, numbered like any new card.
//!
//! `Model::workspace_snapshot_text` and `Model::open_saved_workspace`
//! (`model/persist.rs`) are the two halves; the ui (`runtime.rs`) moves
//! the bytes with `write`, `read` and `list` below, on the effects
//! `SaveWorkspaceFile`, `LoadSavedWorkspace` and `RefreshSavedWorkspaces`.
//! The folder is `paths::saved_workspaces_dir`; the functions take it as an
//! argument so a test never touches the real one.
use crate::palette_usage::Usage;
use crate::saved_layout::{serialise_layout, SavedCard, SavedGroup, SavedWorkspace};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// What stands in for a character a file name cannot hold.
const REPLACEMENT: char = '-';

/// The file name (without `.json`) a workspace saved as `name` gets, or
/// `None` when nothing of the name is left. A path separator, a colon (the
/// Finder's separator) and a control character become a dash, and leading
/// dots go, so a name can neither leave the folder nor make a hidden file.
pub fn file_stem(name: &str) -> Option<String> {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => REPLACEMENT,
            c if c.is_control() => REPLACEMENT,
            c => c,
        })
        .collect();
    let stem = cleaned.trim_start_matches('.').trim();
    (!stem.is_empty()).then(|| stem.to_string())
}

/// The snapshot's JSON value: `workspace` named `name`, the `cards` on it
/// and the groups those cards are in. `cards` and `groups` may be the whole
/// canvas; what is on another workspace is left out here.
pub fn snapshot_value(
    name: &str,
    workspace: &SavedWorkspace,
    cards: &[SavedCard],
    groups: &[SavedGroup],
) -> Value {
    let cards: Vec<SavedCard> = cards
        .iter()
        .filter(|c| c.workspace_id == workspace.id)
        .cloned()
        .map(|c| SavedCard {
            session: None,
            kitty_keys: false,
            agent_session: None,
            agent_kind: None,
            agent_title: None,
            number: 0,
            cover_of: None,
            ..c
        })
        .collect();
    let groups: Vec<SavedGroup> = groups
        .iter()
        .filter(|g| cards.iter().any(|c| c.group_id.as_ref() == Some(&g.id)))
        .cloned()
        .collect();
    let workspace = SavedWorkspace {
        name: name.to_string(),
        ..workspace.clone()
    };
    serialise_layout(
        &cards,
        &groups,
        &workspace.viewport,
        workspace.focused.as_deref(),
        // The chrome scale and the palette's history are the app's, not a
        // workspace's: written as their defaults and ignored when opened.
        1.,
        &Usage::default(),
        std::slice::from_ref(&workspace),
        Some(&workspace.id),
    )
}

/// The saved workspaces' names, sorted: every `*.json` in `dir`. A folder
/// that does not exist yet has none.
pub fn list(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            (path.extension()? == "json")
                .then(|| path.file_stem()?.to_str().map(String::from))
                .flatten()
        })
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// The file the snapshot `name` lives in under `dir`, or `None` when
/// nothing of the name is left (`file_stem`). Whether it exists is the
/// caller's question: `workspace.saveAs` asks before writing over one.
pub fn path(dir: &Path, name: &str) -> Option<PathBuf> {
    Some(dir.join(format!("{}.json", file_stem(name)?)))
}

/// Writes the snapshot `name` into `dir`, over one of the same name.
pub fn write(dir: &Path, name: &str, contents: &str) -> Result<(), String> {
    let file = path(dir, name).ok_or("a saved workspace needs a name")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    crate::files::write_atomically(&file, contents).map_err(|e| e.to_string())
}

/// The snapshot `name` in `dir`, or `None` when there is no such file.
pub fn read(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(path(dir, name)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Rect;
    use crate::saved_layout::{parse_layout_allowing_empty, CardKind};
    use crate::workspaces::INITIAL_VIEWPORT;

    fn card(id: &str, workspace: &str, group: Option<&str>) -> SavedCard {
        SavedCard {
            id: id.into(),
            workspace_id: workspace.into(),
            rect: Rect {
                x: 0.,
                y: 0.,
                w: 69.,
                h: 80.,
            },
            z: 1.,
            title: String::new(),
            cwd: "/tmp".into(),
            group_id: group.map(String::from),
            soft_group_id: None,
            split_from: None,
            kind: CardKind::Terminal,
            path: None,
            root: None,
            explorer: false,
            url: None,
            sidebar: None,
            sidebar_top: false,
            zoom: None,
            tabs: vec![],
            active_tab: 0,
            session: Some("s1".into()),
            kitty_keys: true,
            agent_session: Some("a1".into()),
            agent_kind: Some("claude".into()),
            agent_title: Some("a task".into()),
            number: 7,
            protected: false,
            cover_of: None,
        }
    }

    fn workspace(id: &str) -> SavedWorkspace {
        SavedWorkspace {
            id: id.into(),
            name: "workspace 1".into(),
            viewport: INITIAL_VIEWPORT,
            focused: Some("a".into()),
        }
    }

    #[test]
    fn a_name_becomes_a_file_name_that_stays_in_the_folder() {
        assert_eq!(file_stem("  site  ").as_deref(), Some("site"));
        assert_eq!(file_stem("a/b:c\\d").as_deref(), Some("a-b-c-d"));
        assert_eq!(file_stem("../../etc").as_deref(), Some("-..-etc"));
        assert_eq!(file_stem(".hidden").as_deref(), Some("hidden"));
        assert_eq!(file_stem("tab\there").as_deref(), Some("tab-here"));
        assert_eq!(file_stem("   "), None);
        assert_eq!(file_stem("..."), None);
        let dir = Path::new("/d");
        assert_eq!(path(dir, " a/b "), Some(PathBuf::from("/d/a-b.json")));
        assert_eq!(path(dir, ".."), None);
    }

    #[test]
    fn a_snapshot_holds_one_workspace_and_only_its_groups() {
        let cards = [
            card("a", "w1", Some("g1")),
            card("b", "w2", Some("g2")),
            card("c", "w1", None),
        ];
        let groups = [
            SavedGroup {
                id: "g1".into(),
                name: "mine".into(),
            },
            SavedGroup {
                id: "g2".into(),
                name: "theirs".into(),
            },
        ];
        let value = snapshot_value("site", &workspace("w1"), &cards, &groups);
        let saved = parse_layout_allowing_empty(&value.to_string()).unwrap();
        let ids: Vec<&str> = saved.cards.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["a", "c"]);
        assert_eq!(saved.groups.len(), 1);
        assert_eq!(saved.groups[0].name, "mine");
        assert_eq!(saved.workspaces.len(), 1);
        assert_eq!(saved.workspaces[0].name, "site");
        assert_eq!(saved.workspaces[0].focused.as_deref(), Some("a"));
    }

    #[test]
    fn a_snapshot_leaves_out_what_belongs_to_the_running_shells() {
        let value = snapshot_value("site", &workspace("w1"), &[card("a", "w1", None)], &[]);
        let text = value.to_string();
        for key in [
            "session",
            "kittyKeys",
            "agentSession",
            "agentKind",
            "agentTitle",
            "number",
        ] {
            assert!(!text.contains(&format!("\"{key}\"")), "{key}");
        }
        assert!(text.contains("\"cwd\":\"/tmp\""));
    }

    #[test]
    fn written_listed_read_and_written_over() {
        let dir = std::env::temp_dir().join(format!("ift-saved-ws-{}", uuid::Uuid::new_v4()));
        assert!(list(&dir).is_empty(), "no folder yet");
        assert_eq!(read(&dir, "site"), None);
        write(&dir, "site", "one").unwrap();
        write(&dir, "Api", "x").unwrap();
        write(&dir, "site", "two").unwrap();
        std::fs::write(dir.join("notes.txt"), "not one").unwrap();
        assert_eq!(list(&dir), ["Api", "site"]);
        assert_eq!(read(&dir, "site").as_deref(), Some("two"));
        assert!(write(&dir, "  ", "x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
