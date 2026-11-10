//! Tab commands for a card that holds tabs: new, close, next/prev, jump
//! to N, reopen the last closed one. Plain `Card` field mutation, no
//! `Effect`: `browsers.rs::reconcile_browsers` and `editors.rs::
//! reconcile_editors` turn a changed `card.tabs` into ui action every
//! frame. A tab is a HANDLE: a browser card's is a url and its active one
//! is mirrored into `card.url`; an editor card's is a file path (empty for
//! an untitled buffer) mirrored into `card.path`, so everything downstream
//! that reads `url` or `path` keeps working without knowing about tabs.
//! The functions keep their `browser_tab_` names from the day they were
//! browser-only; they serve both kinds now (2026-09-20, editor tabs).
use super::{Card, Model};
use crate::commands::CommandRegistry;
use crate::saved_layout::CardKind;

/// What an EMPTY tab handle opens as: a browser's blank page; an editor's
/// untitled buffer is the empty string, since `card.path` is `None` then.
fn blank_handle(kind: CardKind) -> &'static str {
    match kind {
        CardKind::Browser => "about:blank",
        _ => "",
    }
}

/// The active tab's handle back into the field the rest of the app reads.
fn sync_handle(card: &mut Card) {
    let handle = card.tabs.get(card.active_tab).cloned();
    match card.kind {
        CardKind::Browser => card.url = handle,
        CardKind::Editor => card.path = handle.filter(|p| !p.is_empty()),
        _ => {}
    }
}

/// The handle a card without `tabs` is showing: its url or its path.
fn current_handle(card: &Card) -> String {
    match card.kind {
        CardKind::Browser => card.url.clone().unwrap_or_else(|| "about:blank".into()),
        _ => card.path.clone().unwrap_or_default(),
    }
}

/// `closed_tabs` is a ring of the last few, not a full history: the reopen
/// command only ever wants the most recent one back.
const CLOSED_TABS_CAP: usize = 10;

/// Shared by `browser_tab_close` and `browser_tab_close_at`: remembers a
/// closed tab's url for `browser.tab.reopenClosed`, capped so a session of
/// closing tabs all day does not grow the save file without bound.
fn remember_closed(card: &mut Card, url: String) {
    card.closed_tabs.push(url);
    if card.closed_tabs.len() > CLOSED_TABS_CAP {
        card.closed_tabs.remove(0);
    }
}

/// A card's tabs, normalised: `tabs` always has at least the active url in
/// it, so `browser.tab.new`/`close` need not special-case "no tabs yet"
/// (a single-tab card keeps `tabs` empty on disk, but in memory it always
/// has exactly the browser card's fields to fall back to).
fn ensure_tabs(m: &mut Model, id: &str) {
    let Some(card) = m.card_mut(id) else { return };
    if card.tabs.is_empty() {
        card.tabs = vec![current_handle(card)];
        card.active_tab = 0;
    }
}

/// A folder editor still on its first, blank buffer and nothing else
/// (`editor.folderScratchTab` decides whether that buffer shows as a tab).
pub fn is_bare_folder_editor(card: &Card) -> bool {
    card.kind == CardKind::Editor
        && card.root.is_some()
        && card.tabs.len() == 1
        && card.tabs[0].is_empty()
}

impl Model {
    /// Appends a new tab at `url` (`None` is `about:blank`, the same
    /// default an empty browser card opens with) and makes it active.
    pub fn browser_tab_open(&mut self, card_id: &str, url: Option<&str>) {
        ensure_tabs(self, card_id);
        let keep_scratch = self.config.editor.folder_scratch_tab;
        let Some(card) = self.card_mut(card_id) else {
            return;
        };
        // Without the scratch tab, a folder editor's blank buffer is not a
        // tab the user has: the first file takes its place.
        if !keep_scratch && url.is_some() && is_bare_folder_editor(card) {
            card.tabs[0] = url.unwrap_or_default().to_string();
            card.active_tab = 0;
            sync_handle(card);
            self.dirty_layout = true;
            return;
        }
        card.tabs
            .push(url.unwrap_or(blank_handle(card.kind)).to_string());
        card.active_tab = card.tabs.len() - 1;
        sync_handle(card);
        self.dirty_layout = true;
    }

    /// A blank tab from Cmd+T or the strip's "+": the address bar opens on a
    /// browser's, since a blank page with no way to type is a dead end. An
    /// editor's blank tab is a buffer to type into, so it gets no bar.
    pub fn tab_open_blank(&mut self, card_id: &str) {
        self.browser_tab_open(card_id, None);
        if self
            .card(card_id)
            .is_some_and(|c| c.kind == CardKind::Browser)
        {
            self.open_omnibox();
            // The "+" is clicked on a card, not on the focus: the bar edits
            // the card that grew the tab.
            self.omni.target = Some(card_id.to_string());
        }
    }

