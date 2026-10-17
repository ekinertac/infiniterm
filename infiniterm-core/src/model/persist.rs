//! Loading the canvas at startup and saving it as it changes. Port of
//! `persistence.svelte.ts`.
//!
//! Nothing is saved until the load has finished: the model starts empty,
//! which is indistinguishable from a first run, and a save that fires first
//! writes that emptiness over the real canvas. `loaded` is set even when
//! there was nothing to load, because a genuine first run must still save.
//! A file from a NEWER build is never written back (`read_only`). Saving is
//! debounced by the ui (500 ms: a drag is one write, not sixty), which
//! calls `save_text` when `dirty_layout` has been set and the window passed.
use super::{Model, NewCard};

/// How many published canvases the host remembers, for working out what a
/// viewer's edit changed. A viewer that edits a canvas older than this many
/// publishes ago is resynced instead.
const PUBLISHED_KEPT: usize = 32;

/// What `Model::apply_remote_layout` did with a viewer's edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteApply {
    /// Made so; the app publishes the new canvas.
    Applied,
    /// Made on a canvas the host no longer remembers: dropped, and the host
    /// resyncs the viewer.
    Stale,
}
use crate::palette_usage::Usage;
use crate::saved_layout::{
    is_newer_than_this_build, layout_text, parse_layout, parse_saved_usage, serialise_layout,
    SavedCard, SavedGroup,
};

impl Model {
    /// Puts a saved canvas back. Restores in dependency order (workspaces,
    /// groups, cards) so no card ever points at something that does not
    /// exist yet. Only ever valid onto an empty canvas.
    pub fn load_layout(&mut self, text: Option<&str>) {
        if let Some(text) = text {
            if is_newer_than_this_build(text) {
                self.read_only = true;
                self.effects.push(super::Effect::Warn("workspace.json was written by a newer infiniterm; this build will not touch it".into()));
                self.notify("this build is older than your saved canvas: nothing will be saved");
                self.loaded = true;
                self.ensure_workspace();
                return;
            }
            // Read on its own: a file with no cards still has a palette
            // history worth keeping and parse_layout refuses such a file.
            self.usage = parse_saved_usage(text);
            if let (Some(saved), true) = (parse_layout(text), self.cards.is_empty()) {
                self.workspaces = saved.workspaces;
                self.active_workspace = saved.active_workspace_id;
                self.groups = saved
                    .groups
                    .into_iter()
                    .map(|g| super::Group {
                        id: g.id,
                        name: g.name,
                    })
                    .collect();
                for c in saved.cards {
                    let id = self.add_card(
                        &c.cwd,
                        NewCard {
                            id: Some(c.id),
                            rect: Some(c.rect),
                            group_id: c.group_id,
                            workspace_id: Some(c.workspace_id),
                            soft_group_id: c.soft_group_id,
                            split_from: c.split_from,
                            kind: c.kind,
                            path: c.path,
                            root: c.root,
                            explorer: c.explorer,
                            url: c.url,
                            sidebar: c.sidebar,
                            sidebar_top: c.sidebar_top,
                            zoom: c.zoom,
                            ..Default::default()
                        },
                    );
                    if let Some(card) = self.card_mut(&id) {
                        card.title = c.title;
                        card.z = c.z;
                        // Zero from a file before numbers; given below,
                        // above every number the file does have.
                        card.number = c.number;
                        card.protected = c.protected;
                        // The session this card's shell was in. The ui checks
                        // whether the backend still has it: if so the card is
                        // adopted, and if not it spawns as any card does.
                        card.session = c.session;
                        // Restored so an adopted card's program is still
                        // known to speak the kitty keyboard protocol: it
                        // announced that once, at a startup that has long
                        // fallen out of the ring we replay.
                        card.kitty_keys = c.kitty_keys;
                        card.agent_session = c.agent_session.clone();
                        card.agent_kind = c.agent_kind.clone();
                        card.osc_title = c.agent_title.clone();
                        card.tabs = c.tabs;
                        card.active_tab = c.active_tab;
                    }
                }
                // Cards the file gave no number (before the field existed)
                // get the lowest free ones, in file order.
                for i in 0..self.cards.len() {
                    if self.cards[i].number == 0 {
                        self.cards[i].number = self.take_number();
                    }
                }
                self.viewport = saved.viewport;
                self.ui_scale = saved.ui_scale;
                self.selection.focused_id = saved.focused_id;
            }
        }
        // Always, including on a first run: every card needs a canvas to be
        // on, and a file written before workspaces existed has none.
        self.ensure_workspace();
        self.loaded = true;
        self.first_run = text.is_none();
        self.dirty_layout = false;
    }

