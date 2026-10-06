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
//! Focus lock: a click into the text, or a bare Enter on a focused card,
//! locks (`locked`); `editors.rs` mirrors it onto `card.locked`, which is
//! what `handle_chord` reads to route Cmd+T/W/1..9 to the tab commands.
//! Double-Escape unlocks (`input.rs`, the browser's rule). The tree, the
//! strip and the find bar do not lock: locking is for typing.
//!
//! Drafts are keyed per tab (`tab_draft_id`): the first tab keeps the
//! card's own id so a draft from before tabs is still found.
use crate::body::{BodyAction, CardBody};
use crate::editor_body::{EditorBody, EditorEvent};
use crate::tab_strip::{paint_strip, strip_hit, strip_world_h, StripStyle};
use crate::terminal_body::Metrics;
use gpui::{point, size, App, Bounds, Keystroke, Pixels, Window};
use infiniterm_core::grid::{Point, Size};

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
    pub style: StripStyle,
    /// Bounds of the last paint, for the strip's hit test.
    world: Size,
    dirty: bool,
    metrics: Metrics,
}

impl EditorTabs {
    /// Escape still has something to close in the active tab (a popup, the
    /// find panel, extra cursors, a selection); see `EditorBody::escape_has_work`.
    pub fn escape_has_work(&self) -> bool {
        self.active_body_ref().is_some_and(|b| b.escape_has_work())
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
            style,
            world,
            dirty: true,
            metrics: metrics.clone(),
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
        if index == 0 {
            return self.card_id.clone();
        }
        if handle.is_empty() {
            self.untitled_serial += 1;
            return format!("{}~untitled{}", self.card_id, self.untitled_serial);
        }
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        handle.hash(&mut h);
        format!("{}~{:x}", self.card_id, h.finish())
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
            h: (self.world.h - strip_world_h(self.style.font_px, self.ui_scale)).max(1.),
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

    /// A tab's label: the file's name, `untitled`, with a mark when dirty.
    fn labels(&self) -> Vec<String> {
        self.tabs
            .iter()
            .map(|(h, b)| {
                let name = if h.is_empty() {
                    "untitled".to_string()
                } else {
                    h.rsplit('/').next().unwrap_or(h).to_string()
                };
                if b.is_dirty() {
                    format!("{name} •")
                } else {
                    name
                }
            })
            .collect()
    }

    fn below_strip(&self, local: Point) -> Point {
        Point {
            x: local.x,
            y: local.y - strip_world_h(self.style.font_px, self.ui_scale),
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
        let labels = self.labels();
        let strip_h = paint_strip(
            bounds,
            scale,
            self.ui_scale,
            &labels,
            self.active,
            self.card_number,
            self.protected,
            &self.style,
            window,
            cx,
        );
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
            if enters(k) {
                self.locked = true;
                self.dirty = true;
            }
            return BodyAction::None;
        }
        if k.key != "escape" {
            self.last_escape_ms = None;
        }
        match self.active_body() {
            Some(b) => b.key(k, now, cx),
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
        if let Some(hit) = strip_hit(local, self.style.font_px, self.ui_scale, self.tabs.len()) {
            if button == gpui::MouseButton::Left {
                return BodyAction::BrowserTab(hit);
            }
            return BodyAction::None;
        }
        let below = self.below_strip(local);
        // A click into the text locks; the tree and the find bar do not.
        let on_tree = self.active_body_ref().is_some_and(|b| b.is_on_tree(below));
        if !on_tree && button == gpui::MouseButton::Left {
            self.locked = true;
            self.dirty = true;
        }
        match self.active_body() {
            Some(b) => b.mouse_down(below, button, modifiers, clicks),
            None => BodyAction::None,
        }
    }

    fn mouse_up(&mut self, local: Point, button: gpui::MouseButton, modifiers: &gpui::Modifiers) {
        let below = self.below_strip(local);
        if let Some(b) = self.active_body() {
            b.mouse_up(below, button, modifiers);
        }
    }

    fn mouse_move(&mut self, local: Point, modifiers: &gpui::Modifiers) {
        let below = self.below_strip(local);
        if let Some(b) = self.active_body() {
            b.mouse_move(below, modifiers);
        }
    }

    fn mouse_leave(&mut self) {
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
        self.active_body_ref().is_some_and(|b| b.captures_drag())
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

#[cfg(test)]
mod tests {
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
            bg: gpui::black(),
            border: gpui::black(),
            active_bg: gpui::black(),
            text_bright: gpui::white(),
            text_muted: gpui::white(),
            font_family: "Menlo".into(),
            font_px: 14.,
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