    /// Opens `path` as a tab of an editor card (`ift -t`, #265). `target` is a
    /// card number or id; without one it is the focused editor card, and when
    /// none is focused the file opens in a card of its own, as `ift <file>`
    /// does. A file already open in that card's tabs is switched to, as the
    /// file tree does. The card is focused and brought into view. Returns the
    /// card's id.
    pub fn open_file_tab(
        &mut self,
        path: &str,
        target: Option<&str>,
        from: Option<&str>,
    ) -> Result<String, String> {
        let is_editor = |m: &Model, id: &str| {
            m.card(id)
                .is_some_and(|c| c.kind == crate::saved_layout::CardKind::Editor)
        };
        let id = match target {
            Some(t) => {
                let id = self
                    .resolve_card_ref(t)
                    .ok_or_else(|| format!("no card {t}"))?;
                if !is_editor(self, &id) {
                    return Err(format!("card {t} is not an editor"));
                }
                id
            }
            None => match self
                .selection
                .focused_id
                .clone()
                .filter(|id| is_editor(self, id))
            {
                Some(id) => id,
                None => {
                    return self
                        .open_in_card(
                            crate::ift::open_plan(path, crate::ift::PathKind::File, None),
                            from,
                        )
                        .ok_or_else(|| self.notice.clone().unwrap_or_default());
                }
            },
        };
        ensure_tabs(self, &id);
        let at = self
            .card(&id)
            .and_then(|c| c.tabs.iter().position(|t| t == path));
        match at {
            Some(i) => self.browser_tab_jump(&id, i),
            None => self.browser_tab_open(&id, Some(path)),
        }
        self.set_focus(Some(&id));
        self.reveal_focused();
        Ok(id)
    }

    /// The editor cards of this workspace as palette rows ("#3 main.rs", the
    /// card id as the row id), less `skip`: what the tab-moving pickers list.
    pub fn editor_card_rows(&self, skip: Option<&str>) -> Vec<crate::palette::PaletteItem> {
        self.cards_on(self.active_workspace.as_deref())
            .into_iter()
            .filter(|c| c.kind == CardKind::Editor && Some(c.id.as_str()) != skip)
            .map(|c| {
                let n = c.tabs.len().max(1);
                crate::palette::PaletteItem {
                    id: c.id.clone(),
                    label: self.numbered_label(c),
                    hint: Some(format!("{n} tab{}", if n == 1 { "" } else { "s" })),
                }
            })
            .collect()
    }

    /// Opens a picker when there is something to pick, else says why not.
    fn open_card_picker(&mut self, source: crate::model::palette_state::Source) {
        let skip = (source != crate::model::palette_state::Source::OpenInCard)
            .then(|| self.selection.focused_id.clone())
            .flatten();
        if self.editor_card_rows(skip.as_deref()).is_empty() {
            self.notify("no other editor card on this workspace");
            return;
        }
        self.open_palette(source);
    }

    /// Takes tab `index` out of an editor card's strip and returns its
    /// handle. Nothing is remembered for "reopen closed tab": the tab did not
    /// close, it moved. The caller closes the card when this was its last tab.
    fn take_tab(&mut self, card_id: &str, index: usize) -> Option<String> {
        let card = self.card_mut(card_id)?;
        if index >= card.tabs.len() {
            return None;
        }
        let handle = card.tabs.remove(index);
        if index < card.active_tab {
            card.active_tab -= 1;
        } else if card.active_tab >= card.tabs.len() {
            card.active_tab = card.tabs.len().saturating_sub(1);
        }
        sync_handle(card);
        self.dirty_layout = true;
        Some(handle)
    }

    /// Puts `handle` in `to`'s strip and returns its index there: the tab it
    /// already has, when the file is open in that card, else a new last one.
    fn adopt_tab(&mut self, to: &str, handle: &str) -> usize {
        ensure_tabs(self, to);
        let at = (!handle.is_empty())
            .then(|| {
                self.card(to)
                    .and_then(|c| c.tabs.iter().position(|t| t == handle))
            })
            .flatten();
        match at {
            Some(i) => i,
            None => {
                self.browser_tab_open(to, (!handle.is_empty()).then_some(handle));
                self.card(to).map_or(0, |c| c.tabs.len() - 1)
            }
        }
    }

    /// Both ends of a tab move, checked: two different editor cards.
    fn check_move_ends(&self, from: &str, to: &str) -> Result<(), String> {
        let is_editor = |id: &str| self.card(id).is_some_and(|c| c.kind == CardKind::Editor);
        if from == to {
            return Err("that is this card".into());
        }
        if !is_editor(from) || !is_editor(to) {
            return Err("not an editor card".into());
        }
        Ok(())
    }

    /// The tab's unsaved text is the editor's, not the model's, and the tab
    /// is rebuilt from the file in the other card: so a dirty one stays put.
    /// `dirty` mirrors the active tab, which is the one a move takes.
    fn check_move_clean(&self, from: &str) -> Result<(), String> {
        match self.card(from).filter(|c| c.dirty) {
            Some(c) => {
                let name = c
                    .path
                    .as_deref()
                    .map_or("untitled", crate::paths::base_name);
                Err(format!("save {name} first, it has unsaved changes"))
            }
            None => Ok(()),
        }
    }