    /// A viewer takes the host's canvas (#423, `ift join`): the same JSON the
    /// host saves, applied IN PLACE. Cards keep their ids when nothing about
    /// them changed, because the ui keeps a body (a terminal's grid) per card
    /// id; rebuilding the cards on every snapshot would drop every grid.
    ///
    /// What is the host's follows the snapshot: which cards, groups and
    /// workspaces exist, and their rects, titles, groups and sessions. What is
    /// this viewer's stays: its viewport and zoom, its focus, its active
    /// workspace, and each workspace's own viewport and focused card. A viewer
    /// never saves (`read_only`), so nothing here can write the host's canvas
    /// into a data dir of the viewer's.
    ///
    /// Refused, with a sentence, when the host's build is newer than this one.
    pub fn apply_snapshot(&mut self, text: &str) -> Result<(), String> {
        if is_newer_than_this_build(text) {
            return Err(
                "the host's infiniterm is newer than this one; update infiniterm on this Mac"
                    .into(),
            );
        }
        let saved =
            parse_layout(text).ok_or_else(|| "the host's canvas could not be read".to_string())?;
        self.read_only = true;
        self.loaded = true;
        // The first canvas a viewer ever gets also sets where it looks; after
        // that the viewport, zoom, focus and workspace are the viewer's own.
        let first = !self.snapshot_seen;
        self.snapshot_seen = true;
        let (host_viewport, host_scale, host_focus, host_active) = (
            saved.viewport,
            saved.ui_scale,
            saved.focused_id.clone(),
            saved.active_workspace_id.clone(),
        );
        // Every kind is a card here, so a slot (an editor on the host, say)
        // moves, closes and groups like any card and goes back up whole; the
        // ui builds no body for it (no path on this Mac is read).
        let cards = saved.cards;

        let mine = std::mem::take(&mut self.workspaces);
        self.workspaces = saved
            .workspaces
            .into_iter()
            .map(|mut w| {
                if let Some(own) = mine.iter().find(|o| o.id == w.id) {
                    w.viewport = own.viewport;
                    w.focused = own.focused.clone();
                }
                w
            })
            .collect();
        self.groups = saved
            .groups
            .into_iter()
            .map(|g| super::Group {
                id: g.id,
                name: g.name,
            })
            .collect();

        let host_ids: Vec<String> = cards.iter().map(|c| c.id.clone()).collect();
        let gone: Vec<String> = self
            .cards
            .iter()
            .filter(|c| !host_ids.contains(&c.id))
            .map(|c| c.id.clone())
            .collect();
        for id in gone {
            self.remove_card(&id);
        }
        for c in cards {
            if let Some(card) = self.card_mut(&c.id) {
                card.workspace_id = c.workspace_id;
                card.rect = c.rect;
                card.z = c.z;
                card.title = c.title;
                card.cwd = c.cwd;
                card.group_id = c.group_id;
                card.soft_group_id = c.soft_group_id;
                card.split_from = c.split_from;
                card.number = c.number;
                card.protected = c.protected;
                card.session = c.session;
                card.agent_session = c.agent_session;
                card.agent_kind = c.agent_kind;
                card.osc_title = c.agent_title;
                continue;
            }
            let id = self.add_card(
                &c.cwd,
                NewCard {
                    id: Some(c.id),
                    rect: Some(c.rect),
                    group_id: c.group_id,
                    workspace_id: Some(c.workspace_id),
                    soft_group_id: c.soft_group_id,
                    split_from: c.split_from,
                    kind: c.kind,
                    path: c.path,
                    root: c.root,
                    explorer: c.explorer,
                    url: c.url,
                    sidebar: c.sidebar,
                    sidebar_top: c.sidebar_top,
                    zoom: c.zoom,
                    ..Default::default()
                },
            );
            if let Some(card) = self.card_mut(&id) {
                card.title = c.title;
                card.z = c.z;
                card.number = c.number;
                card.protected = c.protected;
                card.session = c.session;
                card.kitty_keys = c.kitty_keys;
                card.agent_session = c.agent_session;
                card.agent_kind = c.agent_kind;
                card.osc_title = c.agent_title;
                card.tabs = c.tabs;
                card.active_tab = c.active_tab;
            }
        }
        if first {
            self.active_workspace = host_active;
            self.viewport = host_viewport;
            self.ui_scale = host_scale;
            self.selection.focused_id = host_focus;
        }
        // The viewer's own focus survives unless its card went.
        if let Some(id) = self.selection.focused_id.clone() {
            if self.card(&id).is_none() {
                self.selection.focused_id = None;
            }
        }
        self.ensure_workspace();
        self.dirty_layout = false;
        self.layout_undo.clear();
        Ok(())
    }

    /// The host's live agent states, for a viewer (#423): `{"<card id>":
    /// "working", ...}`. The layout leaves them out on purpose, so they come
    /// on their own. A card the host did not mention has no state; a state
    /// word this build does not know is no state either.
    pub fn apply_agent_states(&mut self, text: &str) {
        let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, String>>(text)
        else {
            return;
        };
        for card in &mut self.cards {
            card.agent = map
                .get(&card.id)
                .map_or(crate::agent_state::AgentState::None, |n| {
                    crate::agent_state::AgentState::from_name(n)
                });
        }
    }

    /// The states a viewer is sent, as `apply_agent_states` reads them. Only
    /// cards with a state, so a quiet canvas is `{}`.
    pub fn agent_states_text(&self) -> String {
        let map: std::collections::BTreeMap<&str, &str> = self
            .cards
            .iter()
            .filter(|c| c.agent != crate::agent_state::AgentState::None)
            .map(|c| (c.id.as_str(), c.agent.name()))
            .collect();
        serde_json::to_string(&map).unwrap_or_else(|_| "{}".into())
    }

