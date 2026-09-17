//! The saved canvas: what goes into workspace.json and what comes back.
//! Port of savedLayout.ts and its tests.
//!
//! What is NOT saved is the interesting part. Pane ids, agent state, output
//! and event timestamps and errors are runtime facts about a shell that no
//! longer exists when this file is read again; persisting `agent` would
//! resurrect a card claiming to be working days after the agent died.
//! Scrollback does not come back either: a restored card is a fresh shell
//! in the saved directory, not a reattached session (that is the tmux
//! backend, v2).
//!
//! Parsing is total: every field is checked and anything malformed is
//! DROPPED rather than failing. The file is written several times a minute
//! by a program that can be force-quit, so a partly-written version has to
//! start the app with whatever survived.
//!
//! Byte-compatible with the Tauri app on purpose, so a native build opens
//! the existing canvas and the Tauri app can open one this build wrote:
//! same key order, same number formatting (`25`, not `25.0`), two-space
//! pretty printing. `serialise_layout` builds a `Value` explicitly for that
//! reason instead of deriving Serialize. `persistence` in the ui crate does
//! the loading and saving; `layout.rs` in the backend owns the path and the
//! write-then-rename.
//!
//! Named saved_layout rather than layout because `layout.rs` is where a new
//! card goes; the reference once lost a file to that confusion.
use crate::chrome::clamp_ui_scale;
use crate::grid::Rect;
use crate::palette_usage::{parse_usage, Usage};
use crate::viewport::Viewport;
use serde_json::{json, Map, Value};

/// Bumped when a change cannot be read by the tolerant parser below.
///
/// 3: workspaces. A version-2 file has cards with no `workspaceId` and no
/// workspace list; `ensure_workspace` puts every such card on a default one.
///
/// 2: `title` means a name somebody CHOSE and nothing else. Under 1 it was
/// also seeded from the directory and overwritten by the shell's OSC title,
/// so a v1 title cannot be told from a stale Claude task name; they are
/// discarded on read.
pub const LAYOUT_VERSION: u64 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CardKind {
    #[default]
    Terminal,
    Editor,
    Diff,
    Browser,
    Transcript,
}