    /// `Editor: move tab to card…` (#265): the active tab of `from` becomes a
    /// tab of `to`, which is focused. A file `to` already has open is
    /// switched to instead. A card left with no tabs closes.
    pub fn move_tab_to_card(&mut self, from: &str, to: &str) -> Result<(), String> {
        self.check_move_ends(from, to)?;
        self.check_move_clean(from)?;
        ensure_tabs(self, from);
        let index = self.card(from).map_or(0, |c| c.active_tab);
        let only = self.card(from).is_some_and(|c| c.tabs.len() <= 1);
        let handle = self
            .take_tab(from, index)
            .ok_or_else(|| "no tab to move".to_string())?;
        let at = self.adopt_tab(to, &handle);
        self.browser_tab_jump(to, at);
        if only {
            self.close_card_with(from, false, true);
        }
        self.set_focus(Some(to));
        self.reveal_focused();
        Ok(())
    }

    /// `Editor: move card into card…` (#265): every tab of `from` joins `to`,
    /// the one that was showing stays the one showing, and `from` closes.
    /// A folder editor's blank first buffer is not a tab to carry over.
    pub fn move_card_into_card(&mut self, from: &str, to: &str) -> Result<(), String> {
        self.check_move_ends(from, to)?;
        self.check_move_clean(from)?;
        ensure_tabs(self, from);
        let Some(card) = self.card(from) else {
            return Err("no such card".into());
        };
        let skip_blank = is_bare_folder_editor(card);
        let showing = card.tabs.get(card.active_tab).cloned().unwrap_or_default();
        let handles = card.tabs.clone();
        let mut show = None;
        for handle in handles {
            if skip_blank && handle.is_empty() {
                continue;
            }
            let at = self.adopt_tab(to, &handle);
            if handle == showing {
                show = Some(at);
            }
        }
        if let Some(at) = show {
            self.browser_tab_jump(to, at);
        }
        self.close_card_with(from, false, true);
        self.set_focus(Some(to));
        self.reveal_focused();
        Ok(())
    }

    /// Closes the active tab. Closes the whole card instead when it was
    /// the last tab, `Cmd+W`'s rule.
    pub fn browser_tab_close(&mut self, card_id: &str) {
        ensure_tabs(self, card_id);
        let Some(card) = self.card(card_id) else {
            return;
        };
        if card.tabs.len() <= 1 {
            let focused_is_this = self.selection.focused_id.as_deref() == Some(card_id);
            if focused_is_this {
                self.close_selected();
            }
            return;
        }
        let Some(card) = self.card_mut(card_id) else {
            return;
        };
        let closed = card.tabs.remove(card.active_tab);
        remember_closed(card, closed);
        if card.active_tab >= card.tabs.len() {
            card.active_tab = card.tabs.len() - 1;
        }
        sync_handle(card);
        self.dirty_layout = true;
    }

    /// Closes the tab AT `index`, which is not necessarily the active one —
    /// the strip's `x` on a background tab must not first jump the user
    /// onto it and only then close it (real Chrome does not move your
    /// active tab when you close a different one). `index` before the
    /// active tab shifts it down by one; `index` at the active tab follows
    /// `browser_tab_close`'s own rule (the neighbour that slides into the
    /// slot becomes active, clamped if it was the last); `index` after the
    /// active tab leaves it untouched. Out of range is a no-op, matching
    /// every other index-taking tab command here.
    pub fn browser_tab_close_at(&mut self, card_id: &str, index: usize) {
        ensure_tabs(self, card_id);
        let Some(card) = self.card(card_id) else {
            return;
        };
        if index >= card.tabs.len() {
            return;
        }
        if card.tabs.len() <= 1 {
            let focused_is_this = self.selection.focused_id.as_deref() == Some(card_id);
            if focused_is_this {
                self.close_selected();
            }
            return;
        }
        let Some(card) = self.card_mut(card_id) else {
            return;
        };
        let closed = card.tabs.remove(index);
        remember_closed(card, closed);
        if index < card.active_tab {
            card.active_tab -= 1;
        } else if card.active_tab >= card.tabs.len() {
            card.active_tab = card.tabs.len() - 1;
        }
        sync_handle(card);
        self.dirty_layout = true;
    }

    /// Reopens the most recently closed tab, if there is one.
    pub fn browser_tab_reopen_closed(&mut self, card_id: &str) {
        let Some(card) = self.card_mut(card_id) else {
            return;
        };
        let Some(url) = card.closed_tabs.pop() else {
            return;
        };
        self.browser_tab_open(card_id, Some(&url));
    }