    /// The file's text, or nothing while saving is not allowed.
    pub fn save_text(&mut self) -> Option<String> {
        if !self.loaded || self.read_only {
            return None;
        }
        Some(self.layout_snapshot())
    }

    /// The canvas as the layout text, whatever the model may save. A viewer
    /// (`read_only`, so it never writes a file) sends this up to its host.
    pub fn layout_snapshot(&mut self) -> String {
        self.sync_workspace_viewport();
        self.dirty_layout = false;
        let cards: Vec<SavedCard> = self
            .cards
            .iter()
            // An in-place editor lives as long as the `ift` waiting on it,
            // which a relaunch ends (cover.rs).
            .filter(|c| !self.covers.contains_key(&c.id))
            .map(|c| SavedCard {
                id: c.id.clone(),
                workspace_id: c.workspace_id.clone(),
                rect: c.rect,
                z: c.z,
                title: c.title.clone(),
                cwd: c.cwd.clone(),
                group_id: c.group_id.clone(),
                soft_group_id: c.soft_group_id.clone(),
                split_from: c.split_from.clone(),
                kind: c.kind,
                path: c.path.clone(),
                root: c.root.clone(),
                explorer: c.explorer,
                url: c.url.clone(),
                sidebar: c.sidebar,
                sidebar_top: c.sidebar_top,
                zoom: c.zoom,
                tabs: c.tabs.clone(),
                active_tab: c.active_tab,
                session: c.session.clone(),
                kitty_keys: c.kitty_keys,
                agent_session: c.agent_session.clone(),
                agent_kind: c.agent_kind.clone(),
                agent_title: c.osc_title.clone().filter(|_| c.runs_agent()),
                number: c.number,
                protected: c.protected,
            })
            .collect();
        let groups: Vec<SavedGroup> = self
            .groups
            .iter()
            .map(|g| SavedGroup {
                id: g.id.clone(),
                name: g.name.clone(),
            })
            .collect();
        let usage: &Usage = &self.usage;
        // A cover is not saved, so the focus it holds is saved as the
        // terminal under it.
        let focused = self
            .selection
            .focused_id
            .as_deref()
            .map(|f| self.covers.get(f).map(String::as_str).unwrap_or(f));
        let mut workspaces = self.workspaces.clone();
        for w in &mut workspaces {
            if let Some(base) = w.focused.as_ref().and_then(|f| self.covers.get(f)) {
                w.focused = Some(base.clone());
            }
        }
        layout_text(&serialise_layout(
            &cards,
            &groups,
            &self.viewport,
            focused,
            self.ui_scale,
            usage,
            &workspaces,
            self.active_workspace.as_deref(),
        ))
    }

    /// Records a canvas the host is about to publish and returns its revision
    /// (#423). The host keeps the last few, so it can tell what a viewer
    /// changed: the viewer names the revision it edited, and its edit is
    /// compared with that canvas.
    pub fn record_published(&mut self, text: &str) -> u64 {
        self.layout_rev += 1;
        if let Some(layout) = parse_layout(text) {
            self.published.push_back((self.layout_rev, layout));
            while self.published.len() > PUBLISHED_KEPT {
                self.published.pop_front();
            }
        }
        self.layout_rev
    }