impl CardKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CardKind::Terminal => "terminal",
            CardKind::Editor => "editor",
            CardKind::Diff => "diff",
            CardKind::Browser => "browser",
            CardKind::Transcript => "transcript",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SavedCard {
    pub id: String,
    /// Empty in a file written before workspaces; assigned on load.
    pub workspace_id: String,
    pub rect: Rect,
    pub z: f64,
    pub title: String,
    pub cwd: String,
    pub group_id: Option<String>,
    pub soft_group_id: Option<String>,
    pub split_from: Option<String>,
    pub kind: CardKind,
    pub path: Option<String>,
    pub root: Option<String>,
    pub explorer: bool,
    pub url: Option<String>,
    /// Sidebar width in card px; `None` is the default width.
    pub sidebar: Option<f64>,
    pub sidebar_top: bool,
    /// A browser card's page zoom; `None` means the config default.
    pub zoom: Option<f64>,
    /// This card's shell, as an opaque handle in whichever backend holds it
    /// (a tmux window id like `@7`, or a daemon session id).
    ///
    /// The ONE runtime fact about a shell that is saved. Everything else is
    /// deliberately left behind, because a restored card must not claim to
    /// be working days later; this is the opposite case. Under tmux or the
    /// daemon backend the shell really is still there, and without this the
    /// card would start a second one beside it and orphan the first.
    ///
    /// Ignored by the local backend, and by any launch where the session has
    /// gone: the card is then a fresh shell in its directory, as before.
    pub session: Option<String>,
    /// Whether the program in this pane speaks the kitty keyboard protocol.
    ///
    /// Saved because it cannot be learned again. A program announces itself
    /// once, at startup, by asking `CSI ? u`; Claude Code was measured
    /// never re-asking, on a resize or a keystroke or anything else. Under
    /// the daemon backend a card comes back to a program that started hours
    /// ago, and the ring holds only the last few MiB, so the announcement
    /// is long gone from the history we replay. Without this the gate stays
    /// shut after every relaunch and Shift+Enter sends the prompt instead
    /// of breaking the line.
    pub kitty_keys: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedGroup {
    pub id: String,
    pub name: String,
}

/// The same shape the live model uses; nothing about a workspace is runtime-only.
pub type SavedWorkspace = crate::workspaces::Workspace;

#[derive(Clone, Debug, PartialEq)]
pub struct SavedLayout {
    pub version: u64,
    pub viewport: Viewport,
    /// The chrome multiplier. Here rather than in the config file, which is
    /// hand-edited and watched; an app writing to it would fight an editor.
    pub ui_scale: f64,
    /// What the palette has been used to run.
    pub usage: Usage,
    pub focused_id: Option<String>,
    pub workspaces: Vec<SavedWorkspace>,
    pub active_workspace_id: Option<String>,
    pub groups: Vec<SavedGroup>,
    pub cards: Vec<SavedCard>,
}

/// A number the way JSON.stringify writes it: whole values without a
/// decimal point, so the file the Tauri app wrote and the one this build
/// writes are the same bytes.
fn num(n: f64) -> Value {
    if n.fract() == 0. && n.abs() < 9e15 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

fn opt_str(s: &Option<String>) -> Value {
    s.as_ref().map_or(Value::Null, |s| Value::String(s.clone()))
}

fn opt_num(n: Option<f64>) -> Value {
    n.map_or(Value::Null, num)
}

fn viewport_value(v: &Viewport) -> Value {
    json!({ "x": num(v.x), "y": num(v.y), "scale": num(v.scale) })
}

fn usage_value(usage: &Usage) -> Value {
    let mut map = Map::new();
    for (key, u) in &usage.0 {
        map.insert(key.clone(), json!({ "n": num(u.n), "at": num(u.at) }));
    }
    Value::Object(map)
}

fn card_value(c: &SavedCard) -> Value {
    let mut card = json!({
        "id": c.id,
        "workspaceId": c.workspace_id,
        "rect": { "x": num(c.rect.x), "y": num(c.rect.y), "w": num(c.rect.w), "h": num(c.rect.h) },
        "z": num(c.z),
        "title": c.title,
        "cwd": c.cwd,
        "groupId": opt_str(&c.group_id),
        "softGroupId": opt_str(&c.soft_group_id),
        "splitFrom": opt_str(&c.split_from),
        "kind": c.kind.as_str(),
        "path": opt_str(&c.path),
        "root": opt_str(&c.root),
        "explorer": c.explorer,
        "url": opt_str(&c.url),
        "sidebar": opt_num(c.sidebar),
        "sidebarTop": c.sidebar_top,
        "zoom": opt_num(c.zoom),
    });
    // Written ONLY when there is one. A card with no session behind it
    // leaves the file exactly as it was, so a canvas that never used tmux or
    // the daemon backend round-trips byte for byte and the file does not
    // grow a column of nulls for a backend nobody chose.
    if let Some(session) = &c.session {
        if let Some(map) = card.as_object_mut() {
            map.insert("session".into(), Value::String(session.clone()));
        }
    }
    // Same rule as `session`: written only when true, so a canvas whose
    // cards run plain shells does not grow a column of falses.
    if c.kitty_keys {
        if let Some(map) = card.as_object_mut() {
            map.insert("kittyKeys".into(), Value::Bool(true));
        }
    }
    card
}

/// The file's JSON value. The caller maps its live cards to `SavedCard`,
/// which is where runtime state is left behind.
#[allow(clippy::too_many_arguments)]
pub fn serialise_layout(
    cards: &[SavedCard],
    groups: &[SavedGroup],
    viewport: &Viewport,
    focused_id: Option<&str>,
    ui_scale: f64,
    usage: &Usage,
    workspaces: &[SavedWorkspace],
    active_workspace_id: Option<&str>,
) -> Value {
    json!({
        "version": LAYOUT_VERSION,
        "viewport": viewport_value(viewport),
        "uiScale": num(ui_scale),
        "usage": usage_value(usage),
        "focusedId": focused_id,
        "workspaces": workspaces.iter().map(|w| json!({ "id": w.id, "name": w.name, "viewport": viewport_value(&w.viewport) })).collect::<Vec<_>>(),
        "activeWorkspaceId": active_workspace_id,
        "groups": groups.iter().map(|g| json!({ "id": g.id, "name": g.name })).collect::<Vec<_>>(),
        "cards": cards.iter().map(card_value).collect::<Vec<_>>(),
    })
}

/// The text written to disk: two-space pretty printing, as the Tauri app
/// wrote it, plus no trailing newline, also as it wrote it.
pub fn layout_text(layout: &Value) -> String {
    serde_json::to_string_pretty(layout).expect("a Value serialises")
}

fn finite(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|n| n.is_finite())
}

/// A non-empty string, or nothing.
fn non_empty(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn as_rect(v: Option<&Value>) -> Option<Rect> {
    let r = v?.as_object()?;
    let (x, y, w, h) = (
        finite(r.get("x"))?,
        finite(r.get("y"))?,
        finite(r.get("w"))?,
        finite(r.get("h"))?,
    );
    // A zero or negative size would render as an invisible card that still
    // owns a shell, which is worse than dropping it.
    if w <= 0. || h <= 0. {
        return None;
    }
    Some(Rect { x, y, w, h })
}

fn as_card(v: &Value) -> Option<SavedCard> {
    let c = v.as_object()?;
    // A card with no id or no directory cannot be restored: the id keys its
    // group membership and the directory is where its shell has to start.
    let rect = as_rect(c.get("rect"))?;
    let id = non_empty(c.get("id"))?;
    let cwd = non_empty(c.get("cwd"))?;
    let kind_str = c.get("kind").and_then(Value::as_str);
    let path = non_empty(c.get("path"));
    let url = non_empty(c.get("url"));
    // "tmuxWindow" is the spelling one evening of tmux-backend save files
    // used; "session" is what every backend's opaque handle is called now.
    // Read both, preferring the current key, so those canvases still load.
    let session = non_empty(c.get("session")).or_else(|| non_empty(c.get("tmuxWindow")));
    let kitty_keys = c.get("kittyKeys").and_then(Value::as_bool).unwrap_or(false);
    // A card is a terminal unless it says otherwise; an editor without a
    // path is an untitled buffer, whose text lives in its draft. A browser
    // without a url or a transcript without a path has nothing to show and
    // comes back as a terminal.
    let kind = match kind_str {
        Some("editor") => CardKind::Editor,
        Some("diff") => CardKind::Diff,
        Some("transcript") if path.is_some() => CardKind::Transcript,
        Some("browser") if url.is_some() => CardKind::Browser,
        _ => CardKind::Terminal,
    };
    let file_kind = matches!(kind_str, Some("editor" | "diff"));
    Some(SavedCard {
        id,
        workspace_id: c
            .get("workspaceId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        rect,
        z: finite(c.get("z")).unwrap_or(0.),
        title: c
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        cwd,
        group_id: non_empty(c.get("groupId")),
        soft_group_id: non_empty(c.get("softGroupId")),
        split_from: non_empty(c.get("splitFrom")),
        kind,
        path: if file_kind || kind_str == Some("transcript") {
            path
        } else {
            None
        },
        root: if file_kind {
            non_empty(c.get("root"))
        } else {
            None
        },
        explorer: file_kind && c.get("explorer") == Some(&Value::Bool(true)),
        url: if kind_str == Some("browser") {
            url
        } else {
            None
        },
        sidebar: finite(c.get("sidebar")).filter(|n| *n > 0.),
        sidebar_top: c.get("sidebarTop") == Some(&Value::Bool(true)),
        zoom: if kind_str == Some("browser") {
            finite(c.get("zoom")).filter(|n| *n > 0.)
        } else {
            None
        },
        session,
        kitty_keys,
    })
}

/// A viewport with each bad field replaced: a zero or negative scale
/// renders nothing at all, so a hand-edited one falls back rather than
/// opening an invisible canvas.
fn as_viewport(v: Option<&Value>) -> Viewport {
    let empty = Map::new();
    let vp = v.and_then(Value::as_object).unwrap_or(&empty);
    Viewport {
        x: finite(vp.get("x")).unwrap_or(0.),
        y: finite(vp.get("y")).unwrap_or(0.),
        scale: finite(vp.get("scale")).filter(|s| *s > 0.).unwrap_or(1.),
    }
}

fn as_workspace(v: &Value) -> Option<SavedWorkspace> {
    let w = v.as_object()?;
    Some(SavedWorkspace {
        id: non_empty(w.get("id"))?,
        name: w
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        viewport: as_viewport(w.get("viewport")),
    })
}

fn as_group(v: &Value) -> Option<SavedGroup> {
    let g = v.as_object()?;
    Some(SavedGroup {
        id: non_empty(g.get("id"))?,
        name: g
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

fn array<'a>(root: &'a Map<String, Value>, key: &str) -> impl Iterator<Item = &'a Value> {
    root.get(key)
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
}

/// The palette's history from a saved file, independent of the layout.
/// `parse_layout` returns nothing for a file with no cards, correctly, but
/// the history has nothing to do with cards: closing every card must not
/// erase what the palette had learned.
pub fn parse_saved_usage(text: &str) -> Usage {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|raw| {
            raw.as_object()
                .map(|r| parse_usage(r.get("usage").unwrap_or(&Value::Null)))
        })
        .unwrap_or_default()
}

/// True when the file was written by a build newer than this one. Kept
/// apart from `parse_layout` returning nothing, because the two demand
/// opposite behaviour: garbage is replaced on the next save, a newer file
/// must NEVER be, since this build would rewrite it minus everything it does
/// not know about. That happened once with a stale release bundle.
pub fn is_newer_than_this_build(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|raw| finite(raw.as_object()?.get("version")))
        .is_some_and(|v| v > LAYOUT_VERSION as f64)
}

/// Reads a saved layout, keeping whatever is valid. `None` only when there
/// is nothing usable at all: bad JSON, a version this build predates, or no
/// cards left after validation. A layout with no cards is the same as no
/// layout; letting it through would be indistinguishable from a first run.
pub fn parse_layout(text: &str) -> Option<SavedLayout> {
    let raw: Value = serde_json::from_str(text).ok()?;
    let root = raw.as_object()?;
    // Forward compatibility only goes one way: a newer file may contain
    // fields this build would silently drop.
    let version = finite(root.get("version"))?;
    if version > LAYOUT_VERSION as f64 {
        return None;
    }

    // Duplicate ids crash the render, and a saved file is only ever as good
    // as the process that wrote it, so the first of each id wins.
    let mut cards: Vec<SavedCard> = vec![];
    for card in array(root, "cards").filter_map(as_card) {
        if !cards.iter().any(|c| c.id == card.id) {
            cards.push(card);
        }
    }
    if cards.is_empty() {
        return None;
    }

    // Version 1 conflated a chosen name with the creation directory and
    // with whatever the shell last wrote, so none of its titles is a name.
    if version < 2. {
        for card in &mut cards {
            card.title.clear();
        }
    }

    // A group nobody belongs to has no frame and cannot be reached, and a
    // card pointing at a group that did not survive would be invisibly
    // grouped.
    let groups: Vec<SavedGroup> = array(root, "groups")
        .filter_map(as_group)
        .filter(|g| cards.iter().any(|c| c.group_id.as_deref() == Some(&g.id)))
        .collect();
    for card in &mut cards {
        if card
            .group_id
            .as_ref()
            .is_some_and(|id| !groups.iter().any(|g| g.id == *id))
        {
            card.group_id = None;
        }
    }

    // A card pointing at a workspace that did not survive is cleared rather
    // than dropped; `ensure_workspace` puts it on the first one.
    let workspaces: Vec<SavedWorkspace> =
        array(root, "workspaces").filter_map(as_workspace).collect();
    for card in &mut cards {
        if !card.workspace_id.is_empty() && !workspaces.iter().any(|w| w.id == card.workspace_id) {
            card.workspace_id.clear();
        }
    }

    let focused_id = root
        .get("focusedId")
        .and_then(Value::as_str)
        .filter(|id| cards.iter().any(|c| c.id == *id))
        .map(String::from);

    let active_workspace_id = root
        .get("activeWorkspaceId")
        .and_then(Value::as_str)
        .filter(|id| workspaces.iter().any(|w| w.id == *id))
        .map(String::from)
        .or_else(|| workspaces.first().map(|w| w.id.clone()));

    Some(SavedLayout {
        version: LAYOUT_VERSION,
        viewport: as_viewport(root.get("viewport")),
        // Clamped on the way in as well as out: a hand-edited 0.001 would
        // render the interface invisible with no way to read the key back.
        ui_scale: clamp_ui_scale(root.get("uiScale").and_then(Value::as_f64).unwrap_or(1.)),
        usage: parse_usage(root.get("usage").unwrap_or(&Value::Null)),
        focused_id,
        workspaces,
        active_workspace_id,
        groups,
        cards,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette_usage::Use;

    fn card() -> SavedCard {
        SavedCard {
            id: "c1".into(),
            workspace_id: "w1".into(),
            rect: Rect {
                x: 25.,
                y: 25.,
                w: 600.,
                h: 800.,
            },
            z: 0.,
            kitty_keys: false,
            title: "api".into(),
            cwd: "/Users/ekinertac/Code/api".into(),
            group_id: None,
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
            session: None,
        }
    }

    fn card_json() -> Value {
        card_value(&card())
    }

    fn with(mut v: Value, over: &[(&str, Value)]) -> Value {
        for (k, val) in over {
            v[k] = val.clone();
        }
        v
    }

    fn vp(x: f64, y: f64, scale: f64) -> Viewport {
        Viewport { x, y, scale }
    }

    // The workspace has to exist, or the parser correctly clears the card's
    // reference to it, which is a different behaviour, tested below.
    fn ws() -> Vec<SavedWorkspace> {
        vec![SavedWorkspace {
            id: "w1".into(),
            name: "w".into(),
            viewport: vp(0., 0., 1.),
        }]
    }

    fn round_trip(cards: &[SavedCard], groups: &[SavedGroup]) -> Option<SavedLayout> {
        let focused = cards.first().map(|c| c.id.as_str());
        let text = layout_text(&serialise_layout(
            cards,
            groups,
            &vp(10., 20., 0.5),
            focused,
            1.,
            &Usage::default(),
            &ws(),
            Some("w1"),
        ));
        parse_layout(&text)
    }

    fn file(version: u64, cards: Vec<Value>) -> String {
        json!({ "version": version, "cards": cards }).to_string()
    }

    #[test]
    fn a_layout_survives_a_round_trip() {
        let saved = round_trip(&[card()], &[]).unwrap();
        assert_eq!(
            saved,
            SavedLayout {
                version: LAYOUT_VERSION,
                viewport: vp(10., 20., 0.5),
                ui_scale: 1.,
                usage: Usage::default(),
                focused_id: Some("c1".into()),
                workspaces: ws(),
                active_workspace_id: Some("w1".into()),
                groups: vec![],
                cards: vec![card()],
            }
        );
    }

    #[test]
    fn group_membership_survives_a_round_trip() {
        let c = SavedCard {
            group_id: Some("g1".into()),
            ..card()
        };
        let saved = round_trip(
            &[c],
            &[SavedGroup {
                id: "g1".into(),
                name: "humbl.ai".into(),
            }],
        )
        .unwrap();
        assert_eq!(
            saved.groups,
            [SavedGroup {
                id: "g1".into(),
                name: "humbl.ai".into()
            }]
        );
        assert_eq!(saved.cards[0].group_id.as_deref(), Some("g1"));
    }

    // Runtime facts about a shell that no longer exists stay out of the file.
    // The typed SavedCard cannot carry them, so the check is the key set.
    // The one runtime fact that IS saved, and only when there is one: a
    // canvas that never used tmux or the daemon backend has to round-trip
    // byte for byte, which the real-file test also checks.
    #[test]
    fn a_session_is_written_only_when_a_card_has_one() {
        let plain = card_value(&card());
        assert!(
            plain.get("session").is_none(),
            "a card with no session must not grow a null"
        );
        let with = card_value(&SavedCard {
            session: Some("@7".into()),
            ..card()
        });
        assert_eq!(with["session"], "@7");
        // And it comes back.
        assert_eq!(as_card(&with).unwrap().session.as_deref(), Some("@7"));
        assert_eq!(as_card(&plain).unwrap().session, None);
    }

    // A program announces the kitty keyboard protocol once, at startup,
    // and Claude Code was measured never re-announcing. An adopted card's
    // startup is long out of the daemon's ring, so the save file is the
    // only thing that can still know, and Shift+Enter depends on it.
    #[test]
    fn the_kitty_flag_survives_the_save_file() {
        let saved = round_trip(
            &[SavedCard {
                kitty_keys: true,
                ..card()
            }],
            &[],
        )
        .unwrap();
        assert!(saved.cards[0].kitty_keys);
    }

    // Written only when true, like `session`, so a canvas of plain shells
    // does not grow a column of falses.
    #[test]
    fn a_plain_shell_writes_no_kitty_flag() {
        let text = layout_text(&serialise_layout(
            &[card()],
            &[],
            &vp(10., 20., 0.5),
            None,
            1.,
            &Usage::default(),
            &ws(),
            Some("w1"),
        ));
        assert!(!text.contains("kittyKeys"), "{text}");
    }

    #[test]
    fn a_session_survives_the_save_file() {
        let saved = round_trip(
            &[SavedCard {
                session: Some("abc-123".into()),
                ..card()
            }],
            &[],
        )
        .unwrap();
        assert_eq!(saved.cards[0].session.as_deref(), Some("abc-123"));
    }

    // Canvases written by the tmux evening still load: the key it used is
    // read as a fallback, even though it is never written any more.
    #[test]
    fn the_old_tmux_window_key_is_still_read() {
        let with_legacy_key = with(card_json(), &[("tmuxWindow", json!("@3"))]);
        assert_eq!(
            as_card(&with_legacy_key).unwrap().session.as_deref(),
            Some("@3")
        );
    }

    #[test]
    fn runtime_state_is_not_saved() {
        let out = serialise_layout(
            &[card()],
            &[],
            &vp(0., 0., 1.),
            None,
            1.,
            &Usage::default(),
            &[],
            None,
        );
        let mut keys: Vec<&str> = out["cards"][0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "cwd",
                "explorer",
                "groupId",
                "id",
                "kind",
                "path",
                "rect",
                "root",
                "sidebar",
                "sidebarTop",
                "softGroupId",
                "splitFrom",
                "title",
                "url",
                "workspaceId",
                "z",
                "zoom"
            ]
        );
    }

    #[test]
    fn bad_json_reads_as_no_layout_rather_than_failing() {
        assert!(parse_layout("{\"cards\": [").is_none());
        assert!(parse_layout("null").is_none());
        assert!(parse_layout("").is_none());
    }

    #[test]
    fn a_layout_with_no_cards_is_the_same_as_no_layout() {
        assert!(parse_layout(&file(1, vec![])).is_none());
    }

    // A newer file may hold fields this build would drop. Older ones are still read.
    #[test]
    fn a_version_from_the_future_is_refused() {
        assert!(parse_layout(&file(99, vec![card_json()])).is_none());
        assert!(parse_layout(&file(1, vec![card_json()])).is_some());
    }

    fn ids(layout: &SavedLayout) -> Vec<&str> {
        layout.cards.iter().map(|c| c.id.as_str()).collect()
    }

    // Force-quit mid-write, or a hand-mangled file: keep what survived.
    #[test]
    fn a_malformed_card_is_dropped_not_fatal() {
        let layout = parse_layout(&file(
            1,
            vec![
                card_json(),
                json!({"id": "x"}),
                with(card_json(), &[("id", json!("c2"))]),
            ],
        ))
        .unwrap();
        assert_eq!(ids(&layout), ["c1", "c2"]);
    }

    #[test]
    fn a_card_with_no_directory_is_dropped_its_shell_has_nowhere_to_start() {
        let layout = parse_layout(&file(
            1,
            vec![
                card_json(),
                with(card_json(), &[("id", json!("c2")), ("cwd", json!(""))]),
            ],
        ))
        .unwrap();
        assert_eq!(ids(&layout), ["c1"]);
    }

    #[test]
    fn a_zero_sized_card_is_dropped_rather_than_restored_invisible() {
        let bad = with(
            card_json(),
            &[
                ("id", json!("c2")),
                ("rect", json!({"x": 0, "y": 0, "w": 0, "h": 100})),
            ],
        );
        let layout = parse_layout(&file(1, vec![card_json(), bad])).unwrap();
        assert_eq!(ids(&layout), ["c1"]);
    }

    // A group nobody belongs to has no frame and cannot be reached.
    #[test]
    fn a_group_whose_members_all_vanished_is_dropped() {
        let text =
            json!({"version": 1, "cards": [card_json()], "groups": [{"id": "g1", "name": "gone"}]})
                .to_string();
        assert!(parse_layout(&text).unwrap().groups.is_empty());
    }

    #[test]
    fn a_card_pointing_at_a_group_that_did_not_survive_comes_back_loose() {
        let text = json!({"version": 1, "cards": [with(card_json(), &[("groupId", json!("ghost"))])], "groups": []}).to_string();
        assert_eq!(parse_layout(&text).unwrap().cards[0].group_id, None);
    }

    #[test]
    fn a_focus_on_a_card_that_did_not_survive_is_dropped() {
        let text = json!({"version": 1, "cards": [card_json()], "focusedId": "gone"}).to_string();
        assert_eq!(parse_layout(&text).unwrap().focused_id, None);
    }

    #[test]
    fn a_missing_or_absurd_viewport_falls_back_to_the_origin_at_100_percent() {
        assert_eq!(
            parse_layout(&file(1, vec![card_json()])).unwrap().viewport,
            vp(0., 0., 1.)
        );
        let bad = json!({"version": 1, "cards": [card_json()], "viewport": {"x": "no", "y": null, "scale": 0}}).to_string();
        assert_eq!(parse_layout(&bad).unwrap().viewport, vp(0., 0., 1.));
    }

    // Duplicate ids crash the render outright; a hot reload once appended a
    // whole restored canvas a second time and saved the result.
    #[test]
    fn a_duplicate_card_id_is_dropped_rather_than_crashing_the_render() {
        // Version 2: version 1 discards titles, and the point here is which
        // of the duplicate pair survives, not the migration.
        let layout = parse_layout(&file(
            2,
            vec![
                card_json(),
                with(card_json(), &[("title", json!("copy"))]),
                with(card_json(), &[("id", json!("c2"))]),
            ],
        ))
        .unwrap();
        assert_eq!(ids(&layout), ["c1", "c2"]);
        assert_eq!(layout.cards[0].title, "api");
    }

    #[test]
    fn the_ui_scale_survives_a_round_trip() {
        let text = layout_text(&serialise_layout(
            &[card()],
            &[],
            &vp(0., 0., 1.),
            None,
            1.4,
            &Usage::default(),
            &[],
            None,
        ));
        assert_eq!(parse_layout(&text).unwrap().ui_scale, 1.4);
    }

    // A hand-edited 0.001 would render the interface too small to read the key back.
    #[test]
    fn an_absurd_ui_scale_is_clamped_rather_than_obeyed() {
        let at = |v: Value| {
            parse_layout(&json!({"version": 1, "cards": [card_json()], "uiScale": v}).to_string())
                .unwrap()
                .ui_scale
        };
        assert_eq!(at(json!(0.001)), 0.6);
        assert_eq!(at(json!(50)), 2.5);
        assert_eq!(at(json!("big")), 1.);
        assert_eq!(at(Value::Null), 1.);
    }

    // Under version 1, `title` was also seeded from the directory and
    // overwritten by any OSC title the shell wrote; it is not a name at all.
    #[test]
    fn titles_saved_under_version_1_are_discarded() {
        let layout = parse_layout(&file(
            1,
            vec![with(
                card_json(),
                &[("title", json!("✳ Install and Configure GUI SQLite Viewer"))],
            )],
        ))
        .unwrap();
        assert_eq!(layout.cards[0].title, "");
    }

    #[test]
    fn titles_saved_under_version_2_are_kept() {
        let layout = parse_layout(&file(
            2,
            vec![with(card_json(), &[("title", json!("deploy"))])],
        ))
        .unwrap();
        assert_eq!(layout.cards[0].title, "deploy");
    }

    #[test]
    fn palette_history_survives_a_round_trip() {
        let usage = Usage(vec![(
            "commands:card.close".into(),
            Use {
                n: 3.,
                at: 1789000000000.,
            },
        )]);
        let text = layout_text(&serialise_layout(
            &[card()],
            &[],
            &vp(0., 0., 1.),
            None,
            1.,
            &usage,
            &[],
            None,
        ));
        assert_eq!(parse_layout(&text).unwrap().usage, usage);
    }

    // parseLayout refuses a file with no cards, correctly, but the palette
    // history has nothing to do with cards.
    #[test]
    fn palette_history_is_readable_from_a_file_with_no_cards() {
        let text = json!({"version": 2, "cards": [], "usage": {"commands:a": {"n": 1, "at": 5}}})
            .to_string();
        assert!(parse_layout(&text).is_none());
        assert_eq!(
            parse_saved_usage(&text),
            Usage(vec![("commands:a".into(), Use { n: 1., at: 5. })])
        );
    }

    #[test]
    fn a_file_with_no_history_reads_as_an_empty_one() {
        assert!(parse_saved_usage("{\"version\":2}").is_empty());
        assert!(parse_saved_usage("{ broken").is_empty());
    }

    // A version-2 file has no workspace list and cards with no workspaceId.
    // Nothing is decided here; `ensure_workspace` puts them on a default canvas.
    #[test]
    fn a_layout_from_before_workspaces_loads_with_its_cards_unassigned() {
        let mut c = card_json();
        c.as_object_mut().unwrap().remove("workspaceId");
        let layout = parse_layout(&file(2, vec![c])).unwrap();
        assert_eq!(layout.cards[0].workspace_id, "");
        assert!(layout.workspaces.is_empty());
        assert_eq!(layout.active_workspace_id, None);
    }

    #[test]
    fn workspaces_survive_a_round_trip_viewport_and_all() {
        let ws = vec![SavedWorkspace {
            id: "w1".into(),
            name: "humbl.ai".into(),
            viewport: vp(10., 20., 0.5),
        }];
        let text = layout_text(&serialise_layout(
            &[card()],
            &[],
            &vp(0., 0., 1.),
            None,
            1.,
            &Usage::default(),
            &ws,
            Some("w1"),
        ));
        let layout = parse_layout(&text).unwrap();
        assert_eq!(layout.workspaces, ws);
        assert_eq!(layout.active_workspace_id.as_deref(), Some("w1"));
    }

    // A card pointing at a canvas that did not survive would be invisible forever.
    #[test]
    fn a_card_on_a_workspace_that_is_gone_comes_back_unassigned() {
        let text = json!({"version": 3, "cards": [with(card_json(), &[("workspaceId", json!("ghost"))])], "workspaces": []}).to_string();
        assert_eq!(parse_layout(&text).unwrap().cards[0].workspace_id, "");
    }

    #[test]
    fn an_active_workspace_that_is_gone_falls_back_to_the_first() {
        let text = json!({"version": 3, "cards": [card_json()], "workspaces": [{"id": "w1", "name": "a", "viewport": {"x": 0, "y": 0, "scale": 1}}], "activeWorkspaceId": "gone"}).to_string();
        assert_eq!(
            parse_layout(&text).unwrap().active_workspace_id.as_deref(),
            Some("w1")
        );
    }

    // A zero scale renders nothing; a hand-edited one must not open a blank canvas.
    #[test]
    fn an_absurd_workspace_scale_falls_back_rather_than_being_obeyed() {
        let text = json!({"version": 3, "cards": [card_json()], "workspaces": [{"id": "w1", "name": "a", "viewport": {"x": 0, "y": 0, "scale": 0}}]}).to_string();
        assert_eq!(
            parse_layout(&text).unwrap().workspaces[0].viewport.scale,
            1.
        );
    }

    #[test]
    fn a_soft_group_survives_the_round_trip_and_a_file_without_one_reads_as_none() {
        let a = SavedCard {
            soft_group_id: Some("s1".into()),
            ..card()
        };
        let b = SavedCard {
            id: "c2".into(),
            soft_group_id: Some("s1".into()),
            ..card()
        };
        let saved = round_trip(&[a, b], &[]).unwrap();
        assert_eq!(saved.cards[0].soft_group_id.as_deref(), Some("s1"));
        assert_eq!(saved.cards[1].soft_group_id.as_deref(), Some("s1"));
        let mut old = card_json();
        old.as_object_mut().unwrap().remove("softGroupId");
        assert_eq!(
            parse_layout(&file(3, vec![old])).unwrap().cards[0].soft_group_id,
            None
        );
    }

    // A newer file is refused by parse_layout AND flagged, so the caller can stop saving.
    #[test]
    fn a_file_from_a_newer_build_is_recognised_garbage_is_not() {
        let newer = json!({"version": LAYOUT_VERSION + 1, "cards": []}).to_string();
        assert!(is_newer_than_this_build(&newer));
        assert!(parse_layout(&newer).is_none());
        assert!(!is_newer_than_this_build(
            &json!({"version": LAYOUT_VERSION, "cards": []}).to_string()
        ));
        assert!(!is_newer_than_this_build("not json"));
        assert!(!is_newer_than_this_build("null"));
    }

    #[test]
    fn an_editor_card_keeps_its_file_and_one_without_a_file_is_an_untitled_editor() {
        let ed = SavedCard {
            kind: CardKind::Editor,
            path: Some("/tmp/a.ts".into()),
            ..card()
        };
        let broken = SavedCard {
            id: "c2".into(),
            kind: CardKind::Editor,
            ..card()
        };
        let saved = round_trip(&[ed, broken], &[]).unwrap();
        assert_eq!(saved.cards[0].kind, CardKind::Editor);
        assert_eq!(saved.cards[0].path.as_deref(), Some("/tmp/a.ts"));
        assert_eq!(saved.cards[1].kind, CardKind::Editor);
        assert_eq!(saved.cards[1].path, None);
        // A file from before editor cards has no kind and no path.
        let mut old = card_json();
        old.as_object_mut().unwrap().remove("kind");
        old.as_object_mut().unwrap().remove("path");
        let restored = parse_layout(&file(3, vec![old])).unwrap();
        assert_eq!(restored.cards[0].kind, CardKind::Terminal);
        assert_eq!(restored.cards[0].path, None);
    }

    #[test]
    fn an_explorer_root_and_its_visibility_round_trip_on_editors_only() {
        let ed = SavedCard {
            kind: CardKind::Editor,
            root: Some("/tmp".into()),
            explorer: true,
            ..card()
        };
        let term = SavedCard {
            id: "c2".into(),
            root: Some("/tmp".into()),
            explorer: true,
            ..card()
        };
        let saved = round_trip(&[ed, term], &[]).unwrap();
        assert_eq!(saved.cards[0].root.as_deref(), Some("/tmp"));
        assert!(saved.cards[0].explorer);
        assert_eq!(saved.cards[1].root, None);
        assert!(!saved.cards[1].explorer);
    }

    #[test]
    fn a_browser_card_keeps_its_url_and_one_without_a_url_comes_back_as_a_terminal() {
        let web = SavedCard {
            kind: CardKind::Browser,
            url: Some("https://example.com".into()),
            ..card()
        };
        let blank = SavedCard {
            id: "c2".into(),
            kind: CardKind::Browser,
            ..card()
        };
        let term = SavedCard {
            id: "c3".into(),
            url: Some("https://example.com".into()),
            ..card()
        };
        let saved = round_trip(&[web, blank, term], &[]).unwrap();
        assert_eq!(saved.cards[0].kind, CardKind::Browser);
        assert_eq!(saved.cards[0].url.as_deref(), Some("https://example.com"));
        assert_eq!(saved.cards[1].kind, CardKind::Terminal);
        assert_eq!(saved.cards[2].url, None);
    }

    #[test]
    fn a_transcript_card_keeps_its_session_file_and_one_without_a_path_comes_back_as_a_terminal() {
        let tr = SavedCard {
            kind: CardKind::Transcript,
            path: Some("/s/abc.jsonl".into()),
            ..card()
        };
        let blank = SavedCard {
            id: "c2".into(),
            kind: CardKind::Transcript,
            ..card()
        };
        let saved = round_trip(&[tr, blank], &[]).unwrap();
        assert_eq!(saved.cards[0].kind, CardKind::Transcript);
        assert_eq!(saved.cards[0].path.as_deref(), Some("/s/abc.jsonl"));
        assert_eq!(saved.cards[1].kind, CardKind::Terminal);
    }

    #[test]
    fn a_sidebar_width_round_trips_and_garbage_means_the_default() {
        let wide = SavedCard {
            sidebar: Some(340.),
            ..card()
        };
        let bad = SavedCard {
            id: "c2".into(),
            sidebar: Some(-5.),
            ..card()
        };
        let saved = round_trip(&[wide, bad], &[]).unwrap();
        assert_eq!(saved.cards[0].sidebar, Some(340.));
        assert_eq!(saved.cards[1].sidebar, None);
    }

    #[test]
    fn the_sidebar_side_round_trips_and_defaults_to_beside() {
        let above = SavedCard {
            sidebar_top: true,
            ..card()
        };
        let beside = SavedCard {
            id: "c2".into(),
            ..card()
        };
        let saved = round_trip(&[above, beside], &[]).unwrap();
        assert!(saved.cards[0].sidebar_top);
        assert!(!saved.cards[1].sidebar_top);
    }

    #[test]
    fn a_browser_cards_page_zoom_round_trips_and_means_nothing_on_other_kinds() {
        let web = SavedCard {
            kind: CardKind::Browser,
            url: Some("https://x".into()),
            zoom: Some(1.5),
            ..card()
        };
        let term = SavedCard {
            id: "c2".into(),
            zoom: Some(1.5),
            ..card()
        };
        let saved = round_trip(&[web, term], &[]).unwrap();
        assert_eq!(saved.cards[0].zoom, Some(1.5));
        assert_eq!(saved.cards[1].zoom, None);
    }

    // Native check, the byte-compatibility claim itself: the Tauri app's real
    // save file parses, re-serialises to the same bytes, and the Tauri app
    // could therefore open what this build writes. Skipped where the file
    // is absent.
    //
    // One deliberate exception: "tmuxWindow" is written as "session" now
    // (still read under either spelling, see
    // `the_old_tmux_window_key_is_still_read`), so a real file saved before
    // that rename is normalised to the new spelling before the comparison
    // rather than the test pinning the old key forever.
    #[test]
    fn the_real_workspace_file_round_trips_byte_for_byte() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let path = std::path::Path::new(&home)
            .join("Library/Application Support/dev.ekinertac.infiniterm/workspace.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let layout = parse_layout(&text).expect("the real file parses");
        let again = layout_text(&serialise_layout(
            &layout.cards,
            &layout.groups,
            &layout.viewport,
            layout.focused_id.as_deref(),
            layout.ui_scale,
            &layout.usage,
            &layout.workspaces,
            layout.active_workspace_id.as_deref(),
        ));
        let expected = text.trim_end().replace("\"tmuxWindow\":", "\"session\":");
        assert_eq!(again, expected);
    }
}
