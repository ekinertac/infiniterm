//! gpui events into model calls. Port of the input halves of `Canvas.svelte`
//! and `CardFrame.svelte` and the key handler of `App.svelte`.
//!
//! The rules, unchanged: Cmd owns every app binding and every canvas
//! gesture (Cmd+scroll zooms, Cmd+drag or middle-drag pans); a bare scroll
//! and a bare drag belong to the card under the cursor. A Cmd+left press is
//! PENDING and becomes a pan only after `DRAG_SLOP`, so a Cmd+click still
//! reaches the card as a click. A card's TOP edge moves it, every other edge
//! and corner resizes it, the body belongs to the card. Shift+click extends
//! the selection and never reaches the body. A press that misses every card
//! deselects. A bare key reaches the app only through `handle_bare_key`.
use crate::{now_ms, AppView, Gesture, GestureKind, Pan, DRAG_SLOP, EDGE_HIT};
use gpui::{
    KeyDownEvent, Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ScrollDelta, ScrollWheelEvent,
};
use infiniterm_core::grid::{snap, Point, Rect};
use infiniterm_core::keymap::{chord_for, KeyPress};
use infiniterm_core::model::focus_cmd::BareKey;
use infiniterm_core::model::register::handle_chord;
use infiniterm_core::momentum::{velocity_from, Sample, VELOCITY_SAMPLE_MS};
use infiniterm_core::pan_mode::starts_pan;
use infiniterm_core::resize::{apply_resize, Edge};
use infiniterm_core::viewport::{anchored_viewport, world_pos_of, MAX_SCALE, MIN_SCALE};

/// What is under a screen point, in hit order: handles before bodies,
/// corners before edges.
pub enum Hit {
    CardEdge { id: String, edge: Option<Edge> },
    CardBody { id: String, local: Point },
    GroupTab(String),
    Nothing,
}

impl AppView {
    /// Screen coordinates are the content area's: the title bar is above.
    pub(crate) fn to_content(&self, position: gpui::Point<gpui::Pixels>) -> Point {
        Point {
            x: f32::from(position.x) as f64,
            y: f32::from(position.y) as f64 - self.titlebar_h() as f64,
        }
    }

    pub fn hit(&self, screen: Point) -> Hit {
        // The label chip first, as the frame it is: painted over the body,
        // last painted wins.
        if let Some((id, _)) = self.label_hits.iter().rev().find(|(_, r)| {
            screen.x >= r.x && screen.x <= r.x + r.w && screen.y >= r.y && screen.y <= r.y + r.h
        }) {
            return Hit::CardEdge {
                id: id.clone(),
                edge: None,
            };
        }
        let vp = self.model.viewport;
        let world = world_pos_of(screen, vp);
        let band = EDGE_HIT / vp.scale;
        let ws = self.model.active_workspace.clone().unwrap_or_default();
        // Later cards paint over earlier ones, so the last hit wins.
        for c in self
            .model
            .cards
            .iter()
            .rev()
            .filter(|c| c.workspace_id == ws)
        {
            let r = c.rect;
            let inside = |pad: f64| {
                world.x >= r.x - pad
                    && world.x < r.x + r.w + pad
                    && world.y >= r.y - pad
                    && world.y < r.y + r.h + pad
            };
            if !inside(band / 2.) {
                continue;
            }
            if std::env::var_os("INFINITERM_KEYLOG").is_some() {
                eprintln!("[hit] screen={screen:?} world={world:?} r={r:?} band={band} vp={vp:?}");
            }
            let n = world.y < r.y + band;
            let s = world.y > r.y + r.h - band;
            let w = world.x < r.x + band;
            let e = world.x > r.x + r.w - band;
            let edge = match (n, s, w, e) {
                (true, _, true, _) => Some(Edge::Nw),
                (true, _, _, true) => Some(Edge::Ne),
                (_, true, true, _) => Some(Edge::Sw),
                (_, true, _, true) => Some(Edge::Se),
                (true, ..) => None, // the top edge moves
                (_, true, ..) => Some(Edge::S),
                (_, _, true, _) => Some(Edge::W),
                (_, _, _, true) => Some(Edge::E),
                _ => {
                    return Hit::CardBody {
                        id: c.id.clone(),
                        local: Point {
                            x: world.x - r.x,
                            y: world.y - r.y,
                        },
                    }
                }
            };
            return Hit::CardEdge {
                id: c.id.clone(),
                edge,
            };
        }
        // A group's name tab, above its frame's top-left corner.
        let tab_h = self.model.config.ui.group_label_size * 1.6 / vp.scale;
        for g in &self.model.groups {
            if let Some(f) = self.model.group_frame(&g.id, &ws) {
                if world.x >= f.x
                    && world.x < f.x + f.w.min(300. / vp.scale)
                    && world.y < f.y
                    && world.y >= f.y - tab_h
                {
                    return Hit::GroupTab(g.id.clone());
                }
            }
        }
        Hit::Nothing
    }