    /// A viewer's edit, merged into this canvas (#423, `ift join`). The viewer
    /// edited its own copy with the app's usual commands and sent the result
    /// with the revision it started from. What it CHANGED is what is applied:
    /// the edit is compared with the canvas the host published at that
    /// revision, and only the differences are made, on top of the host's
    /// CURRENT state, through the host's own paths:
    ///
    /// - a card the viewer closed is closed by the host's normal close (a
    ///   terminal parks and ends as it always does);
    /// - a terminal the viewer made becomes a normal new card (the host's
    ///   reconcile starts its shell, here, with this Mac's login);
    /// - a field the viewer changed (rect, title, group, workspace, ...) is set;
    ///   the same for groups and workspaces (made, removed, renamed, ordered).
    ///
    /// A field the viewer did not touch is left as the host has it, so a busy
    /// host (agents renaming cards, a user moving things) is never reverted by
    /// a viewer's unrelated edit, and an edit is never refused for arriving
    /// while the host has an unpublished change. Where both changed the same
    /// field, the viewer's, the later writer, wins.
    ///
    /// What stays this host's: its viewport, zoom, focus, active workspace,
    /// each workspace's own viewport, and every card's session, agent state and
    /// number (a number the viewer picked can collide with a host's). The only
    /// edit refused is one made on a revision the host no longer remembers:
    /// `Stale`, and the app republishes its canvas so the viewer resyncs.
    pub fn apply_remote_layout(&mut self, text: &str, base: u64) -> Result<RemoteApply, String> {
        let Some(was) = self
            .published
            .iter()
            .find(|(rev, _)| *rev == base)
            .map(|(_, layout)| layout.clone())
        else {
            return Ok(RemoteApply::Stale);
        };
        let now = parse_layout(text)
            .ok_or_else(|| "the viewer's canvas could not be read".to_string())?;

        let was_card = |id: &str| was.cards.iter().find(|c| c.id == id);

        // Closed in the viewer, closed here. An in-place editor (a cover) is
        // the host's own and lives as long as its `ift`.
        let closed: Vec<String> = was
            .cards
            .iter()
            .filter(|c| !now.cards.iter().any(|n| n.id == c.id))
            .map(|c| c.id.clone())
            .filter(|id| self.card(id).is_some() && !self.covers.contains_key(id))
            .collect();
        for id in closed {
            self.close_card(&id, false);
        }

        for c in &now.cards {
            match was_card(&c.id) {
                // A card the viewer had and still has: only what it changed.
                Some(b) => {
                    if let Some(card) = self.card_mut(&c.id) {
                        if c.rect != b.rect {
                            card.rect = c.rect;
                        }
                        if c.z != b.z {
                            card.z = c.z;
                        }
                        if c.title != b.title {
                            card.title = c.title.clone();
                        }
                        if c.workspace_id != b.workspace_id {
                            card.workspace_id = c.workspace_id.clone();
                        }
                        if c.group_id != b.group_id {
                            card.group_id = c.group_id.clone();
                        }
                        if c.soft_group_id != b.soft_group_id {
                            card.soft_group_id = c.soft_group_id.clone();
                        }
                        if c.split_from != b.split_from {
                            card.split_from = c.split_from.clone();
                        }
                        if c.protected != b.protected {
                            card.protected = c.protected;
                        }
                    }
                }
                // A card the viewer made. Only a terminal can be made from a
                // viewer: an editor, diff or browser card would name a file or
                // page on the viewer's Mac.
                None => {
                    if self.card(&c.id).is_some()
                        || c.kind != crate::saved_layout::CardKind::Terminal
                    {
                        continue;
                    }
                    // `~` and an empty folder are this host's own start
                    // directory; a folder the viewer named is tried as it is.
                    let cwd = if c.cwd.is_empty() || c.cwd.starts_with('~') {
                        self.start_dir.clone()
                    } else {
                        c.cwd.clone()
                    };
                    let id = self.add_card(
                        &cwd,
                        NewCard {
                            id: Some(c.id.clone()),
                            rect: Some(c.rect),
                            group_id: c.group_id.clone(),
                            workspace_id: Some(c.workspace_id.clone()),
                            soft_group_id: c.soft_group_id.clone(),
                            split_from: c.split_from.clone(),
                            kind: c.kind,
                            ..Default::default()
                        },
                    );
                    let number = self.take_number();
                    if let Some(card) = self.card_mut(&id) {
                        card.title = c.title.clone();
                        card.z = c.z;
                        card.protected = c.protected;
                        card.number = number;
                    }
                }
            }
        }

        // Groups: made, removed, renamed.
        for g in &was.groups {
            if !now.groups.iter().any(|n| n.id == g.id) {
                self.groups.retain(|h| h.id != g.id);
            }
        }
        for g in &now.groups {
            match was.groups.iter().find(|b| b.id == g.id) {
                Some(b) if b.name != g.name => {
                    if let Some(h) = self.groups.iter_mut().find(|h| h.id == g.id) {
                        h.name = g.name.clone();
                    }
                }
                Some(_) => {}
                None => {
                    if !self.groups.iter().any(|h| h.id == g.id) {
                        self.groups.push(super::Group {
                            id: g.id.clone(),
                            name: g.name.clone(),
                        });
                    }
                }
            }
        }

        // Workspaces: made, removed (unless a card of the host's is still in
        // it), renamed, and put in the order the viewer put them.
        for w in &was.workspaces {
            if !now.workspaces.iter().any(|n| n.id == w.id) {
                let occupied = self.cards.iter().any(|c| c.workspace_id == w.id);
                if !occupied {
                    self.workspaces.retain(|h| h.id != w.id);
                }
            }
        }
        for w in &now.workspaces {
            match was.workspaces.iter().find(|b| b.id == w.id) {
                Some(b) if b.name != w.name => {
                    if let Some(h) = self.workspaces.iter_mut().find(|h| h.id == w.id) {
                        h.name = w.name.clone();
                    }
                }
                Some(_) => {}
                None => {
                    if !self.workspaces.iter().any(|h| h.id == w.id) {
                        self.workspaces.push(w.clone());
                    }
                }
            }
        }
        let order_of = |ws: &[crate::workspaces::Workspace],
                        keep: &[crate::workspaces::Workspace]|
         -> Vec<String> {
            ws.iter()
                .filter(|w| keep.iter().any(|k| k.id == w.id))
                .map(|w| w.id.clone())
                .collect()
        };
        if order_of(&was.workspaces, &now.workspaces) != order_of(&now.workspaces, &was.workspaces)
        {
            let want: Vec<String> = now.workspaces.iter().map(|w| w.id.clone()).collect();
            let mut mine = std::mem::take(&mut self.workspaces);
            let mut ordered = Vec::with_capacity(mine.len());
            for id in &want {
                if let Some(at) = mine.iter().position(|w| &w.id == id) {
                    ordered.push(mine.remove(at));
                }
            }
            ordered.extend(mine);
            self.workspaces = ordered;
        }

        self.ensure_workspace();
        // Changed: the app publishes the new canvas with a new revision, to
        // every viewer, the sender included.
        self.dirty_layout = true;
        Ok(RemoteApply::Applied)
    }

