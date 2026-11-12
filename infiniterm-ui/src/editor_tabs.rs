//! An editor card with tabs: one `EditorBody` (a buffer with its file,
//! drafts, undo, search) per tab, the tab strip across the top, and the
//! focus lock the browser card has. This is the card's body; `EditorBody`
//! is a tab's. The tree is one per card and travels with the active tab.
//!
//! `Card.tabs` is the authority, the way it is for a browser card:
//! `editors.rs::reconcile_editors` calls `rebuild_tabs` every frame with
//! the card's handles (paths, an empty one for an untitled buffer), and a
//! tab command from the strip or the keyboard goes through the model
//! (`tabs_cmd.rs`) rather than mutating here, so a second, opposite path
//! cannot fight the first. A body is matched to a handle in order, first
//! come, so closing one tab never reloads the others and two untitled
//! tabs keep their own text.
//!
//! Focus lock: a click into the text or the tree, or a bare Enter on a
//! focused card, locks (`locked`); `editors.rs` mirrors it onto
//! `card.locked`, which is what `handle_chord` reads to route Cmd+T/W/1..9
//! to the tab commands. Double-Escape unlocks (`input.rs`, the browser's
//! rule). The strip, the tree's divider and the find bar do not lock. The
//! tree locks so its arrows work after a click (#776); with the tree
//! focused only its keys are taken, so nothing typed reaches the file.
//!
//! Drafts are keyed per tab (`tab_draft_id`): the first tab keeps the
//! card's own id so a draft from before tabs is still found.
use crate::body::{BodyAction, CardBody};
use crate::editor_body::{EditorBody, EditorEvent};
use crate::tab_strip::{
    hover_lines, hovered, paint_hover_label, paint_strip, strip_hit, strip_world_h, StripStyle,
    TabDrag,
};
use crate::terminal_body::Metrics;
use gpui::{point, size, App, Bounds, Keystroke, Pixels, Window};
use infiniterm_core::grid::{Point, Size};

