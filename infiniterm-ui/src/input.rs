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
    fn to_content(&self, position: gpui::Point<gpui::Pixels>) -> Point {
        Point {
            x: f32::from(position.x) as f64,
            y: f32::from(position.y) as f64 - crate::TITLEBAR_H as f64,
        }
    }

    pub fn hit(&self, screen: Point) -> Hit {
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
                });
            }
            Hit::CardEdge { id, edge } => {
                self.model.set_focus(Some(&id));
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
                });
            }
            Hit::CardBody { id, local } => {
                if e.modifiers.shift {
                    // The text-field rule; and it must not reach the body,
                    // where it would extend a TEXT selection.
                    self.model.extend_to(&id);
                } else {
                    let already = self.model.selection.focused_id.as_deref() == Some(&id);
                    if !already {
                        self.model.set_focus(Some(&id));
                    }
                    let (action, captures) = match self.bodies.get_mut(&id) {
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
                GestureKind::Move => {
                    let r = Rect {
                        x: g.start_rect.x + dx,
                        y: g.start_rect.y + dy,
                        ..g.start_rect
                    };
                    if let Some(c) = self.model.card_mut(&card) {
                        c.rect = r;
                        if std::env::var_os("INFINITERM_KEYLOG").is_some() {
                            eprintln!("[gesture] rect now {:?} (dx {dx} dy {dy})", c.rect);
                        }
                    }
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
                if let Some(body) = self.bodies.get_mut(&id) {
                    body.mouse_move(local, &e.modifiers);
                }
            }
            return;
        }
        if let Hit::CardBody { id, local } = self.hit(p) {
            if let Some(body) = self.bodies.get_mut(&id) {
                body.mouse_move(local, &e.modifiers);
            }
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
        let was_dragging = matches!(self.pan, Some(Pan::Dragging(_)));
        self.pan = None;
        if was_dragging {
            let v = velocity_from(&self.samples, now_ms());
            self.samples.clear();
            if self.model.config.canvas.momentum {
                self.animator.glide(v, now_ms());
            }
            return;
        }
        if let Some(g) = self.gesture.take() {
            // The drop: snapped, or put back if it landed on another card.
            let (ids, start): (Vec<String>, Vec<(String, Rect)>) = match &g.kind {
                GestureKind::MoveGroup(_) => (
                    g.start_rects.iter().map(|(id, _)| id.clone()).collect(),
                    g.start_rects.clone(),
                ),
                _ => (vec![g.card.clone()], vec![(g.card.clone(), g.start_rect)]),
            };
            self.model.end_gesture(&ids, &start);
            return;
        }
        if let Some(id) = self.body_drag.take() {
            if let Some(local) = self.local_in(&id, p) {
                if let Some(body) = self.bodies.get_mut(&id) {
                    body.mouse_up(local, e.button, &e.modifiers);
                }
            }
            return;
        }
        if let Hit::CardBody { id, local } = self.hit(p) {
            if let Some(body) = self.bodies.get_mut(&id) {
                body.mouse_up(local, e.button, &e.modifiers);
            }
        }
    }

    /// Cmd+scroll (and a trackpad pinch, which arrives as ctrl) zooms about
    /// the cursor: the world point under the pointer must not move.
    pub fn wheel(&mut self, e: &ScrollWheelEvent) {
        let p = self.to_content(e.position);
        let (dx, dy) = match e.delta {
            ScrollDelta::Pixels(d) => (f32::from(d.x) as f64, f32::from(d.y) as f64),
            ScrollDelta::Lines(l) => (l.x as f64 * 20., l.y as f64 * 20.),
        };
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
            if let Some(body) = self.bodies.get_mut(&id) {
                body.wheel(local, dx / scale, dy / scale, &e.modifiers);
            }
            self.flush_writes();
        }
    }

    /// A key: a bare key to a mode that owns one, else a chord to the keymap,
    /// else the palette or prompt if open, else the focused card's body.
    pub fn key_down(&mut self, e: &KeyDownEvent, cx: &mut gpui::App) {
        let k: &Keystroke = &e.keystroke;
        let m = &k.modifiers;
        if std::env::var_os("INFINITERM_KEYLOG").is_some() {
            eprintln!("[key] {k:?}");
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
                return;
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
        if (m.platform || m.control) && handle_chord(&mut self.model, &self.registry, &chord) {
            self.perform_effects();
            return;
        }
        if self.model.palette_open() {
            self.palette_key(k, cx);
            return;
        }
        if self.model.prompt.is_open() {
            self.prompt_key(k, cx);
            return;
        }
        if self.model.shortcuts_open {
            self.shortcuts_key(k, cx);
            return;
        }
        if let Some(id) = self.model.selection.focused_id.clone() {
            let action = match self.bodies.get_mut(&id) {
                Some(body) => body.key(k, now_ms(), cx),
                None => crate::body::BodyAction::None,
            };
            self.body_action(&id, action);
            self.flush_writes();
            self.perform_effects();
        }
    }

    /// What a body asked for from a click: a card beside it, or the system.
    fn body_action(&mut self, id: &str, action: crate::body::BodyAction) {
        match action {
            crate::body::BodyAction::None => {}
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
        }
    }
}