    /// Opens the first card, once, when the layout has been read, the canvas
    /// measured and the home directory known. Without the once-flag this
    /// re-fired the moment the canvas emptied, so closing your last card
    /// silently opened a replacement.
    pub fn seed_first_card(&mut self, seeded: &mut bool) {
        if *seeded
            || !self.loaded
            || !self.cards.is_empty()
            || self.view_size.w <= 0.
            || self.home.is_empty()
        {
            return;
        }
        *seeded = true;
        // A first launch opens the "Start here" card ALONE, centred: the newcomer
        // makes the first terminal with Cmd+T and comes back with
        // Cmd+Alt+Left, which teaches the canvas by using it (Ekin,
        // 2026-10-04). A terminal beside it took the focus and the eye, and
        // the card went unread.
        if self.first_run {
            // Centred and framed from the first frame: set, not animated, so
            // a first launch does not open with the canvas sliding. The pan
            // `open_in_card` asked for is dropped, or it would glide off the
            // centre. Framing on, so Cmd+T and the arrows frame the next card
            // the same way.
            if let Some(rect) = self
                .open_welcome(None)
                .and_then(|id| self.card(&id))
                .map(|c| c.rect)
            {
                self.effects.retain(|e| {
                    !matches!(
                        e,
                        super::Effect::AnimatePan { .. } | super::Effect::AnimateFit(_)
                    )
                });
                self.viewport = self.fit_viewport(rect);
                self.framing = true;
            }
        } else {
            let start = self.start_dir.clone();
            let ws = self.active_workspace.clone();
            let id = self.add_card(
                &start,
                NewCard {
                    workspace_id: ws,
                    ..Default::default()
                },
            );
            self.set_focus(Some(&id));
        }
        // The seed is the canvas's floor, not something to undo.
        self.layout_undo.clear();
    }

    /// Opens the in-app docs (`help_docs.rs`, written by the ui at launch):
    /// the first page, with the folder's tree beside it as the contents.
    /// Focuses the docs card instead when one is already open.
    pub fn open_docs(&mut self, after: Option<&str>) -> Option<String> {
        let dir = crate::help_docs::docs_dir().to_string_lossy().into_owned();
        let existing = self
            .here()
            .into_iter()
            .find(|c| {
                c.kind == crate::saved_layout::CardKind::Editor
                    && c.root.as_deref() == Some(dir.as_str())
            })
            .map(|c| c.id.clone());
        existing.or_else(|| {
            let first = crate::help_docs::docs_dir().join(crate::help_docs::FIRST_PAGE);
            self.open_in_card(
                crate::ift::OpenPlan::Editor {
                    cwd: dir.clone(),
                    path: Some(first.to_string_lossy().into_owned()),
                    root: Some(dir.clone()),
                    line: None,
                },
                after,
            )
        })
    }

    /// Opens the welcome card (`welcome::welcome_path`, written by the ui at
    /// launch) beside `after`, or focuses it when one is already open.
    pub fn open_welcome(&mut self, after: Option<&str>) -> Option<String> {
        self.open_page(crate::welcome::welcome_path(), after)
    }

    /// Opens the changelog card (`changelog::changelog_path`, written by the
    /// ui at launch) beside `after`, or focuses it when one is already open.
    pub fn open_changelog(&mut self, after: Option<&str>) -> Option<String> {
        self.open_page(crate::changelog::changelog_path(), after)
    }

