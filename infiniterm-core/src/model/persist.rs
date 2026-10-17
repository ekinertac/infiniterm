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
        if !self.loaded {
            self.load_layout(Some(text));
            self.read_only = true;
            self.layout_undo.clear();
            return Ok(());
        }
        let saved = parse_layout(text).ok_or_else(|| "the host's canvas could not be read".to_string())?;
        self.read_only = true;

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

        let host_ids: Vec<String> = saved.cards.iter().map(|c| c.id.clone()).collect();
        let gone: Vec<String> = self
            .cards
            .iter()
            .filter(|c| !host_ids.contains(&c.id))
            .map(|c| c.id.clone())
            .collect();
        for id in gone {
            self.remove_card(&id);
        }
        for c in saved.cards {
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
        Some(layout_text(&serialise_layout(
            &cards,
            &groups,
            &self.viewport,
            focused,
            self.ui_scale,
            usage,
            &workspaces,
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
        Rect { x, y, w: 800., h: 600. }
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
        assert!(viewer.save_text().is_none(), "a viewer never writes a canvas");
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

        assert_eq!(viewer.viewport, own, "the host's pan does not move the viewer");
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

    #[test]
    fn a_snapshot_from_a_newer_build_is_refused_with_a_sentence() {
        let mut viewer = Model::new();
        let err = viewer
            .apply_snapshot("{\"version\": 999999, \"cards\": []}")
            .unwrap_err();
        assert!(err.contains("newer than this one"), "{err}");
    }
}