    pub fn mouse_down(&mut self, e: &MouseDownEvent) {
        let p = self.to_content(e.position);
        self.mouse = p;
        let button = match e.button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            // Never pans (`starts_pan` only fires for 0 and 1): a right
            // click still needs the hit test below, to reach the browser
            // card and let CEF ask for its slim menu.
            MouseButton::Right => 2,
            _ => return,
        };
        if starts_pan(button, e.modifiers.platform) {
            // Grabbing the canvas mid-glide must stop it dead.
            self.animator.cancel();
            self.pan = Some(if button == 1 {
                Pan::Dragging(p)
            } else {
                Pan::Pending(p)
            });
            if button == 1 {
                self.begin_pan(p);
            }
            return;
        }
        match self.hit(p) {
            Hit::Nothing => {
                // A double-click on bare canvas fits everything, Cmd+2 for
                // the mouse, the way a double-click on a frame is Cmd+1.
                if e.click_count >= 2 {
                    self.run_command("canvas.zoom.fitAll");
                    self.perform_effects();
                    return;
                }
                // A left press that missed every card deselects. It does not
                // pan: a plain drag means text selection on a card.
                self.model.set_focus(None);
                self.model.selection.phantom = None;
                self.animator.cancel();
            }
            Hit::GroupTab(group) => {
                let first = self
                    .model
                    .cards
                    .iter()
                    .find(|c| c.group_id.as_deref() == Some(&group))
                    .map(|c| c.id.clone());
                let Some(first) = first else { return };
                self.model.set_focus(Some(&first));
                let start_rects = self
                    .model
                    .cards
                    .iter()
                    .filter(|c| c.group_id.as_deref() == Some(&group))
                    .map(|c| (c.id.clone(), c.rect))
                    .collect();
                let start_rect = self.model.card(&first).map(|c| c.rect).unwrap_or_default();
                self.gesture = Some(Gesture {
                    card: first,
                    kind: GestureKind::MoveGroup(group),
                    start_px: p,
                    start_rect,
                    start_rects,
                    ghost: None,
                });
            }
            Hit::CardEdge { id, edge } => {
                self.model.set_focus(Some(&id));
                // A double-click on the frame fits the card, which is Cmd+1
                // for the mouse. The frame and not the body: a double-click
                // in the body belongs to the program (a word selection in a
                // terminal), and the frame is the one part of a card that
                // is ours and never the program's.
                if e.click_count >= 2 {
                    self.run_command("canvas.zoom.fitCard");
                    self.perform_effects();
                    return;
                }
                let start_rect = self.model.card(&id).map(|c| c.rect).unwrap_or_default();
                let kind = match edge {
                    None => GestureKind::Move,
                    Some(edge) => GestureKind::Resize(edge),
                };
                if std::env::var_os("INFINITERM_KEYLOG").is_some() {
                    eprintln!("[gesture] down edge={edge:?} start_rect={start_rect:?}");
                }
                self.gesture = Some(Gesture {
                    card: id,
                    kind,
                    start_px: p,
                    start_rect,
                    start_rects: vec![],
                    ghost: None,
                });
            }
            Hit::CardBody { id, local } => {
                let already = self.model.selection.focused_id.as_deref() == Some(&id);
                // Shift+click on ANOTHER card adds it to the card selection
                // (the text-field rule). On the focused card it reaches the
                // body, where Shift+click extends the TEXT selection, the
                // same rule one level down; a text selection can only live
                // in the focused card, so the two never compete.
                if e.modifiers.shift && !already {
                    self.model.extend_to(&id);
                } else {
                    if !already {
                        self.model.set_focus(Some(&id));
                        // Revealed on the RELEASE, if the press turns out to
                        // have been a click: a card clipped by the window's
                        // edge came into focus and stayed clipped, and Cmd+1
                        // was the only way to see all of it. Not on the
                        // press, or a drag to select text would have the
                        // card slide under the pointer as it went.
                        self.reveal_on_release = Some(p);
                    }
                    let (action, captures) = match self.live_body(&id) {
                        Some(body) => (
                            body.mouse_down(local, e.button, &e.modifiers, e.click_count),
                            body.captures_drag(),
                        ),
                        None => (crate::body::BodyAction::None, false),
                    };
                    if captures {
                        self.body_drag = Some(id.clone());
                    }
                    self.body_action(&id, action);
                }
            }
        }
        self.perform_effects();
    }

    fn begin_pan(&mut self, p: Point) {
        self.pan = Some(Pan::Dragging(p));
        self.samples = vec![Sample {
            x: p.x,
            y: p.y,
            t: now_ms(),
        }];
        self.model.framing = false;
    }

    pub fn mouse_move(&mut self, e: &MouseMoveEvent) {
        let p = self.to_content(e.position);
        self.mouse = p;
        match self.pan {
            Some(Pan::Pending(start)) => {
                if (p.x - start.x).hypot(p.y - start.y) >= DRAG_SLOP {
                    self.begin_pan(start);
                    self.drag_pan(p);
                }
                return;
            }
            Some(Pan::Dragging(_)) => {
                self.drag_pan(p);
                return;
            }
            None => {}
        }
        if let Some(g) = &self.gesture {
            if std::env::var_os("INFINITERM_KEYLOG").is_some() {
                eprintln!(
                    "[gesture] move p={:?} start={:?} scale={}",
                    p, g.start_px, self.model.viewport.scale
                );
            }
            // Screen delta / scale = world delta, snapped so the card steps
            // from grid line to grid line while you drag.
            let scale = self.model.viewport.scale;
            let dx = snap((p.x - g.start_px.x) / scale);
            let dy = snap((p.y - g.start_px.y) / scale);
            let card = g.card.clone();
            match &g.kind {
                // The ghost follows, snapped to the slots around it; the
                // card waits for the drop.
                GestureKind::Move => {
                    let free = Rect {
                        x: g.start_rect.x + dx,
                        y: g.start_rect.y + dy,
                        ..g.start_rect
                    };
                    let r = self.model.snap_ghost(&card, free);
                    if std::env::var_os("INFINITERM_KEYLOG").is_some() {
                        eprintln!("[gesture] ghost now {r:?} (dx {dx} dy {dy})");
                    }
                    if let Some(g) = self.gesture.as_mut() {
                        g.ghost = Some(r);
                    }
                    self.redraw = true;
                }
                GestureKind::Resize(edge) => {
                    let r = apply_resize(g.start_rect, *edge, dx, dy);
                    if let Some(c) = self.model.card_mut(&card) {
                        c.rect = r;
                    }
                }
                GestureKind::MoveGroup(_) => {
                    let moves: Vec<(String, Rect)> = g
                        .start_rects
                        .iter()
                        .map(|(id, r)| {
                            (
                                id.clone(),
                                Rect {
                                    x: r.x + dx,
                                    y: r.y + dy,
                                    ..*r
                                },
                            )
                        })
                        .collect();
                    for (id, r) in moves {
                        if let Some(c) = self.model.card_mut(&id) {
                            c.rect = r;
                        }
                    }
                }
            }
            self.model.dirty_layout = true;
            return;
        }
        // A selection or a program's drag follows the pointer past the card.
        if let Some(id) = self.body_drag.clone() {
            if let Some(local) = self.local_in(&id, p) {
                if let Some(body) = self.live_body(&id) {
                    body.mouse_move(local, &e.modifiers);
                }
            }
            return;
        }
        let hit_id = if let Hit::CardBody { id, local } = self.hit(p) {
            if let Some(body) = self.live_body(&id) {
                body.mouse_move(local, &e.modifiers);
            }
            Some(id)
        } else {
            None
        };
        // A body that cares about hover (the browser) needs telling when the
        // pointer moves off it, the way a real mouseout would.
        if self.hover_body != hit_id {
            if let Some(left) = self.hover_body.take().and_then(|id| self.live_body(&id)) {
                left.mouse_leave();
            }
            self.hover_body = hit_id;
        }
    }

    /// `screen` as card pixels of `id`, wherever the pointer is.
    fn local_in(&self, id: &str, screen: Point) -> Option<Point> {
        let r = self.model.card(id)?.rect;
        let world = world_pos_of(screen, self.model.viewport);
        Some(Point {
            x: world.x - r.x,
            y: world.y - r.y,
        })
    }

    fn drag_pan(&mut self, p: Point) {
        let Some(Pan::Dragging(last)) = self.pan else {
            return;
        };
        let scale = self.model.viewport.scale;
        // Divide by scale so the content tracks the cursor exactly at any zoom.
        self.model.viewport.x -= (p.x - last.x) / scale;
        self.model.viewport.y -= (p.y - last.y) / scale;
        self.model.framing = false;
        self.model.dirty_layout = true;
        self.pan = Some(Pan::Dragging(p));
        let t = now_ms();
        self.samples.push(Sample { x: p.x, y: p.y, t });
        // A little more than the measurement window, so the filter has
        // something to discard without the list growing for the drag.
        let cutoff = t - VELOCITY_SAMPLE_MS * 3.;
        while self.samples.len() > 2 && self.samples[0].t < cutoff {
            self.samples.remove(0);
        }
    }

    pub fn mouse_up(&mut self, e: &MouseUpEvent) {
        let p = self.to_content(e.position);
        // A press that focused a card and did not travel was a click on
        // it: bring the whole card into view, as a keyboard focus move
        // already does. A drag (text selection) leaves the view alone.
        if let Some(start) = self.reveal_on_release.take() {
            if (p.x - start.x).hypot(p.y - start.y) < crate::DRAG_SLOP {
                self.model.reveal_focused();
                self.perform_effects();
            }
        }
        let was_dragging = matches!(self.pan, Some(Pan::Dragging(_)));
        // A Cmd+press that never moved far enough to pan was a CLICK, and the
        // body never saw the press: `starts_pan` claims every Cmd+left press
        // before the hit test, because a Cmd+drag over a card has to pan the
        // canvas. So the press is delivered here instead, which is what makes
        // Cmd+click open a link. Cmd must still be down: releasing it first
        // means a plain click, which would start a text selection.
        let was_click = matches!(self.pan, Some(Pan::Pending(_))) && e.modifiers.platform;
        self.pan = None;
        if was_click {
            if let Hit::CardBody { id, local } = self.hit(p) {
                let action = match self.live_body(&id) {
                    Some(body) => body.mouse_down(local, e.button, &e.modifiers, e.click_count),
                    None => crate::body::BodyAction::None,
                };
                self.body_action(&id, action);
                self.perform_effects();
            }
            return;
        }
        if was_dragging {
            let v = velocity_from(&self.samples, now_ms());
            self.samples.clear();
            if self.model.config.canvas.momentum {
                self.animator.glide(v, now_ms());
            }
            return;
        }
        if let Some(g) = self.gesture.take() {
            // A single card's drop: to the ghost, or a swap with the card
            // under it.
            if let (GestureKind::Move, Some(ghost)) = (&g.kind, g.ghost) {
                self.model.drop_card(&g.card, ghost);
                self.perform_effects();
                return;
            }
            // The drop: snapped, or put back if it landed on another card.
            // Remembered from the start rects, so Cmd+Z has the "before".
            let (ids, start): (Vec<String>, Vec<(String, Rect)>) = match &g.kind {
                GestureKind::MoveGroup(_) => (
                    g.start_rects.iter().map(|(id, _)| id.clone()).collect(),
                    g.start_rects.clone(),
                ),
                _ => (vec![g.card.clone()], vec![(g.card.clone(), g.start_rect)]),
            };
            if self.model.end_gesture(&ids, &start) {
                self.model.remember_layout_from(&start);
            }
            return;
        }
        if let Some(id) = self.body_drag.take() {
            if let Some(local) = self.local_in(&id, p) {
                if let Some(body) = self.live_body(&id) {
                    body.mouse_up(local, e.button, &e.modifiers);
                }
            }
            return;
        }
        if let Hit::CardBody { id, local } = self.hit(p) {
            if let Some(body) = self.live_body(&id) {
                body.mouse_up(local, e.button, &e.modifiers);
            }
        }
    }

    /// Cmd+scroll (and a trackpad pinch, which arrives as ctrl) zooms about
    /// the cursor: the world point under the pointer must not move.
    pub fn wheel(&mut self, e: &ScrollWheelEvent) {
        let keylog = std::env::var_os("INFINITERM_KEYLOG").is_some();
        // An overlay's list scrolls itself, but the event still arrives at the
        // canvas under it: a scroll in the shortcuts panel also scrolled the
        // terminal the pointer happened to be over. Nothing behind a modal
        // moves, zoom included.
        if self.model.overlay_open() || self.context_menu.is_some() {
            if keylog {
                eprintln!("[wheel] {:?} dropped: overlay open", e.delta);
            }
            return;
        }
        let p = self.to_content(e.position);
        let (dx, dy) = match e.delta {
            ScrollDelta::Pixels(d) => (f32::from(d.x) as f64, f32::from(d.y) as f64),
            ScrollDelta::Lines(l) => (l.x as f64 * 20., l.y as f64 * 20.),
        };
        if keylog {
            // Where a wheel event goes is a chain of four decisions and a
            // mouse could not scroll a terminal once with nothing to say
            // which link failed. The delta's KIND is the first thing to
            // know: a trackpad sends pixels, a mouse wheel sends lines.
            eprintln!(
                "[wheel] {:?} -> dy {dy:.2} at {p:?} hit {:?}",
                e.delta,
                match self.hit(p) {
                    Hit::CardBody { id, .. } => format!("body {}", &id[..8.min(id.len())]),
                    Hit::CardEdge { .. } => "card edge".to_string(),
                    _ => "not a card".to_string(),
                }
            );
        }
        if e.modifiers.platform || e.modifiers.control {
            self.animator.cancel();
            let sensitivity = self.model.config.canvas.zoom_sensitivity;
            // gpui's delta is positive when scrolling up, the DOM's negative.
            let factor = (dy * 0.002 * sensitivity).exp();
            let vp = self.model.viewport;
            let next = (vp.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
            self.model.viewport = anchored_viewport(world_pos_of(p, vp), p, next);
            self.model.framing = false;
            self.model.dirty_layout = true;
            return;
        }
        if let Hit::CardBody { id, local } = self.hit(p) {
            let scale = self.model.viewport.scale;
            if let Some(body) = self.live_body(&id) {
                body.wheel(local, dx / scale, dy / scale, &e.modifiers);
            }
            self.flush_writes();
        }
    }

    /// A key: a bare key to a mode that owns one, else a chord to the keymap,
    /// else the palette or prompt if open, else the focused card's body.
    /// Returns whether the key was taken by a chord. macOS delivers a Cmd
    /// key twice, first as a key equivalent and then as a key down if the
    /// first was not marked handled; gpui drops the second only when the
    /// two parse identically, which Cmd+= on a non-US layout does not, so
    /// the caller must stop propagation on a handled chord.
    pub fn key_down(&mut self, e: &KeyDownEvent, cx: &mut gpui::App) -> bool {
        let k: &Keystroke = &e.keystroke;
        let m = &k.modifiers;
        if std::env::var_os("INFINITERM_KEYLOG").is_some() {
            eprintln!("[key] {k:?} dead={}", crate::keycode::last_dead());
        }
        // A dead key (Option+E on a US layout, waiting for its vowel) is
        // nobody's to take: taking it tells macOS it was handled and the
        // composition never happens. gpui reports it as the standalone
        // accent, which every body and field would happily type, so the
        // NSEvent's own word is used and the key is left for the input
        // context. The composed letter comes back through ime.rs.
        if crate::keycode::last_dead() && !m.platform && !m.control {
            return false;
        }
        // Cmd+F is in `browser_override`'s always-on list, so it opens the
        // app's find bar over a LOCKED browser card too (lock only gates
        // the tab-strip chords, `browser_keys::lock_override`'s list).
        // Without this guard a double-Escape meant for the find bar would
        // be claimed here first and unlock the card instead, leaving the
        // bar open and unresponsive. `find.open` is checked on its own
        // because it deliberately does not join `overlay_open()` (see
        // `overlays.rs::render_find_bar`).
        if k.key == "escape"
            && !m.platform
            && !m.control
            && !m.alt
            && !self.model.overlay_open()
            && !self.model.find.open
        {
            if let Some(id) = self.model.selection.focused_id.clone() {
                if self.model.card(&id).is_some_and(|c| c.locked) {
                    let now = now_ms();
                    let step = |last: &mut Option<f64>| {
                        let was = infiniterm_core::browser_keys::is_double_escape(*last, now);
                        *last = if was { None } else { Some(now) };
                        was
                    };
                    // The same double-Escape for a locked editor card as
                    // for a browser: whichever body the card has.
                    let double = match self.browser_for(&id) {
                        Some(b) => step(&mut b.last_escape_ms),
                        None => self
                            .editor_tabs_for(&id)
                            .is_some_and(|t| step(&mut t.last_escape_ms)),
                    };
                    if double {
                        if let Some(body) = self.browser_for(&id) {
                            body.set_focus(false);
                        }
                        if let Some(t) = self.editor_tabs_for(&id) {
                            t.locked = false;
                            t.mark_dirty();
                        }
                        if let Some(c) = self.model.card_mut(&id) {
                            c.locked = false;
                        }
                        return true;
                    }
                    // A single Escape while locked reaches the page, same
                    // as real Chrome (closes an autocomplete, exits
                    // fullscreen). Falls through to the normal body.key()
                    // dispatch below by NOT returning here.
                }
            }
        }
        if !m.platform && !m.control && !m.alt {
            let bare = match k.key.as_str() {
                "enter" => BareKey::Enter,
                "escape" => BareKey::Escape,
                _ => match k
                    .key_char
                    .as_deref()
                    .and_then(|s| s.chars().next())
                    .filter(|c| !c.is_control())
                {
                    Some(c)
                        if k.key_char
                            .as_deref()
                            .is_some_and(|s| s.chars().count() == 1) =>
                    {
                        BareKey::Char(c)
                    }
                    _ => BareKey::Other,
                },
            };
            if self.model.handle_bare_key(bare) {
                self.perform_effects();
                return false;
            }
        }
        // The physical key and the real Shift state from the NSEvent, so
        // the keymap is layout-proof; gpui's keystroke has neither.
        let code = crate::keycode::last_code();
        let press = KeyPress {
            key: &k.key,
            code,
            cmd: m.platform,
            ctrl: m.control,
            alt: m.alt,
            shift: if code.is_some() {
                crate::keycode::last_shift()
            } else {
                m.shift
            },
        };
        let chord = chord_for(&press);
        if std::env::var_os("INFINITERM_KEYLOG").is_some() {
            eprintln!("[chord] {chord} (key {:?}, code {:?})", k.key, code);
        }
        // Escape mid-drag: the ghost goes, the card never moved; a group
        // drag or a resize goes back to where it started.
        if k.key == "escape" && !m.platform && !m.control && !m.alt {
            if let Some(g) = self.gesture.take() {
                for (id, r) in &g.start_rects {
                    if let Some(c) = self.model.card_mut(id) {
                        c.rect = *r;
                    }
                }
                if matches!(g.kind, GestureKind::Resize(_)) {
                    if let Some(c) = self.model.card_mut(&g.card) {
                        c.rect = g.start_rect;
                    }
                }
                self.redraw = true;
                return true;
            }
        }
        // Cmd+Alt+Arrow on an editor card first tries to move between the
        // text and the tree; only at the card's edge does it go on to the
        // next card, like every other Cmd+Alt+Arrow.
        if m.platform && m.alt && !m.control && !m.shift {
            let dir = match k.key.as_str() {
                "left" => Some(infiniterm_core::navigate::Direction::Left),
                "right" => Some(infiniterm_core::navigate::Direction::Right),
                "up" => Some(infiniterm_core::navigate::Direction::Up),
                "down" => Some(infiniterm_core::navigate::Direction::Down),
                _ => None,
            };
            if let (Some(dir), Some(id)) = (dir, self.model.selection.focused_id.clone()) {
                if self
                    .editor_for(&id)
                    .is_some_and(|b| b.move_focus_within(dir))
                {
                    self.redraw = true;
                    return true;
                }
            }
        }
        if (m.platform || m.control) && handle_chord(&mut self.model, &self.registry, &chord) {
            self.perform_effects();
            return true;
        }
        // The slim menu takes no chord of its own: Escape dismisses it,
        // everything else is swallowed so it cannot reach the page under it
        // while the menu is up.
        if self.context_menu.is_some() {
            if k.key == "escape" {
                self.context_menu = None;
            }
            return true;
        }
        if self.model.palette_open() {
            return self.palette_key(k, cx);
        }
        if self.model.omni.open {
            return self.omni_key(k, cx);
        }
        if self.model.find.open {
            return self.find_key(k, cx);
        }
        if self.model.prompt.is_open() {
            return self.prompt_key(k, cx);
        }
        if self.model.shortcuts_open {
            return self.shortcuts_key(k, cx);
        }
        if let Some(id) = self.model.selection.focused_id.clone() {
            // A masked card takes no keys: Enter or Escape lifts the mask,
            // anything else is swallowed so it reaches neither the decoy
            // nor the shell under it. The chords above still work.
            if self.model.card(&id).is_some_and(|c| c.masked) {
                if matches!(k.key.as_str(), "enter" | "escape") {
                    self.model.set_mask(&id, false);
                    self.redraw = true;
                }
                return true;
            }
            let action = match self.live_body(&id) {
                Some(body) => body.key(k, now_ms(), cx),
                None => crate::body::BodyAction::None,
            };
            // A key the body took is HANDLED, so macOS does not also hand
            // it to the input context, which would type it a second time
            // through `replace_text_in_range` now that there is an input
            // handler. A key the body ignored (a dead key on its own) is
            // not, so the input context gets it and a composition begins.
            let taken = !matches!(action, crate::body::BodyAction::Ignored);
            self.body_action(&id, action);
            self.flush_writes();
            self.perform_effects();
            return taken;
        }
        false
    }

    /// The body input may reach: none while the card is masked, so a click,
    /// a wheel, a paste or a composed character cannot get under the decoy
    /// (`decoys.rs`). Everything in this file and ime.rs goes through here.
    pub fn live_body(&mut self, id: &str) -> Option<&mut Box<dyn crate::body::CardBody>> {
        if self.model.card(id).is_some_and(|c| c.masked) {
            return None;
        }
        self.bodies.get_mut(id)
    }

    /// What a body asked for from a click: a card beside it, or the system.
    fn body_action(&mut self, id: &str, action: crate::body::BodyAction) {
        match action {
            crate::body::BodyAction::None | crate::body::BodyAction::Ignored => {}
            crate::body::BodyAction::Retry => {
                // Clearing the error is what makes the reconcile spawn again.
                if let Some(t) = self.bodies.get_mut(id).and_then(|b| {
                    b.as_any_mut()
                        .downcast_mut::<crate::terminal_body::TerminalBody>()
                }) {
                    t.error = None;
                    t.mark_dirty();
                }
                if let Some(c) = self.model.card_mut(id) {
                    c.error = None;
                }
            }
            crate::body::BodyAction::Open(plan) => {
                self.model.open_in_card(plan, Some(id));
            }
            crate::body::BodyAction::OpenExternal { url, path } => {
                let result = match (url, path) {
                    (Some(url), _) => infiniterm_core::links_fs::open_url(url.as_str()),
                    (_, Some((cwd, path))) => infiniterm_core::links_fs::open_path(&cwd, &path),
                    _ => Ok(()),
                };
                if let Err(e) = result {
                    self.model.notify(format!("could not open: {e}"));
                }
            }
            // Routed through the model, not the body: `Card.tabs` is the
            // authority `browsers.rs::reconcile_browsers` syncs the body
            // from, the same commands a keyboard shortcut already calls.
            crate::body::BodyAction::BrowserTab(hit) => match hit {
                crate::body::TabClick::Switch(index) => {
                    self.model.browser_tab_jump(id, index);
                }
                crate::body::TabClick::New => {
                    self.model.browser_tab_open(id, None);
                }
                crate::body::TabClick::Close(index) => {
                    // Closes the CLICKED tab without moving the active one,
                    // matching Chrome: closing a background tab's `x` must
                    // not first jump you onto it. `browser_tab_close` (the
                    // keyboard `Cmd+W` path) stays jump-then-close-active
                    // on purpose, since Cmd+W has no index of its own.
                    self.model.browser_tab_close_at(id, index);
                }
            },
        }
    }
}