    /// A Page card on `path` beside `after`, or the one already open on it.
    fn open_page(&mut self, path: std::path::PathBuf, after: Option<&str>) -> Option<String> {
        let path = path.to_string_lossy().into_owned();
        let existing = self
            .here()
            .into_iter()
            .find(|c| {
                c.kind == crate::saved_layout::CardKind::Page
                    && c.path.as_deref() == Some(path.as_str())
            })
            .map(|c| c.id.clone());
        existing.or_else(|| {
            // A Page, not a read-only editor: it never takes the keyboard,
            // which a newcomer clicking it needs, while the docs keep an
            // editor's keys (#41, #42).
            let cwd = self.start_dir.clone();
            self.open_in_card(crate::ift::OpenPlan::Page { cwd, path }, after)
        })
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::grid::{Rect, Size};

    fn host_model() -> Model {
        let mut m = Model::new();
        m.view_size = Size { w: 1600., h: 1000. };
        m.home = "/Users/me".into();
        m.start_dir = "/Users/me".into();
        m.load_layout(None);
        m.first_run = false;
        m
    }

    fn add(m: &mut Model, rect: Rect) -> String {
        let ws = m.active_workspace.clone();
        m.add_card(
            "/Users/me",
            NewCard {
                rect: Some(rect),
                workspace_id: ws,
                ..Default::default()
            },
        )
    }

    fn rect(x: f64, y: f64) -> Rect {
        Rect {
            x,
            y,
            w: 800.,
            h: 600.,
        }
    }

    #[test]
    fn the_first_snapshot_makes_the_hosts_cards_and_never_saves() {
        let mut host = host_model();
        let a = add(&mut host, rect(0., 0.));
        let b = add(&mut host, rect(900., 0.));
        let text = host.save_text().unwrap();

        let mut viewer = Model::new();
        viewer.apply_snapshot(&text).unwrap();
        let ids: Vec<&str> = viewer.cards.iter().map(|c| c.id.as_str()).collect();
        assert!(ids.contains(&a.as_str()) && ids.contains(&b.as_str()));
        assert!(viewer.read_only);
        assert!(
            viewer.save_text().is_none(),
            "a viewer never writes a canvas"
        );
    }

    #[test]
    fn a_later_snapshot_keeps_ids_and_follows_the_host() {
        let mut host = host_model();
        let a = add(&mut host, rect(0., 0.));
        let b = add(&mut host, rect(900., 0.));
        let mut viewer = Model::new();
        viewer.apply_snapshot(&host.save_text().unwrap()).unwrap();

        // The host moves a, closes b and opens c.
        host.card_mut(&a).unwrap().rect = rect(50., 60.);
        host.card_mut(&a).unwrap().title = "renamed".into();
        host.remove_card(&b);
        let c = add(&mut host, rect(900., 700.));
        viewer.apply_snapshot(&host.save_text().unwrap()).unwrap();

        assert_eq!(viewer.card(&a).unwrap().rect, rect(50., 60.));
        assert_eq!(viewer.card(&a).unwrap().title, "renamed");
        assert!(viewer.card(&b).is_none(), "a card the host closed goes");
        assert!(viewer.card(&c).is_some(), "a card the host opened comes");
    }

    #[test]
    fn the_viewer_keeps_its_own_viewport_and_focus() {
        let mut host = host_model();
        let a = add(&mut host, rect(0., 0.));
        let b = add(&mut host, rect(900., 0.));
        let mut viewer = Model::new();
        viewer.apply_snapshot(&host.save_text().unwrap()).unwrap();
        viewer.selection.focused_id = Some(b.clone());
        let own = viewer.viewport;
        viewer.workspaces[0].viewport.x = 123.;

        host.viewport.x = 5000.;
        host.card_mut(&a).unwrap().rect = rect(10., 10.);
        viewer.apply_snapshot(&host.save_text().unwrap()).unwrap();

        assert_eq!(
            viewer.viewport, own,
            "the host's pan does not move the viewer"
        );
        assert_eq!(viewer.workspaces[0].viewport.x, 123.);
        assert_eq!(viewer.selection.focused_id, Some(b));
    }

    #[test]
    fn the_hosts_agent_colours_reach_the_viewer_and_unknown_words_are_no_state() {
        use crate::agent_state::AgentState;
        let mut host = host_model();
        let a = add(&mut host, rect(0., 0.));
        let b = add(&mut host, rect(900., 0.));
        host.card_mut(&a).unwrap().agent = AgentState::Waiting;
        host.card_mut(&b).unwrap().agent = AgentState::Done;
        let mut viewer = Model::new();
        viewer.apply_snapshot(&host.save_text().unwrap()).unwrap();

        viewer.apply_agent_states(&host.agent_states_text());
        assert_eq!(viewer.card(&a).unwrap().agent, AgentState::Waiting);
        assert_eq!(viewer.card(&b).unwrap().agent, AgentState::Done);

        // A newer host's new word, and a card it stopped mentioning.
        viewer.apply_agent_states(&format!("{{\"{a}\":\"brand-new\"}}"));
        assert_eq!(viewer.card(&a).unwrap().agent, AgentState::None);
        assert_eq!(viewer.card(&b).unwrap().agent, AgentState::None);
        // Garbage changes nothing.
        viewer.card_mut(&a).unwrap().agent = AgentState::Working;
        viewer.apply_agent_states("not json");
        assert_eq!(viewer.card(&a).unwrap().agent, AgentState::Working);
    }

    fn editor_card(m: &mut Model, rect: Rect) -> String {
        use crate::saved_layout::CardKind;
        let ws = m.active_workspace.clone();
        m.add_card(
            "/Users/me",
            NewCard {
                rect: Some(rect),
                workspace_id: ws,
                kind: CardKind::Editor,
                path: Some("/Users/me/notes.rs".into()),
                ..Default::default()
            },
        )
    }

    /// A host with two terminals and an editor, and a viewer that has applied
    /// its canvas. Returns (host, viewer, term a, term b, editor).
    fn host_and_viewer() -> (Model, Model, String, String, String) {
        let mut host = host_model();
        let a = add(&mut host, rect(0., 0.));
        let b = add(&mut host, rect(900., 0.));
        let e = editor_card(&mut host, rect(0., 700.));
        let mut viewer = Model::new();
        let text = host.save_text().unwrap();
        // The host publishes it, and the viewer applies it: its edits name
        // this revision.
        let rev = host.record_published(&text);
        viewer.apply_snapshot(&text).unwrap();
        viewer.layout_rev = rev;
        (host, viewer, a, b, e)
    }

    #[test]
    fn a_card_that_is_not_a_terminal_is_a_real_card_in_a_viewer_and_goes_back_up_whole() {
        use crate::saved_layout::CardKind;
        let (mut host, mut viewer, _a, _b, e) = host_and_viewer();
        // It is a card here: it moves like one.
        let card = viewer
            .card(&e)
            .expect("an editor slot is a card in a viewer");
        assert_eq!(card.kind, CardKind::Editor);
        viewer.card_mut(&e).unwrap().rect = rect(50., 760.);
        // And it goes up with its path intact; the host moves it and keeps it.
        let up = viewer.layout_snapshot();
        assert_eq!(
            host.apply_remote_layout(&up, viewer.layout_rev),
            Ok(RemoteApply::Applied)
        );
        let on_host = host.card(&e).expect("the editor is still on the host");
        assert_eq!(on_host.rect, rect(50., 760.));
        assert_eq!(on_host.path.as_deref(), Some("/Users/me/notes.rs"));
    }

    #[test]
    fn closed_in_the_viewer_is_closed_on_the_host() {
        let (mut host, mut viewer, a, b, e) = host_and_viewer();
        viewer.close_card(&b, false);
        viewer.close_card(&e, false);
        let up = viewer.layout_snapshot();
        assert_eq!(
            host.apply_remote_layout(&up, viewer.layout_rev),
            Ok(RemoteApply::Applied)
        );
        assert!(host.card(&a).is_some());
        assert!(
            host.card(&b).is_none(),
            "the host closed it by its own close"
        );
        assert!(host.card(&e).is_none(), "a slot closes too");
    }

    #[test]
    fn made_in_the_viewer_is_a_new_terminal_on_the_host_and_never_an_editor() {
        use crate::saved_layout::CardKind;
        let (mut host, mut viewer, _a, _b, _e) = host_and_viewer();
        let made = add(&mut viewer, rect(1800., 0.));
        // A viewer cannot name a file on the host: an editor it makes is dropped.
        let ws = viewer.active_workspace.clone();
        let phantom = viewer.add_card(
            "/Users/me",
            NewCard {
                rect: Some(rect(1800., 700.)),
                workspace_id: ws,
                kind: CardKind::Editor,
                path: Some("/Users/viewer/only.rs".into()),
                ..Default::default()
            },
        );
        let up = viewer.layout_snapshot();
        assert_eq!(
            host.apply_remote_layout(&up, viewer.layout_rev),
            Ok(RemoteApply::Applied)
        );
        let new = host.card(&made).expect("a normal new card on the host");
        assert_eq!(new.kind, CardKind::Terminal);
        assert_eq!(new.rect, rect(1800., 0.));
        assert!(
            new.session.is_none(),
            "the host's reconcile starts its shell"
        );
        assert!(host.card(&phantom).is_none());
        // Its number is the host's: nobody else on the host has it.
        let same = host.cards.iter().filter(|c| c.number == new.number).count();
        assert_eq!(same, 1, "number {} is unique on the host", new.number);
    }

    #[test]
    fn moved_renamed_and_grouped_in_the_viewer_is_so_on_the_host() {
        let (mut host, mut viewer, a, b, _e) = host_and_viewer();
        viewer.card_mut(&a).unwrap().rect = rect(25., 40.);
        viewer.card_mut(&a).unwrap().title = "build".into();
        viewer.groups.push(crate::model::Group {
            id: "g1".into(),
            name: "pair".into(),
        });
        viewer.card_mut(&a).unwrap().group_id = Some("g1".into());
        viewer.card_mut(&b).unwrap().group_id = Some("g1".into());
        let up = viewer.layout_snapshot();
        assert_eq!(
            host.apply_remote_layout(&up, viewer.layout_rev),
            Ok(RemoteApply::Applied)
        );
        assert_eq!(host.card(&a).unwrap().rect, rect(25., 40.));
        assert_eq!(host.card(&a).unwrap().title, "build");
        assert_eq!(host.card(&a).unwrap().group_id.as_deref(), Some("g1"));
        assert_eq!(host.card(&b).unwrap().group_id.as_deref(), Some("g1"));
        assert!(host.groups.iter().any(|g| g.id == "g1"));
    }

    #[test]
    fn a_busy_host_is_never_reverted_by_a_viewers_unrelated_edit() {
        let (mut host, mut viewer, a, b, _e) = host_and_viewer();
        // The host changes things the viewer never saw and has not published:
        // an agent renames a card, the user moves another.
        host.card_mut(&a).unwrap().title = "agent renamed me".into();
        host.card_mut(&b).unwrap().rect = rect(333., 444.);
        host.dirty_layout = true;
        // Meanwhile the viewer only moves a third card.
        let e = viewer
            .cards
            .iter()
            .find(|c| c.kind != crate::saved_layout::CardKind::Terminal)
            .map(|c| c.id.clone());
        let moved = e.unwrap_or_else(|| a.clone());
        viewer.card_mut(&moved).unwrap().rect = rect(70., 80.);
        let up = viewer.layout_snapshot();
        assert_eq!(
            host.apply_remote_layout(&up, viewer.layout_rev),
            Ok(RemoteApply::Applied),
            "an edit is never refused for arriving over a host change"
        );
        assert_eq!(
            host.card(&moved).unwrap().rect,
            rect(70., 80.),
            "the edit landed"
        );
        assert_eq!(
            host.card(&b).unwrap().rect,
            rect(333., 444.),
            "the host's move stands"
        );
        if moved != a {
            assert_eq!(
                host.card(&a).unwrap().title,
                "agent renamed me",
                "so does its rename"
            );
        }
    }

    #[test]
    fn where_both_changed_the_same_field_the_viewer_wins_and_the_rest_is_the_hosts() {
        let (mut host, mut viewer, a, _b, _e) = host_and_viewer();
        host.card_mut(&a).unwrap().rect = rect(1., 1.);
        host.card_mut(&a).unwrap().title = "the host's title".into();
        viewer.card_mut(&a).unwrap().rect = rect(2., 2.);
        let up = viewer.layout_snapshot();
        host.apply_remote_layout(&up, viewer.layout_rev).unwrap();
        assert_eq!(
            host.card(&a).unwrap().rect,
            rect(2., 2.),
            "the later writer wins a field"
        );
        assert_eq!(
            host.card(&a).unwrap().title,
            "the host's title",
            "a field it did not touch stays"
        );
    }

    #[test]
    fn an_edit_made_on_a_canvas_the_host_no_longer_remembers_is_resynced() {
        let (mut host, mut viewer, a, b, _e) = host_and_viewer();
        viewer.close_card(&b, false);
        let up = viewer.layout_snapshot();
        assert_eq!(host.apply_remote_layout(&up, 9999), Ok(RemoteApply::Stale));
        assert!(host.card(&b).is_some(), "nothing was closed");
        assert!(host.card(&a).is_some());
    }

    #[test]
    fn a_viewers_edit_leaves_the_hosts_own_view_alone() {
        let (mut host, mut viewer, a, b, _e) = host_and_viewer();
        host.viewport.x = 321.;
        host.selection.focused_id = Some(b.clone());
        viewer.viewport.x = 999.;
        viewer.selection.focused_id = Some(a.clone());
        viewer.card_mut(&a).unwrap().rect = rect(5., 5.);
        let up = viewer.layout_snapshot();
        host.apply_remote_layout(&up, viewer.layout_rev).unwrap();
        assert_eq!(host.viewport.x, 321., "the viewer's pan is not the host's");
        assert_eq!(host.selection.focused_id, Some(b), "nor its focus");
    }

    #[test]
    fn an_applied_edit_makes_the_host_publish_and_a_viewer_never_saves() {
        let (mut host, mut viewer, a, _b, _e) = host_and_viewer();
        viewer.card_mut(&a).unwrap().rect = rect(1., 1.);
        let up = viewer.layout_snapshot();
        assert!(!host.dirty_layout);
        host.apply_remote_layout(&up, viewer.layout_rev).unwrap();
        assert!(
            host.dirty_layout,
            "the host's next save is the new revision"
        );
        assert!(
            viewer.save_text().is_none(),
            "a viewer writes nothing of its own"
        );
    }

    #[test]
    fn a_host_with_no_terminals_still_sets_the_first_view_once() {
        use crate::saved_layout::CardKind;
        let mut host = host_model();
        let ws = host.active_workspace.clone();
        host.add_card(
            "/Users/me",
            NewCard {
                rect: Some(rect(0., 0.)),
                workspace_id: ws,
                kind: CardKind::Page,
                path: Some("/Users/me/x.md".into()),
                ..Default::default()
            },
        );
        host.viewport.x = 700.;
        let mut viewer = Model::new();
        viewer.apply_snapshot(&host.save_text().unwrap()).unwrap();
        assert_eq!(
            viewer.viewport.x, 700.,
            "the first snapshot sets where the viewer looks"
        );
        viewer.viewport.x = 5.;
        host.viewport.x = 900.;
        viewer.apply_snapshot(&host.save_text().unwrap()).unwrap();
        assert_eq!(
            viewer.viewport.x, 5.,
            "after that the view is the viewer's own"
        );
    }

    #[test]
    fn a_snapshot_from_a_newer_build_is_refused_with_a_sentence() {
        let mut viewer = Model::new();
        let err = viewer
            .apply_snapshot("{\"version\": 999999, \"cards\": []}")
            .unwrap_err();
        assert!(err.contains("newer than this one"), "{err}");
    }
}

#[cfg(test)]
mod agent_states_text_tests {
    use super::*;
    use crate::agent_state::AgentState;
    use crate::grid::Size;