    /// `delta` +1/-1, wrapping.
    pub fn browser_tab_step(&mut self, card_id: &str, delta: i32) {
        ensure_tabs(self, card_id);
        let Some(card) = self.card_mut(card_id) else {
            return;
        };
        if card.tabs.is_empty() {
            return;
        }
        let n = card.tabs.len() as i32;
        card.active_tab = ((card.active_tab as i32 + delta).rem_euclid(n)) as usize;
        sync_handle(card);
        self.dirty_layout = true;
    }

    /// 0-based `index`; out of range is a no-op, matching Chrome's
    /// `Cmd+N` on a window with fewer than N tabs.
    pub fn browser_tab_jump(&mut self, card_id: &str, index: usize) {
        ensure_tabs(self, card_id);
        let Some(card) = self.card_mut(card_id) else {
            return;
        };
        if index >= card.tabs.len() {
            return;
        }
        card.active_tab = index;
        sync_handle(card);
        self.dirty_layout = true;
    }

    pub fn browser_tab_jump_last(&mut self, card_id: &str) {
        ensure_tabs(self, card_id);
        let Some(card) = self.card_mut(card_id) else {
            return;
        };
        if let Some(last) = card.tabs.len().checked_sub(1) {
            card.active_tab = last;
            sync_handle(card);
            self.dirty_layout = true;
        }
    }
}

/// Runs `f` against the focused card's id if it is a browser card, else
/// notifies, matching `browser_history`'s existing not-a-browser message.
fn with_focused_browser(m: &mut Model, f: impl FnOnce(&mut Model, String)) {
    m.with_active_card(|m, id| {
        let Some(card) = m.card(&id) else { return };
        if card.kind != CardKind::Browser {
            m.notify("not a browser card");
            return;
        }
        f(m, id);
    });
}

pub fn register(r: &mut CommandRegistry<Model>) {
    r.register("browser.tab.new", "Browser: new tab…", |m| {
        with_focused_browser(m, |m, id| m.tab_open_blank(&id));
    });
    r.register("browser.tab.close", "Browser: close tab", |m| {
        with_focused_browser(m, |m, id| m.browser_tab_close(&id));
    });
    r.register(
        "browser.tab.reopenClosed",
        "Browser: reopen closed tab",
        |m| {
            with_focused_browser(m, |m, id| m.browser_tab_reopen_closed(&id));
        },
    );
    r.register("browser.tab.next", "Browser: next tab", |m| {
        with_focused_browser(m, |m, id| m.browser_tab_step(&id, 1));
    });
    r.register("browser.tab.prev", "Browser: previous tab", |m| {
        with_focused_browser(m, |m, id| m.browser_tab_step(&id, -1));
    });
    for n in 1..=8 {
        r.register(
            &format!("browser.tab.jump.{n}"),
            &format!("Browser: tab {n}"),
            move |m| with_focused_browser(m, |m, id| m.browser_tab_jump(&id, n - 1)),
        );
    }
    r.register("browser.tab.jump.last", "Browser: last tab", |m| {
        with_focused_browser(m, |m, id| m.browser_tab_jump_last(&id));
    });
    // The editor's, the same functions behind an editor-only guard, so a
    // locked editor card's Cmd+T/W/1..9 mean what a locked browser's do.
    r.register("editor.tab.new", "Editor: new tab", |m| {
        with_focused_editor(m, |m, id| m.browser_tab_open(&id, None));
    });
    // Palette jobs, like `card.moveToWorkspace`: pick the command, then the card.
    r.register(
        "editor.tab.moveToCard",
        "Editor: move tab to card…",
        |m| {
            with_focused_editor(m, |m, _| {
                m.open_card_picker(crate::model::palette_state::Source::MoveTab)
            });
        },
    );
    r.register(
        "editor.card.moveIntoCard",
        "Editor: move card into card…",
        |m| {
            with_focused_editor(m, |m, _| {
                m.open_card_picker(crate::model::palette_state::Source::MoveCardInto)
            });
        },
    );
    r.register(
        "editor.openFileInCard",
        "Editor: open a file in card…",
        |m| m.open_card_picker(crate::model::palette_state::Source::OpenInCard),
    );
    r.register("editor.tab.close", "Editor: close tab", |m| {
        with_focused_editor(m, |m, id| m.browser_tab_close(&id));
    });
    r.register(
        "editor.tab.reopenClosed",
        "Editor: reopen closed tab",
        |m| {
            with_focused_editor(m, |m, id| m.browser_tab_reopen_closed(&id));
        },
    );
    r.register("editor.tab.next", "Editor: next tab", |m| {
        with_focused_editor(m, |m, id| m.browser_tab_step(&id, 1));
    });
    r.register("editor.tab.prev", "Editor: previous tab", |m| {
        with_focused_editor(m, |m, id| m.browser_tab_step(&id, -1));
    });
    for n in 1..=8 {
        r.register(
            &format!("editor.tab.jump.{n}"),
            &format!("Editor: tab {n}"),
            move |m| with_focused_editor(m, |m, id| m.browser_tab_jump(&id, n - 1)),
        );
    }
    r.register("editor.tab.jump.last", "Editor: last tab", |m| {
        with_focused_editor(m, |m, id| m.browser_tab_jump_last(&id));
    });
}