impl AppView {
    /// A file dragged from the Finder. What happens is the card's to decide
    /// (`infiniterm_core::drop::drop_plan`); this finds which card was under
    /// the pointer, asks the filesystem what each path is, and carries the
    /// answer out.
    pub fn file_drop(&mut self, paths: &[std::path::PathBuf], at: Point) {
        use infiniterm_core::drop::{drop_plan, DropAction};
        use infiniterm_core::ift::PathKind;
        let items: Vec<(String, PathKind)> = paths
            .iter()
            .filter_map(|p| {
                // Only what still exists: a path from a drag that has gone
                // stale would open an empty card or type a lie.
                let meta = std::fs::metadata(p).ok()?;
                Some((
                    p.to_string_lossy().into_owned(),
                    if meta.is_dir() {
                        PathKind::Directory
                    } else {
                        PathKind::File
                    },
                ))
            })
            .collect();
        let hit = self.hit(at);
        let card = match &hit {
            Hit::CardBody { id, .. } | Hit::CardEdge { id, .. } => self.model.card(id).cloned(),
            _ => None,
        };
        match drop_plan(&items, card.as_ref().map(|c| c.kind)) {
            DropAction::Type(text) => {
                let Some(card) = card else { return };
                // The drop focuses the card it landed on: the text has gone
                // into that shell and the keyboard should follow it.
                self.model.set_focus(Some(&card.id));
                if let Some(body) = self.live_body(&card.id).and_then(|b| {
                    b.as_any_mut()
                        .downcast_mut::<crate::terminal_body::TerminalBody>()
                }) {
                    body.paste_text(&text);
                }
                self.flush_writes();
            }
            DropAction::Navigate(url) => {
                let Some(card) = card else { return };
                if let Some(c) = self.model.card_mut(&card.id) {
                    c.url = Some(url);
                    self.model.dirty_layout = true;
                }
                self.model.set_focus(Some(&card.id));
            }
            DropAction::Open(plans) => {
                let from = card.as_ref().map(|c| c.id.clone());
                for plan in plans {
                    self.model.open_in_card(plan, from.as_deref());
                }
            }
            DropAction::None => {}
        }
        self.perform_effects();
    }
}