    #[test]
    fn only_cards_with_a_state_are_listed_by_name() {
        let mut m = Model::new();
        m.view_size = Size { w: 1600., h: 1000. };
        m.load_layout(None);
        let ws = m.active_workspace.clone();
        let quiet = m.add_card(
            "/",
            NewCard {
                workspace_id: ws.clone(),
                ..Default::default()
            },
        );
        let busy = m.add_card(
            "/",
            NewCard {
                workspace_id: ws.clone(),
                ..Default::default()
            },
        );
        let asking = m.add_card(
            "/",
            NewCard {
                workspace_id: ws,
                ..Default::default()
            },
        );
        m.card_mut(&busy).unwrap().agent = AgentState::Working;
        m.card_mut(&asking).unwrap().agent = AgentState::Waiting;
        let v: serde_json::Value = serde_json::from_str(&m.agent_states_text()).unwrap();
        assert_eq!(v[&busy], "working");
        assert_eq!(v[&asking], "waiting");
        assert!(
            v.get(&quiet).is_none(),
            "a card with no state is not listed"
        );
    }

    #[test]
    fn a_canvas_with_no_states_is_an_empty_object() {
        let mut m = Model::new();
        m.load_layout(None);
        assert_eq!(m.agent_states_text(), "{}");
    }
}