fn with_focused_editor(m: &mut Model, f: impl FnOnce(&mut Model, String)) {
    m.with_active_card(|m, id| {
        let Some(card) = m.card(&id) else { return };
        if card.kind != CardKind::Editor {
            m.notify("not an editor card");
            return;
        }
        f(m, id);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Model, NewCard};

    fn browser_card() -> (Model, String) {
        let mut m = Model::new();
        m.home = "/h".into();
        m.start_dir = "/h".into();
        // `close_selected` (behind `browser_tab_close`'s single-tab case)
        // walks `here()`, which is scoped to `active_workspace`; a card's
        // `workspace_id` defaults to "" (`add_card`'s `unwrap_or_default`),
        // so the active workspace has to match or the card is invisible to
        // selection, same as a real session where `load_layout` sets both
        // before any card exists.
        m.active_workspace = Some(String::new());
        let id = m.add_card(
            "/h",
            NewCard {
                kind: CardKind::Browser,
                url: Some("https://a.example".into()),
                ..Default::default()
            },
        );
        m.set_focus(Some(&id));
        (m, id)
    }

    #[test]
    fn opening_a_tab_appends_and_activates_it() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        let card = m.card(&id).unwrap();
        assert_eq!(card.tabs, vec!["https://a.example", "https://b.example"]);
        assert_eq!(card.active_tab, 1);
        assert_eq!(card.url.as_deref(), Some("https://b.example"));
    }

    #[test]
    fn closing_the_active_tab_removes_it_and_activates_a_neighbour() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        m.browser_tab_open(&id, Some("https://c.example"));
        m.browser_tab_jump(&id, 1); // b.example active
        m.browser_tab_close(&id);
        let card = m.card(&id).unwrap();
        assert_eq!(card.tabs, vec!["https://a.example", "https://c.example"]);
        assert_eq!(card.active_tab, 1, "the tab that slid into the closed slot");
        assert_eq!(card.closed_tabs, vec!["https://b.example"]);
    }

    #[test]
    fn closing_the_last_tab_closes_the_card_instead() {
        let (mut m, id) = browser_card();
        m.browser_tab_close(&id);
        assert!(m.card(&id).is_none());
    }

    // Closing a BACKGROUND tab by index must not move you off the tab you
    // are looking at: [a, b, c] with c active, closing index 0 (a) leaves
    // c active still, not "whatever is now at the old active index" (which
    // a naive jump-then-close-active would give: index 2 after removal is
    // c anyway here, so the real regression case is caught by the next
    // test, where the shift would land on the wrong tab).
    #[test]
    fn closing_a_non_active_tab_by_index_leaves_the_active_tab_the_same() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        m.browser_tab_open(&id, Some("https://c.example"));
        m.browser_tab_jump(&id, 2); // c.example active
        m.browser_tab_close_at(&id, 0); // close a.example, not the active one
        let card = m.card(&id).unwrap();
        assert_eq!(card.tabs, vec!["https://b.example", "https://c.example"]);
        assert_eq!(
            card.active_tab, 1,
            "still on c.example, shifted down by the removed tab before it"
        );
        assert_eq!(card.url.as_deref(), Some("https://c.example"));
        assert_eq!(card.closed_tabs, vec!["https://a.example"]);
    }

    // The mirror case: closing a tab AFTER the active one must not move
    // the active index at all.
    #[test]
    fn closing_a_later_tab_by_index_does_not_move_the_active_index() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        m.browser_tab_open(&id, Some("https://c.example"));
        m.browser_tab_jump(&id, 0); // a.example active
        m.browser_tab_close_at(&id, 2); // close c.example, after the active one
        let card = m.card(&id).unwrap();
        assert_eq!(card.tabs, vec!["https://a.example", "https://b.example"]);
        assert_eq!(card.active_tab, 0, "unaffected: a.example is still active");
        assert_eq!(card.url.as_deref(), Some("https://a.example"));
    }

    // Closing the ACTIVE tab by its own index is the same rule
    // `browser_tab_close` already gives it: the neighbour that slides into
    // the slot becomes active.
    #[test]
    fn closing_the_active_tab_by_index_activates_the_neighbour_that_slides_in() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        m.browser_tab_open(&id, Some("https://c.example"));
        m.browser_tab_jump(&id, 1); // b.example active
        m.browser_tab_close_at(&id, 1);
        let card = m.card(&id).unwrap();
        assert_eq!(card.tabs, vec!["https://a.example", "https://c.example"]);
        assert_eq!(card.active_tab, 1, "the tab that slid into the closed slot");
    }

    // Closing the active tab by index when it was the LAST tab still closes
    // the whole card, matching `browser_tab_close`'s existing behavior.
    #[test]
    fn closing_the_active_tab_by_index_closes_the_card_when_it_was_the_last_tab() {
        let (mut m, id) = browser_card();
        m.browser_tab_close_at(&id, 0);
        assert!(m.card(&id).is_none());
    }

    // An out-of-range index is a no-op: the click that produced it came
    // from a strip diffed against `card.tabs`, and a stale click must not
    // touch a tab it no longer names.
    #[test]
    fn closing_by_an_out_of_range_index_is_a_no_op() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        m.dirty_layout = false;
        m.browser_tab_close_at(&id, 5);
        assert_eq!(m.card(&id).unwrap().tabs.len(), 2);
        assert!(!m.dirty_layout);
    }

    #[test]
    fn reopen_closed_puts_the_last_closed_tab_back_and_activates_it() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        m.browser_tab_close(&id); // closes b.example (was active)
        m.browser_tab_reopen_closed(&id);
        let card = m.card(&id).unwrap();
        assert_eq!(card.tabs, vec!["https://a.example", "https://b.example"]);
        assert_eq!(card.active_tab, 1);
        assert!(card.closed_tabs.is_empty());
    }

    #[test]
    fn next_and_prev_wrap() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        m.dirty_layout = false; // opening already dirtied it; isolate the step
        m.browser_tab_step(&id, 1);
        assert_eq!(m.card(&id).unwrap().active_tab, 0, "wrapped past the end");
        assert!(
            m.dirty_layout,
            "a tab switch must reach the save file or a restart loses it"
        );
        m.browser_tab_step(&id, -1);
        assert_eq!(m.card(&id).unwrap().active_tab, 1, "wrapped past the start");
    }

    #[test]
    fn jump_out_of_range_is_a_no_op() {
        let (mut m, id) = browser_card();
        m.dirty_layout = false; // set_focus in the fixture already dirtied it
        m.browser_tab_jump(&id, 5);
        assert_eq!(m.card(&id).unwrap().active_tab, 0);
        assert!(!m.dirty_layout, "a no-op jump changed nothing to save");
        m.browser_tab_jump(&id, 0);
        assert!(m.dirty_layout, "a real jump does");
    }

    // An editor card's tabs are paths: the active one is `card.path`, an
    // empty handle is an untitled buffer, and the closed-tab ring, the
    // stepping and the jumps are the browser's.
    #[test]
    fn an_editor_cards_tabs_are_paths_mirrored_into_its_path() {
        let mut m = Model::new();
        m.home = "/h".into();
        m.start_dir = "/h".into();
        m.active_workspace = Some(String::new());
        let id = m.add_card(
            "/h",
            NewCard {
                kind: CardKind::Editor,
                path: Some("/h/a.rs".into()),
                ..Default::default()
            },
        );
        m.set_focus(Some(&id));
        m.browser_tab_open(&id, Some("/h/b.rs"));
        let card = m.card(&id).unwrap();
        assert_eq!(card.tabs, vec!["/h/a.rs", "/h/b.rs"]);
        assert_eq!(card.path.as_deref(), Some("/h/b.rs"));
        assert_eq!(card.url, None, "a path is not a url");
        m.browser_tab_open(&id, None);
        assert_eq!(
            m.card(&id).unwrap().path,
            None,
            "an empty handle is untitled"
        );
        m.browser_tab_step(&id, -1);
        assert_eq!(m.card(&id).unwrap().path.as_deref(), Some("/h/b.rs"));
        m.browser_tab_close(&id);
        assert_eq!(m.card(&id).unwrap().tabs, vec!["/h/a.rs", ""]);
        m.browser_tab_reopen_closed(&id);
        assert_eq!(m.card(&id).unwrap().path.as_deref(), Some("/h/b.rs"));
    }

    // #572: Cmd+W on an editor opened on a folder closes one tab; the last
    // tab, and a card with no folder, close the card as before.
    #[test]
    fn card_close_on_a_folder_editor_closes_the_active_tab_first() {
        let mut m = Model::new();
        m.home = "/h".into();
        m.start_dir = "/h".into();
        m.active_workspace = Some(String::new());
        let id = m.add_card(
            "/h/proj",
            NewCard {
                kind: CardKind::Editor,
                root: Some("/h/proj".into()),
                ..Default::default()
            },
        );
        m.set_focus(Some(&id));
        m.browser_tab_open(&id, Some("/h/proj/a.rs"));
        m.browser_tab_open(&id, Some("/h/proj/b.rs"));
        m.close_selected();
        assert_eq!(m.card(&id).unwrap().tabs, vec!["", "/h/proj/a.rs"]);
        m.close_selected();
        assert_eq!(m.card(&id).unwrap().tabs, vec![""]);
        m.close_selected();
        assert!(m.card(&id).is_none(), "the last tab closes the card");
    }

    // `editor.folderScratchTab` off: the blank buffer is not a tab, so the
    // first file takes its place and Cmd+W on that file closes the card.
    #[test]
    fn without_the_scratch_tab_the_first_file_replaces_the_blank_buffer() {
        let setup = |scratch: bool| {
            let mut m = Model::new();
            m.config.editor.folder_scratch_tab = scratch;
            m.home = "/h".into();
            m.start_dir = "/h".into();
            m.active_workspace = Some(String::new());
            let id = m.add_card(
                "/h/proj",
                NewCard {
                    kind: CardKind::Editor,
                    root: Some("/h/proj".into()),
                    ..Default::default()
                },
            );
            m.set_focus(Some(&id));
            (m, id)
        };
        let (mut m, id) = setup(false);
        m.browser_tab_open(&id, Some("/h/proj/a.rs"));
        assert_eq!(m.card(&id).unwrap().tabs, vec!["/h/proj/a.rs"]);
        m.browser_tab_open(&id, Some("/h/proj/b.rs"));
        assert_eq!(m.card(&id).unwrap().tabs.len(), 2, "later files add tabs");
        m.browser_tab_open(&id, None);
        assert_eq!(m.card(&id).unwrap().tabs.len(), 3, "+ still adds a tab");
        let (mut m, id) = setup(false);
        m.browser_tab_open(&id, Some("/h/proj/a.rs"));
        m.close_selected();
        assert!(m.card(&id).is_none(), "the only file closes the card");
        let (mut m, id) = setup(true);
        m.browser_tab_open(&id, Some("/h/proj/a.rs"));
        assert_eq!(m.card(&id).unwrap().tabs, vec!["", "/h/proj/a.rs"]);
    }

    // The same sheet as a card's, for the active tab: Cancel keeps it, Don't
    // Save closes it, Save closes it once the editor reports the file clean,
    // and a save that never lands closes nothing.
    #[test]
    fn a_dirty_folder_tab_asks_before_cmd_w_closes_it() {
        use crate::prompt::ButtonKind;
        let setup = || {
            let mut m = Model::new();
            m.home = "/h".into();
            m.start_dir = "/h".into();
            m.active_workspace = Some(String::new());
            let id = m.add_card(
                "/h/proj",
                NewCard {
                    kind: CardKind::Editor,
                    root: Some("/h/proj".into()),
                    ..Default::default()
                },
            );
            m.set_focus(Some(&id));
            m.browser_tab_open(&id, Some("/h/proj/a.rs"));
            m.card_mut(&id).unwrap().dirty = true; // the editor's mirror of the active tab
            m.close_selected();
            (m, id)
        };
        let press = |m: &mut Model, k: ButtonKind| {
            let (pending, text) = m.prompt.press(k).unwrap();
            m.answer(pending, text, |_| true);
        };

        let (mut m, id) = setup();
        assert!(m.prompt.open && m.prompt.label == "save changes to a.rs?");
        assert_eq!(m.card(&id).unwrap().tabs.len(), 2, "asked, not closed");
        press(&mut m, ButtonKind::Cancel);
        assert_eq!(m.card(&id).unwrap().tabs.len(), 2, "Cancel keeps it");

        let (mut m, id) = setup();
        press(&mut m, ButtonKind::Alt);
        assert_eq!(m.card(&id).unwrap().tabs, vec![""], "Don't Save closes it");

        let (mut m, id) = setup();
        press(&mut m, ButtonKind::Primary);
        assert_eq!(
            m.card(&id).unwrap().tabs.len(),
            2,
            "not before the save lands"
        );
        m.card_mut(&id).unwrap().dirty = false;
        m.tick(m.now_ms + 16.);
        assert_eq!(m.card(&id).unwrap().tabs, vec![""], "saved, then closed");

        let (mut m, id) = setup();
        press(&mut m, ButtonKind::Primary);
        m.tick(m.now_ms + 3000.);
        assert_eq!(
            m.card(&id).unwrap().tabs.len(),
            2,
            "a save that never landed closes nothing"
        );
    }

    /// Two editor cards: `a` with tabs a.rs and b.rs (b.rs showing), `b` with c.rs.
    fn two_editors() -> (Model, String, String) {
        let mut m = Model::new();
        m.home = "/h".into();
        m.start_dir = "/h".into();
        m.active_workspace = Some(String::new());
        let new = |m: &mut Model, path: &str| {
            m.add_card(
                "/h",
                NewCard {
                    kind: CardKind::Editor,
                    path: Some(path.into()),
                    ..Default::default()
                },
            )
        };
        let a = new(&mut m, "/h/a.rs");
        let b = new(&mut m, "/h/c.rs");
        m.browser_tab_open(&a, Some("/h/b.rs"));
        m.set_focus(Some(&a));
        (m, a, b)
    }

    #[test]
    fn a_tab_moves_to_another_card_and_leaves_the_rest() {
        let (mut m, a, b) = two_editors();
        m.move_tab_to_card(&a, &b).unwrap();
        let from = m.card(&a).unwrap();
        assert_eq!(from.tabs, vec!["/h/a.rs"]);
        assert_eq!(from.path.as_deref(), Some("/h/a.rs"));
        assert!(from.closed_tabs.is_empty(), "moved, not closed");
        let to = m.card(&b).unwrap();
        assert_eq!(to.tabs, vec!["/h/c.rs", "/h/b.rs"]);
        assert_eq!(to.active_tab, 1);
        assert_eq!(to.path.as_deref(), Some("/h/b.rs"));
        assert_eq!(m.selection.focused_id.as_deref(), Some(b.as_str()));
    }

    #[test]
    fn the_last_tab_of_a_card_closes_it_when_it_moves() {
        let (mut m, a, b) = two_editors();
        m.move_tab_to_card(&b, &a).unwrap();
        assert!(m.card(&b).is_none());
        assert_eq!(
            m.card(&a).unwrap().tabs,
            vec!["/h/a.rs", "/h/b.rs", "/h/c.rs"]
        );
    }

    #[test]
    fn a_file_the_target_has_open_is_switched_to_not_doubled() {
        let (mut m, a, b) = two_editors();
        m.browser_tab_open(&b, Some("/h/b.rs"));
        m.browser_tab_jump(&b, 0);
        m.move_tab_to_card(&a, &b).unwrap();
        let to = m.card(&b).unwrap();
        assert_eq!(to.tabs, vec!["/h/c.rs", "/h/b.rs"]);
        assert_eq!(to.active_tab, 1);
    }

    #[test]
    fn a_dirty_tab_stays_where_it_is() {
        let (mut m, a, b) = two_editors();
        m.card_mut(&a).unwrap().dirty = true;
        let err = m.move_tab_to_card(&a, &b).unwrap_err();
        assert_eq!(err, "save b.rs first, it has unsaved changes");
        assert_eq!(m.card(&a).unwrap().tabs.len(), 2);
        assert_eq!(m.card(&b).unwrap().path.as_deref(), Some("/h/c.rs"));
        assert!(m.card(&b).unwrap().tabs.len() <= 1, "nothing arrived");
        assert!(m.move_card_into_card(&a, &b).is_err());
    }

    #[test]
    fn a_card_moves_into_another_with_its_active_tab_showing() {
        let (mut m, a, b) = two_editors();
        m.move_card_into_card(&a, &b).unwrap();
        assert!(m.card(&a).is_none(), "the emptied card closes");
        let to = m.card(&b).unwrap();
        assert_eq!(to.tabs, vec!["/h/c.rs", "/h/a.rs", "/h/b.rs"]);
        assert_eq!(to.path.as_deref(), Some("/h/b.rs"), "b.rs was showing");
    }

    #[test]
    fn moving_needs_two_different_editors() {
        let (mut m, a, _) = two_editors();
        assert_eq!(m.move_tab_to_card(&a, &a).unwrap_err(), "that is this card");
        let term = m.add_card("/h", NewCard::default());
        assert!(m.move_tab_to_card(&a, &term).is_err());
        assert!(m.move_card_into_card(&term, &a).is_err());
        assert_eq!(m.card(&a).unwrap().tabs.len(), 2);
    }

    #[test]
    fn the_pickers_list_the_other_editor_cards() {
        use crate::model::palette_state::Source;
        let (mut m, a, b) = two_editors();
        m.add_card("/h", NewCard::default()); // a terminal is never a row
        let rows = m.palette_items(Source::MoveTab, &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, b);
        assert_eq!(rows[0].hint.as_deref(), Some("1 tab"));
        assert_eq!(m.palette_items(Source::OpenInCard, &[]).len(), 2);
        m.palette_run(Source::MoveTab, &b);
        assert_eq!(m.card(&b).unwrap().tabs.len(), 2);
        assert_eq!(m.card(&a).unwrap().tabs.len(), 1);
    }

    #[test]
    fn open_in_card_asks_the_open_panel_and_the_pick_becomes_a_tab() {
        use crate::model::palette_state::Source;
        use crate::model::Effect;
        let (mut m, a, b) = two_editors();
        m.palette_run(Source::OpenInCard, &b);
        let effects = std::mem::take(&mut m.effects);
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::PickFile { from, into } if from.as_deref() == Some(a.as_str())
                && into.as_deref() == Some(b.as_str())
        )));
        m.open_picked_into("/h/new.rs", &b, Some(&a));
        assert_eq!(m.card(&b).unwrap().tabs, vec!["/h/c.rs", "/h/new.rs"]);
    }

    #[test]
    fn a_non_browser_card_notifies_and_changes_nothing() {
        let mut m = Model::new();
        m.home = "/h".into();
        m.start_dir = "/h".into();
        m.active_workspace = Some(String::new());
        let id = m.add_card("/h", NewCard::default());
        m.set_focus(Some(&id));
        m.browser_tab_open(&id, None); // direct call still works, no notice
        with_focused_browser(&mut m, |_, _| panic!("must not run: not a browser card"));
        assert_eq!(m.notice.as_deref(), Some("not a browser card"));
    }
}