/// The draft id of the tab at `index` of card `card_id`: the first tab keeps
/// the card's own id, a file's is its path hashed, an untitled one counts
/// `serial` up. A restore hands them out in tab order from a zero `serial`.
fn draft_id(card_id: &str, index: usize, handle: &str, serial: &mut u64) -> String {
    if index == 0 {
        return card_id.to_string();
    }
    if handle.is_empty() {
        *serial += 1;
        return format!("{card_id}~untitled{serial}");
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    handle.hash(&mut h);
    format!("{card_id}~{:x}", h.finish())
}

pub struct EditorTabs {
    pub card_id: String,
    /// (handle, body): the handle is the path, or "" for untitled.
    tabs: Vec<(String, EditorBody)>,
    active: usize,
    /// Untitled tabs past the first get a draft id of their own.
    untitled_serial: u64,
    pub locked: bool,
    pub last_escape_ms: Option<f64>,
    pub ui_scale: f32,
    pub card_number: u32,
    /// Mirrors `card.protected`: `card.protect`'s own lock, a different
    /// one from `locked` above (the keyboard's), shown beside `#N`.
    pub protected: bool,
    /// `editor.folderScratchTab`, mirrored by `editors.rs` each frame.
    pub scratch_tab: bool,
    pub style: StripStyle,
    /// Whether the strip is drawn. A diff card is one body in this wrapper
    /// with no tabs and no strip (`new_diff`): it takes the editor's lock and
    /// key routing from here, which is the point of hosting it.
    strip: bool,
    /// A tab held by the pointer, from the press to the release (#682).
    tab_drag: Option<TabDrag>,
    /// The tab under the pointer, whose full name and path the hover label
    /// shows (#742).
    hover_tab: Option<usize>,
    /// A drop since `editors.rs` last looked: (from, to) for `Model::move_tab`.
    pub tab_drop: Option<(usize, usize)>,
    /// Bounds of the last paint, for the strip's hit test.
    world: Size,
    dirty: bool,
    metrics: Metrics,
    /// Where the card's files are (#560): handed to each tab's body before it
    /// loads. This machine's disk unless the window is a joined one.
    pub source: std::sync::Arc<dyn infiniterm_core::file_source::FileSource>,
}

impl EditorTabs {
    /// Escape still has something to close in the active tab (a popup, the
    /// find panel, extra cursors, a selection); see `EditorBody::escape_has_work`.
    pub fn escape_has_work(&self) -> bool {
        self.active_body_ref().is_some_and(|b| b.escape_has_work())
    }

    /// The tab under a point of the card (a right-click's), `None` off the
    /// tabs, on the "+" and below the strip.
    pub fn tab_at(&self, local: Point) -> Option<usize> {
        if !self.strip {
            return None;
        }
        match self.hit(local) {
            Some(crate::body::TabClick::Switch(i) | crate::body::TabClick::Close(i)) => Some(i),
            _ => None,
        }
    }

    /// The tree row at a point of the card body, in the active tab.
    pub fn tree_row_at(&self, local: Point) -> Option<usize> {
        let below = self.below_strip(local);
        self.active_body_ref().and_then(|b| b.tree_row_at(below))
    }

    /// Lights (or clears) the tree row a menu is open on.
    pub fn set_menu_row(&mut self, row: Option<usize>) {
        if let Some(b) = self.active_body() {
            b.set_menu_row(row);
        }
    }

    /// The editor's part of a keybinding's context (`EditorBody::context_flags`).
    pub fn context_flags(&self) -> infiniterm_core::model::UiContext {
        self.active_body_ref()
            .map(|b| b.context_flags())
            .unwrap_or_default()
    }

    /// The resize cursor while the tree's divider is under the pointer or held.
    pub fn divider_cursor(&self) -> Option<gpui::CursorStyle> {
        self.active_body_ref().and_then(|b| b.divider_cursor())
    }

    pub fn new(card_id: &str, metrics: &Metrics, world: Size, style: StripStyle) -> EditorTabs {
        EditorTabs {
            card_id: card_id.to_string(),
            tabs: vec![],
            active: 0,
            untitled_serial: 0,
            locked: false,
            last_escape_ms: None,
            ui_scale: 1.,
            card_number: 0,
            protected: false,
            scratch_tab: true,
            style,
            strip: true,
            tab_drag: None,
            hover_tab: None,
            tab_drop: None,
            world,
            dirty: true,
            metrics: metrics.clone(),
            source: std::sync::Arc::new(infiniterm_core::file_source::LocalSource),
        }
    }

    /// A diff card's body: one `EditorBody` in diff mode and no strip. The
    /// card's `Card.tabs` stays empty; `rebuild_tabs` is never called on it.
    pub fn new_diff(
        card_id: &str,
        root: &str,
        metrics: &Metrics,
        world: Size,
        style: StripStyle,
    ) -> EditorTabs {
        let mut tabs = EditorTabs::new(card_id, metrics, world, style);
        tabs.strip = false;
        tabs.tabs = vec![(
            String::new(),
            EditorBody::new_diff(card_id, root, metrics, world),
        )];
        tabs
    }

    /// The strip's height in world px: none for a diff.
    fn strip_h(&self) -> f64 {
        if self.strip {
            strip_world_h(self.style.font_px, self.ui_scale)
        } else {
            0.
        }
    }

    pub fn active_body(&mut self) -> Option<&mut EditorBody> {
        self.tabs.get_mut(self.active).map(|(_, b)| b)
    }

    pub fn active_body_ref(&self) -> Option<&EditorBody> {
        self.tabs.get(self.active).map(|(_, b)| b)
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// The draft id a tab's body keeps its unsaved text under.
    fn tab_draft_id(&mut self, index: usize, handle: &str) -> String {
        draft_id(&self.card_id, index, handle, &mut self.untitled_serial)
    }

    /// Puts tab `from` at `to`, as `Model::move_tab` did to `card.tabs`
    /// (#682): each body goes with its tab, the active one stays active.
    /// Out of range is a no-op, as for every index from the card.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() || to >= self.tabs.len() || from == to {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = infiniterm_core::model::tabs_cmd::index_after_move(self.active, from, to);
        self.rekey_drafts();
        self.dirty = true;
    }

    /// A restored card finds each tab's draft by its PLACE (`draft_id`), so
    /// after a move every draft goes under the id its tab's new place will
    /// look for: else the next start would put one file's unsaved text in
    /// another's tab. Read and cleared first, written after, so two tabs
    /// trading ids cannot overwrite each other.
    fn rekey_drafts(&mut self) {
        let mut serial = 0;
        let want: Vec<String> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, (h, _))| draft_id(&self.card_id, i, h, &mut serial))
            .collect();
        self.untitled_serial = self.untitled_serial.max(serial);
        let mut moving = vec![];
        for ((_, body), id) in self.tabs.iter_mut().zip(want) {
            if body.card_id == id {
                continue;
            }
            let text = infiniterm_core::files::draft_read(&body.card_id)
                .ok()
                .flatten();
            let _ = infiniterm_core::files::draft_delete(&body.card_id);
            body.card_id = id.clone();
            moving.push((id, text));
        }
        for (id, text) in moving {
            let _ = match text {
                Some(t) => infiniterm_core::files::draft_write(&id, &t),
                None => infiniterm_core::files::draft_delete(&id),
            };
        }
    }

    fn body_for(
        &mut self,
        index: usize,
        handle: &str,
        cwd: &str,
        now: f64,
        restoring: bool,
    ) -> EditorBody {
        let draft_id = self.tab_draft_id(index, handle);
        let path = (!handle.is_empty()).then(|| handle.to_string());
        let mut body = EditorBody::new(
            &draft_id,
            path.clone(),
            cwd.to_string(),
            &self.metrics,
            self.text_world(),
        );
        // Before the load: a host's file is read through it, on a thread.
        body.source = self.source.clone();
        match path {
            Some(p) => body.load(&p, restoring, now),
            None => body.load_untitled(),
        }
        body
    }

    /// The card's world minus the strip: what a tab's body lays out in.
    fn text_world(&self) -> Size {
        Size {
            w: self.world.w,
            h: (self.world.h - self.strip_h()).max(1.),
        }
    }

    /// Makes the bodies match `handles` (the card's tabs, or its one path
    /// when it has none yet) and `active`. Matched in order by handle so
    /// a close keeps every other tab's buffer; a tab that becomes active
    /// takes the tree from the one that was.
    pub fn rebuild_tabs(
        &mut self,
        handles: &[String],
        active: usize,
        cwd: &str,
        now: f64,
        restoring: bool,
    ) {
        let handles: Vec<String> = if handles.is_empty() {
            vec![String::new()]
        } else {
            handles.to_vec()
        };
        let same = self.tabs.len() == handles.len()
            && self
                .tabs
                .iter()
                .zip(&handles)
                .all(|((h, _), want)| h == want);
        if !same {
            let mut old: Vec<Option<(String, EditorBody)>> = std::mem::take(&mut self.tabs)
                .into_iter()
                .map(Some)
                .collect();
            let mut next: Vec<(String, EditorBody)> = Vec::with_capacity(handles.len());
            for (i, want) in handles.iter().enumerate() {
                let found = old
                    .iter_mut()
                    .find(|slot| slot.as_ref().is_some_and(|(h, _)| h == want))
                    .and_then(Option::take);
                let body = match found {
                    Some((_, b)) => b,
                    None => self.body_for(i, want, cwd, now, restoring),
                };
                next.push((want.clone(), body));
            }
            // Whichever old tab held the tree hands it to the new active.
            let tree = old
                .iter_mut()
                .flatten()
                .find_map(|(_, b)| b.take_tree())
                .or_else(|| next.iter_mut().find_map(|(_, b)| b.take_tree()));
            self.tabs = next;
            self.active = active.min(self.tabs.len().saturating_sub(1));
            if let Some((tree, focused, w, top)) = tree {
                if let Some((_, b)) = self.tabs.get_mut(self.active) {
                    b.put_tree(tree, focused, w, top);
                }
            }
            self.dirty = true;
            return;
        }
        let active = active.min(self.tabs.len().saturating_sub(1));
        if active != self.active {
            let tree = self.tabs[self.active].1.take_tree();
            self.active = active;
            if let Some((tree, focused, w, top)) = tree {
                self.tabs[active].1.put_tree(tree, focused, w, top);
            }
            self.dirty = true;
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Every tab's housekeeping (drafts, the disk poll): the wrapper is
    /// what the idle loop finds, so a downcast to `EditorBody` there found
    /// nothing and no tab polled its file for a day.
    pub fn idle(&mut self, now: f64) {
        for (_, b) in &mut self.tabs {
            b.idle(now);
        }
        if self.tabs.iter().any(|(_, b)| b.wants_frame(now)) {
            self.dirty = true;
        }
    }

    /// Every tab's events, the active one's first: a `Notice` from a
    /// background tab (its disk poll does not run, but a save might) is
    /// still worth a line.
    pub fn take_events(&mut self) -> Vec<EditorEvent> {
        let mut out = vec![];
        let active = self.active;
        if let Some((_, b)) = self.tabs.get_mut(active) {
            out.extend(b.take_events());
        }
        for (i, (_, b)) in self.tabs.iter_mut().enumerate() {
            if i != active {
                out.extend(b.take_events());
            }
        }
        out
    }

    /// The folder header over the tree: the folder's name and the tree's
    /// width in world px. Only with the tree on the left (a tree above the
    /// text has no column to sit over) and a strip to draw it in.
    fn header(&self) -> Option<(String, f64)> {
        if !self.strip {
            return None;
        }
        let b = self.active_body_ref()?;
        let root = b.tree_root()?;
        (!b.sidebar_top && b.sidebar_w > 0.).then(|| (folder_label(root), b.sidebar_w))
    }

    fn header_w(&self) -> f64 {
        self.header().map_or(0., |(_, w)| w)
    }

    /// The strip's hit test, past the folder header.
    fn hit(&self, local: Point) -> Option<crate::body::TabClick> {
        strip_hit(
            local,
            self.style.font_px,
            self.ui_scale,
            self.labels().len(),
            self.header_w(),
            self.style.pad_x(),
        )
    }

    /// `editor.folderScratchTab` off: a folder editor's blank first buffer
    /// is neither a tab nor a place to type (`tabs_cmd.rs`).
    fn scratch_hidden(&self) -> bool {
        !self.scratch_tab
            && self.tabs.len() == 1
            && self.tabs[0].0.is_empty()
            && self.header().is_some()
    }

    /// A tab's label: the file's name, `untitled`, with a mark when dirty.
    /// None while the scratch buffer is hidden.
    fn labels(&self) -> Vec<String> {
        if self.scratch_hidden() {
            return vec![];
        }
        self.tabs
            .iter()
            .map(|(h, b)| {
                let name = if h.is_empty() {
                    "untitled".to_string()
                } else {
                    infiniterm_core::paths::base_name(h).to_string()
                };
                if b.is_dirty() {
                    format!("{name} •")
                } else {
                    name
                }
            })
            .collect()
    }

    /// The hover label's lines for tab `i` (#742): its full name, then its
    /// path with the home folder as `~`. None while the scratch buffer is
    /// hidden, as `labels`.
    fn hover_text(&self, i: usize) -> Option<Vec<String>> {
        if self.scratch_hidden() {
            return None;
        }
        let (h, _) = self.tabs.get(i)?;
        if h.is_empty() {
            return Some(hover_lines("untitled", None, self.style.hover_detail));
        }
        let home = std::env::var("HOME").unwrap_or_default();
        let path = infiniterm_core::card_label::tilde_path(h, &home);
        Some(hover_lines(
            infiniterm_core::paths::base_name(h),
            Some(&path),
            self.style.hover_detail,
        ))
    }

    /// Notes the tab under the pointer, asking for a frame when it changed.
    fn set_hover(&mut self, tab: Option<usize>) {
        if self.hover_tab != tab {
            self.hover_tab = tab;
            self.dirty = true;
        }
    }

    fn below_strip(&self, local: Point) -> Point {
        Point {
            x: local.x,
            y: local.y - self.strip_h(),
        }
    }
}

impl CardBody for EditorTabs {
    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        scale: f64,
        focused: bool,
        now: f64,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.dirty = false;
        self.world = Size {
            w: f32::from(bounds.size.width) as f64 / scale,
            h: f32::from(bounds.size.height) as f64 / scale,
        };
        let strip_h = if self.strip {
            let labels = self.labels();
            let header = self.header();
            paint_strip(
                bounds,
                scale,
                self.ui_scale,
                &labels,
                self.active,
                self.tab_drag.as_ref(),
                self.card_number,
                self.protected,
                header.as_ref().map(|(n, w)| (n.as_str(), *w)),
                &self.style,
                window,
                cx,
            )
        } else {
            gpui::px(0.)
        };
        let below = Bounds::new(
            point(bounds.origin.x, bounds.origin.y + strip_h),
            size(
                bounds.size.width,
                (bounds.size.height - strip_h).max(gpui::px(1.)),
            ),
        );
        if let Some((_, b)) = self.tabs.get_mut(self.active) {
            window.with_content_mask(Some(gpui::ContentMask { bounds: below }), |window| {
                b.paint(below, scale, focused, now, window, cx)
            });
        }
        // Over the text, after it. Not while a tab is being dragged.
        let dragging = self.tab_drag.is_some_and(|d| d.moving);
        if let Some(i) = self.hover_tab.filter(|_| self.strip && !dragging) {
            if let Some(lines) = self.hover_text(i) {
                paint_hover_label(
                    bounds,
                    scale,
                    self.ui_scale,
                    i,
                    self.header_w(),
                    &lines,
                    &self.style,
                    window,
                    cx,
                );
            }
        }
    }

    fn resized(&mut self, world: Size) {
        self.world = world;
        let text = self.text_world();
        for (_, b) in &mut self.tabs {
            b.resized(text);
        }
        self.dirty = true;
    }

    fn insert_text(&mut self, text: &str) {
        // Text from the emoji panel or an input method reaches only an
        // editor you are in, like every key.
        if !self.locked {
            return;
        }
        if let Some(b) = self.active_body() {
            b.insert_text(text);
        }
    }

    fn caret_bounds(&self) -> Option<Bounds<Pixels>> {
        self.active_body_ref().and_then(|b| b.caret_bounds())
    }

    fn key(&mut self, k: &Keystroke, now: f64, cx: &mut App) -> BodyAction {
        // An editor you are not IN takes no keys at all. Bare Enter is the
        // one way in from the keyboard (a click in the text is the mouse's),
        // the browser card's rule. It used to be every plain key, which
        // locked and swallowed whatever you pressed; Ekin found that too
        // loose: an arrowed-to editor is a card on the canvas, drawn dimmed,
        // and nothing typed at the canvas may end up in a file.
        // Swallowed rather than ignored, so macOS neither repeats a Cmd key
        // nor hands a letter to the input context behind our back.
        if !self.locked {
            if enters(k) && !self.scratch_hidden() {
                self.locked = true;
                self.dirty = true;
            }
            return BodyAction::None;
        }
        if k.key != "escape" {
            self.last_escape_ms = None;
        }
        // The word popup draws on the card's other tabs too; their word
        // lists are cached per tab and rebuilt only when that tab's text
        // changes.
        let active = self.active;
        let others: Vec<(String, std::sync::Arc<Vec<String>>)> = if self.tabs.len() > 1 {
            self.tabs
                .iter_mut()
                .enumerate()
                .filter(|(i, _)| *i != active)
                .map(|(_, (handle, b))| {
                    let name = match infiniterm_core::paths::base_name(handle) {
                        n if !n.is_empty() => n.to_string(),
                        _ => "untitled".to_string(),
                    };
                    (name, b.word_set())
                })
                .collect()
        } else {
            vec![]
        };
        match self.active_body() {
            Some(b) => {
                b.other_words = others;
                b.key(k, now, cx)
            }
            None => BodyAction::None,
        }
    }

    fn mouse_down(
        &mut self,
        local: Point,
        button: gpui::MouseButton,
        modifiers: &gpui::Modifiers,
        clicks: usize,
    ) -> BodyAction {
        if let Some(hit) = self.strip.then(|| self.hit(local)).flatten() {
            if button == gpui::MouseButton::Left {
                // The press switches; held and moved, it drags (#682).
                if let crate::body::TabClick::Switch(i) = hit {
                    self.tab_drag = Some(TabDrag::new(i, local.x));
                }
                return BodyAction::BrowserTab(hit);
            }
            // Middle-click closes a tab, as in every browser.
            if let (gpui::MouseButton::Middle, crate::body::TabClick::Switch(i)) = (button, hit) {
                return BodyAction::BrowserTab(crate::body::TabClick::Close(i));
            }
            return BodyAction::None;
        }
        let below = self.below_strip(local);
        // A click into the text or the tree locks, so the arrows reach the
        // tree after a click in it (#776); grabbing the divider does not.
        let on_divider = self.active_body_ref().is_some_and(|b| b.on_divider(below));
        if !on_divider && button == gpui::MouseButton::Left && !self.scratch_hidden() {
            self.locked = true;
            self.dirty = true;
        }
        match self.active_body() {
            Some(b) => b.mouse_down(below, button, modifiers, clicks),
            None => BodyAction::None,
        }
    }

    fn mouse_up(&mut self, local: Point, button: gpui::MouseButton, modifiers: &gpui::Modifiers) {
        if let Some(drag) = self.tab_drag.take() {
            let n = self.labels().len();
            self.tab_drop = drag.drop(self.style.font_px, self.ui_scale, n);
            self.dirty |= drag.moving;
            return;
        }
        let below = self.below_strip(local);
        if let Some(b) = self.active_body() {
            b.mouse_up(below, button, modifiers);
        }
    }

    fn mouse_move(&mut self, local: Point, modifiers: &gpui::Modifiers) {
        if let Some(drag) = self.tab_drag.as_mut() {
            if drag.moved(local.x, self.style.font_px, self.ui_scale) {
                self.dirty = true;
            }
            return;
        }
        let over = self.strip.then(|| hovered(self.hit(local))).flatten();
        self.set_hover(over);
        let below = self.below_strip(local);
        if let Some(b) = self.active_body() {
            b.mouse_move(below, modifiers);
        }
    }

    fn mouse_leave(&mut self) {
        self.set_hover(None);
        if let Some(b) = self.active_body() {
            b.mouse_leave();
        }
    }

    fn wheel(&mut self, local: Point, dx: f64, dy: f64, modifiers: &gpui::Modifiers) {
        let below = self.below_strip(local);
        if let Some(b) = self.active_body() {
            b.wheel(below, dx, dy, modifiers);
        }
    }

    fn wants_frame(&self, now: f64) -> bool {
        self.dirty || self.active_body_ref().is_some_and(|b| b.wants_frame(now))
    }

    fn captures_drag(&self) -> bool {
        self.tab_drag.is_some() || self.active_body_ref().is_some_and(|b| b.captures_drag())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Whether a key pressed at an editor you are not in puts you in it: a bare
/// Enter, and nothing else (`EditorTabs::key`).
fn enters(k: &Keystroke) -> bool {
    let m = &k.modifiers;
    k.key == "enter" && !m.platform && !m.control && !m.alt && !m.shift
}

/// The folder header's text: the folder's name, no trailing slash (the tree
/// under it already says it is a folder, #694). The filesystem root has no
/// name, so it keeps its `/`.
fn folder_label(root: &str) -> String {
    match infiniterm_core::paths::base_name(root.trim_end_matches('/')) {
        "" => "/".to_string(),
        name => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_folder_header_is_the_bare_name() {
        assert_eq!(folder_label("/Users/me/infiniterm"), "infiniterm");
        assert_eq!(folder_label("/Users/me/infiniterm/"), "infiniterm");
        assert_eq!(folder_label("/"), "/");
    }

    #[test]
    fn only_a_bare_enter_gets_you_into_an_editor() {
        let k = |s: &str| gpui::Keystroke::parse(s).unwrap();
        assert!(enters(&k("enter")));
        for other in [
            "a",
            "cmd-/",
            "cmd-z",
            "shift-enter",
            "cmd-enter",
            "escape",
            "space",
        ] {
            assert!(!enters(&k(other)), "{other} must not enter the editor");
        }
    }

    use super::*;
    use gpui::FontWeight;

    fn tabs() -> EditorTabs {
        let metrics = Metrics {
            family: "Menlo".into(),
            font_px: 13.,
            line_height: 1.4,
            cell_w: 8.,
            weight: FontWeight::NORMAL,
            bold_weight: FontWeight::BOLD,
        };
        let style = StripStyle {
            font_px: 14.,
            ..crate::tab_strip::strip_style(
                &crate::chrome::Chrome::default_chrome(),
                &Default::default(),
            )
        };
        EditorTabs::new("c1", &metrics, Size { w: 800., h: 600. }, style)
    }

    // Bodies follow the card's handles by identity: closing the first
    // keeps the second's buffer (its text is proof), two untitled tabs
    // stay two, and the active index follows the card.
    #[test]
    fn tabs_follow_the_cards_handles_without_reloading_the_rest() {
        let dir = std::env::temp_dir().join(format!("ift-tabs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt").to_string_lossy().to_string();
        let b = dir.join("b.txt").to_string_lossy().to_string();
        std::fs::write(&a, "aaa").unwrap();
        std::fs::write(&b, "bbb").unwrap();
        let mut t = tabs();
        t.rebuild_tabs(&[a.clone(), b.clone()], 1, "/tmp", 0., false);
        assert_eq!(t.tab_count(), 2);
        assert_eq!(t.active_body().unwrap().buffer.text(), "bbb");
        t.active_body().unwrap().buffer.insert("!", 0.);
        // Close a: b keeps its edit.
        t.rebuild_tabs(std::slice::from_ref(&b), 0, "/tmp", 0., false);
        assert_eq!(t.tab_count(), 1);
        assert_eq!(t.active_body().unwrap().buffer.text(), "!bbb");
        // Two untitled tabs are two bodies; typing in one leaves the other.
        t.rebuild_tabs(
            &[b.clone(), String::new(), String::new()],
            2,
            "/tmp",
            0.,
            false,
        );
        assert_eq!(t.tab_count(), 3);
        t.active_body().unwrap().buffer.insert("x", 0.);
        t.rebuild_tabs(
            &[b.clone(), String::new(), String::new()],
            1,
            "/tmp",
            0.,
            false,
        );
        assert_eq!(t.active_body().unwrap().buffer.text(), "");
        assert_eq!(t.labels(), vec!["b.txt •", "untitled", "untitled •"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A move (#682) carries each body with its tab, which handles alone
    // cannot do for two untitled tabs; the active tab stays the same tab,
    // and each draft id becomes the one a restart looks for at its place.
    #[test]
    fn a_moved_tab_takes_its_own_text_and_its_places_draft_id() {
        let mut t = tabs();
        t.rebuild_tabs(
            &["/x/a.txt".into(), String::new(), String::new()],
            2,
            "/tmp",
            0.,
            false,
        );
        t.active_body().unwrap().buffer.insert("x", 0.);
        let ids = |t: &EditorTabs| -> Vec<String> {
            t.tabs.iter().map(|(_, b)| b.card_id.clone()).collect()
        };
        t.move_tab(2, 0);
        assert_eq!(t.active, 0, "still on the tab with the x");
        assert_eq!(t.active_body().unwrap().buffer.text(), "x");
        assert_eq!(t.labels(), vec!["untitled •", "a.txt", "untitled"]);
        assert_eq!(ids(&t)[0], "c1", "the first place is the card's own id");
        let mut serial = 0;
        assert_eq!(ids(&t)[1], draft_id("c1", 1, "/x/a.txt", &mut serial));
        assert_eq!(ids(&t)[2], "c1~untitled1");
        t.move_tab(0, 9);
        assert_eq!(t.active, 0, "out of range is a no-op");
    }

    // A file changing on disk reaches a clean buffer through the idle
    // loop, active tab or not; this is the path a downcast to the wrong
    // type silently skipped.
    #[test]
    fn idle_polls_every_tabs_file() {
        let dir = std::env::temp_dir().join(format!("ift-tabidle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt").to_string_lossy().to_string();
        let b = dir.join("b.txt").to_string_lossy().to_string();
        std::fs::write(&a, "a1").unwrap();
        std::fs::write(&b, "b1").unwrap();
        let mut t = tabs();
        t.rebuild_tabs(&[a.clone(), b.clone()], 0, "/tmp", 0., false);
        // A second on, both files change; the poll runs every two seconds.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&a, "a2").unwrap();
        std::fs::write(&b, "b2").unwrap();
        t.idle(5000.);
        assert_eq!(t.tabs[0].1.buffer.text(), "a2", "the active tab");
        assert_eq!(t.tabs[1].1.buffer.text(), "b2", "the background tab too");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Opening a file from the tree hands the keys to the new buffer, on this
    // machine's disk and a host's alike. So in a LOCKED card a bare Enter
    // that follows (the habit: Enter opens a row) is a newline in the file;
    // in a card that is not locked it only locks. Written down because it
    // looked like a joined-window bug (#570); it is the lock's rule.
    #[test]
    fn after_a_tree_open_the_keys_are_the_buffers() {
        let dir = std::env::temp_dir().join(format!("ift-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt").to_string_lossy().to_string();
        let b = dir.join("b.txt").to_string_lossy().to_string();
        std::fs::write(&a, "aaa\n").unwrap();
        std::fs::write(&b, "bbb\n").unwrap();
        let mut t = tabs();
        t.rebuild_tabs(
            std::slice::from_ref(&a),
            0,
            &dir.to_string_lossy(),
            0.,
            false,
        );
        t.active_body().unwrap().show_tree(&dir.to_string_lossy());
        t.locked = true;
        let body = t.active_body().unwrap();
        let row = body
            .tree
            .as_ref()
            .unwrap()
            .rows()
            .iter()
            .position(|r| r.entry.path == b)
            .unwrap();
        body.tree.as_mut().unwrap().cursor = row;
        body.tree_open_entry();
        let opened: Vec<_> = t.take_events();
        assert!(opened
            .iter()
            .any(|e| matches!(e, EditorEvent::OpenTab(p) if *p == b)));
        t.rebuild_tabs(
            &[a.clone(), b.clone()],
            1,
            &dir.to_string_lossy(),
            0.,
            false,
        );
        let body = t.active_body().unwrap();
        assert!(!body.tree_focused);
        assert_eq!(body.buffer.text(), "bbb\n");
        assert!(t.locked);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A click on a file in the tree of a card that is not locked locks it,
    // opens the file and leaves the keys in the tree, so Up and Down walk on
    // from that row (#776). Escape then has work: back to the text first.
    #[test]
    fn a_click_in_the_tree_locks_and_the_tree_keeps_the_keys() {
        let dir = std::env::temp_dir().join(format!("ift-click-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt").to_string_lossy().to_string();
        let b = dir.join("b.txt").to_string_lossy().to_string();
        std::fs::write(&a, "aaa\n").unwrap();
        std::fs::write(&b, "bbb\n").unwrap();
        let mut t = tabs();
        t.rebuild_tabs(
            std::slice::from_ref(&a),
            0,
            &dir.to_string_lossy(),
            0.,
            false,
        );
        let body = t.active_body().unwrap();
        body.show_tree(&dir.to_string_lossy());
        // The width a paint would set.
        body.sidebar_w = 200.;
        assert!(!t.locked);
        let row = t
            .active_body_ref()
            .unwrap()
            .tree
            .as_ref()
            .unwrap()
            .rows()
            .iter()
            .position(|r| r.entry.path == b)
            .unwrap();
        let at = (0..1000)
            .map(|y| Point {
                x: 20.,
                y: y as f64,
            })
            .find(|p| t.tree_row_at(*p) == Some(row))
            .unwrap();
        t.mouse_down(at, gpui::MouseButton::Left, &gpui::Modifiers::default(), 1);
        assert!(t.locked, "a click in the tree locks");
        assert!(t
            .take_events()
            .iter()
            .any(|e| matches!(e, EditorEvent::OpenTab(p) if *p == b)));
        t.rebuild_tabs(
            &[a.clone(), b.clone()],
            1,
            &dir.to_string_lossy(),
            0.,
            false,
        );
        let body = t.active_body_ref().unwrap();
        assert!(body.tree_focused, "the tree keeps the keys");
        assert_eq!(body.buffer.text(), "bbb\n");
        assert!(t.escape_has_work(), "Escape goes back to the text first");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A diff card is one body with no strip: nothing at the card's top is a
    // tab, a click there is the text (and locks), and the body gets the whole
    // card. It locks the editor's way, since this is the editor's wrapper.
    #[test]
    fn a_diff_has_no_strip_and_locks_like_an_editor() {
        let m = Metrics {
            family: "Menlo".into(),
            font_px: 13.,
            line_height: 1.4,
            cell_w: 8.,
            weight: FontWeight::NORMAL,
            bold_weight: FontWeight::BOLD,
        };
        let style = tabs().style.clone();
        let mut t = EditorTabs::new_diff("c1", "/tmp", &m, Size { w: 800., h: 600. }, style);
        assert_eq!(t.tab_count(), 1);
        assert_eq!(t.tab_at(Point { x: 10., y: 10. }), None);
        assert_eq!(t.text_world().h, 600., "the body gets the whole card");
        assert!(t.active_body().unwrap().is_diff());
        assert!(!t.locked);
        // Clicking where the strip would be reaches the body and locks.
        let hit = t.mouse_down(
            Point { x: 400., y: 10. },
            gpui::MouseButton::Left,
            &gpui::Modifiers::default(),
            1,
        );
        assert_ne!(
            hit,
            BodyAction::BrowserTab(crate::body::TabClick::Switch(0))
        );
        assert!(t.locked);
    }

    // Enter locks a focused card; the strip is never the text.
    #[test]
    fn enter_locks_and_the_strip_is_a_tab_click() {
        let mut t = tabs();
        t.rebuild_tabs(&[], 0, "/tmp", 0., false);
        assert!(!t.locked);
        let hit = t.mouse_down(
            Point { x: 10., y: 10. },
            gpui::MouseButton::Left,
            &gpui::Modifiers::default(),
            1,
        );
        assert_eq!(
            hit,
            BodyAction::BrowserTab(crate::body::TabClick::Switch(0))
        );
        assert!(!t.locked, "the strip does not lock");
        t.mouse_down(
            Point { x: 400., y: 200. },
            gpui::MouseButton::Left,
            &gpui::Modifiers::default(),
            1,
        );
        assert!(t.locked, "the text does");
    }
}
