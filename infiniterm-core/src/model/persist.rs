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
        self.dirty_layout = false;
    }

    /// The file's text, or nothing while saving is not allowed.
    pub fn save_text(&mut self) -> Option<String> {
        if !self.loaded || self.read_only {
            return None;
        }
        self.sync_workspace_viewport();
        self.dirty_layout = false;
        let cards: Vec<SavedCard> = self
            .cards
            .iter()
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
        Some(layout_text(&serialise_layout(
            &cards,
            &groups,
            &self.viewport,
            self.selection.focused_id.as_deref(),
            self.ui_scale,
            usage,
            &self.workspaces,
            self.active_workspace.as_deref(),
        )))
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
        // The seed is the canvas's floor, not something to undo.
        self.layout_undo.clear();
    }
}
