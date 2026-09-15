//! The canvas, painted: the grid in screen space, the group frames, the
//! phantoms, every card's body and chrome, the hints. Port of the paint
//! halves of `Canvas.svelte`, `CardFrame.svelte`, `GroupFrame.svelte` and
//! `StateRing.svelte`.
//!
//! Two decisions from the reference that stay: the grid is computed line by
//! line from world coordinates rather than tiled, so it never drifts from
//! the cards; and every piece of chrome (border, ring, labels) is sized in
//! SCREEN pixels then divided by the scale, so a 2 px border is 2 px at 25%
//! and at 400%. Maximise paints the focused card alone, filling the view.
use crate::{now_ms, AppView, SWAP_MS};
use gpui::{fill, outline, point, px, size, App, BorderStyle, Bounds, Pixels, Window};
use infiniterm_core::agent_state::AgentState;
use infiniterm_core::card_label::split_label;
use infiniterm_core::chrome::{
    card_border_world_px, focus_ring_alpha, focus_ring_screen_px, inverse_scale,
    CARD_BORDER_SCREEN_PX,
};
use infiniterm_core::grid::{grid_line_offsets, is_grid_visible, Rect, Size};
use infiniterm_core::groups::aggregate_state;
use infiniterm_core::label_colors::{label_color, readable_on};
use infiniterm_core::saved_layout::CardKind;
use infiniterm_core::viewport::{ease_out_cubic, Viewport};

/// Cards are live but too small to read between these zooms; a big label
/// names them.
const MID_ZOOM_MIN: f64 = 0.25;
const MID_ZOOM_MAX: f64 = 0.6;

pub fn screen_rect(rect: Rect, vp: Viewport) -> Bounds<Pixels> {
    Bounds::new(
        point(
            px(((rect.x - vp.x) * vp.scale) as f32),
            px(((rect.y - vp.y) * vp.scale) as f32),
        ),
        size(
            px((rect.w * vp.scale) as f32),
            px((rect.h * vp.scale) as f32),
        ),
    )
}

impl AppView {
    /// The rect a card is drawn at: its own, or mid-glide between the old
    /// and the new after a swap or a split.
    fn drawn_rect(&self, id: &str, rect: Rect, now: f64) -> Rect {
        match self.glides.get(id) {
            Some(g) => {
                let t = ease_out_cubic((now - g.started) / SWAP_MS);
                Rect {
                    x: g.from.x + (rect.x - g.from.x) * t,
                    y: g.from.y + (rect.y - g.from.y) * t,
                    w: g.from.w + (rect.w - g.from.w) * t,
                    h: g.from.h + (rect.h - g.from.h) * t,
                }
            }
            None => rect,
        }
    }

    /// One frame: the plumbing, then the paint. Called from the canvas
    /// element's paint callback with the content area's bounds.
    pub fn frame(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let now = now_ms();
        self.frames += 1;
        if self.fps_window.elapsed().as_secs_f32() >= 1. {
            self.fps = self.frames as f32 / self.fps_window.elapsed().as_secs_f32();
            self.frames = 0;
            self.fps_window = std::time::Instant::now();
        }
        self.model.view_size = Size {
            w: f32::from(bounds.size.width) as f64,
            h: f32::from(bounds.size.height) as f64,
        };
        self.model.tick(now);
        self.drain_backend();
        self.maybe_sweep(now);
        self.animator
            .step(&mut self.model.viewport, self.model.view_size, now);
        self.model.pending_scale = self.animator.pending_scale();
        let mut seeded = self.seeded;
        self.model.seed_first_card(&mut seeded);
        self.seeded = seeded;
        self.perform_effects();
        self.start_glides(now);
        self.reconcile_bodies();
        self.reconcile_terminals(window);
        let t0 = std::time::Instant::now();
        self.feed_terminals(now, cx);
        let t1 = std::time::Instant::now();
        self.paint_world(bounds, now, window, cx);
        let t2 = std::time::Instant::now();
        self.schedule_save(now);
        // Where a frame goes, once a second, for the stress numbers.
        if std::env::var_os("INFINITERM_KEYLOG").is_some() {
            self.timing.0 += (t1 - t0).as_secs_f64() * 1000.;
            self.timing.1 += (t2 - t1).as_secs_f64() * 1000.;
            self.timing.2 += 1;
            if now - self.timing.3 >= 1000. {
                let n = self.timing.2.max(1) as f64;
                let b = crate::terminal_body::timing_take();
                eprintln!(
                    "[paint] {} frames: feed {:.1} ms, paint {:.1} ms per frame (frame {:.1}, links {:.1}, shape {:.1}, glyphs {:.1})",
                    self.timing.2,
                    self.timing.0 / n,
                    self.timing.1 / n,
                    b[0] / n,
                    b[1] / n,
                    b[2] / n,
                    b[3] / n
                );
                self.timing = (0., 0., 0, now);
            }
        }
    }

    /// Every card gets a body once; a closed card's body goes with it.
    fn reconcile_bodies(&mut self) {
        let ids: Vec<String> = self.model.cards.iter().map(|c| c.id.clone()).collect();
        self.bodies.retain(|id, _| ids.contains(id));
        for c in &self.model.cards {
            self.bodies.entry(c.id.clone()).or_insert_with(|| {
                Box::new(crate::body::Blank {
                    color: self.chrome.card_bg,
                })
            });
        }
    }

    fn paint_world(&mut self, bounds: Bounds<Pixels>, now: f64, window: &mut Window, cx: &mut App) {
        let origin = bounds.origin;
        let vp = self.model.viewport;
        let view = self.model.view_size;
        let chrome = self.chrome.clone();
        window.paint_quad(fill(bounds, chrome.canvas_bg));

        let maximized = self.model.selection.maximized && self.model.selection.focused_id.is_some();
        if maximized {
            let Some(card) = self.model.focused().cloned() else {
                return;
            };
            let focused = true;
            if let Some(body) = self.bodies.get_mut(&card.id) {
                body.paint(bounds, 1., focused, now, window, cx);
            }
            self.paint_labels(&card, bounds, 1., false, window, cx);
            return;
        }

        // The grid, one line per world coordinate, +0.5 so a 1 px stroke sits
        // on a pixel rather than straddling two.
        if is_grid_visible(vp.scale) {
            for x in grid_line_offsets(vp.x, view.w, vp.scale) {
                let sx = origin.x + px(x.round() as f32 + 0.5);
                window.paint_quad(fill(
                    Bounds::new(point(sx, origin.y), size(px(1.), bounds.size.height)),
                    chrome.grid_line,
                ));
            }
            for y in grid_line_offsets(vp.y, view.h, vp.scale) {
                let sy = origin.y + px(y.round() as f32 + 0.5);
                window.paint_quad(fill(
                    Bounds::new(point(origin.x, sy), size(bounds.size.width, px(1.))),
                    chrome.grid_line,
                ));
            }
        }

        let ws = self.model.active_workspace.clone().unwrap_or_default();
        let at = |r: Rect| -> Bounds<Pixels> {
            let b = screen_rect(r, vp);
            Bounds::new(point(origin.x + b.origin.x, origin.y + b.origin.y), b.size)
        };
        let inv = inverse_scale(vp.scale, self.model.ui_scale) as f32;
        let border_w = px((card_border_world_px(vp.scale) * vp.scale) as f32);

        // Groups render BEFORE cards so they paint behind them.
        for group in self.model.groups.clone() {
            let Some(rect) = self.model.group_frame(&group.id, &ws) else {
                continue;
            };
            let members: Vec<_> = self
                .model
                .cards
                .iter()
                .filter(|c| c.group_id.as_deref() == Some(&group.id) && c.workspace_id == ws)
                .collect();
            let state = aggregate_state(&members.iter().map(|c| c.agent).collect::<Vec<_>>());
            let active = members
                .iter()
                .any(|c| Some(&c.id) == self.model.selection.focused_id.as_ref());
            let color = match state {
                AgentState::Working => chrome.agent_working,
                AgentState::Idle => chrome.agent_idle,
                AgentState::None if active => chrome.text_faint,
                AgentState::None => chrome.group_border,
            };
            let b = at(rect);
            window.paint_quad(outline(b, color, BorderStyle::Solid).border_widths(border_w));
            // The name tab above the frame's top-left corner.
            let label_px = px(self.model.config.ui.group_label_size as f32 * inv * vp.scale as f32);
            let line = crate::text::shape(
                window,
                &group.name,
                label_px,
                &chrome.ui_font,
                chrome.group_label_fg,
            );
            let h = label_px * 1.6;
            let tab = Bounds::new(
                point(b.origin.x, b.origin.y - h),
                size(line.width + label_px, h),
            );
            window.paint_quad(fill(tab, chrome.canvas_bg));
            crate::text::paint_in(window, cx, &line, tab, label_px / 2.);
        }

        // The phantom and its company: hollow cards where one could be.
        let sel = self.model.selection.clone();
        if let Some(p) = &sel.phantom {
            let picks: Vec<_> = sel
                .slot_picks
                .iter()
                .filter(|s| s.rect.x != p.rect.x || s.rect.y != p.rect.y)
                .collect();
            for s in picks {
                self.paint_phantom(
                    at(s.rect),
                    border_w,
                    Some(s.key),
                    false,
                    inv,
                    vp.scale,
                    window,
                    cx,
                );
            }
            for q in &sel.phantom_extra {
                self.paint_phantom(at(q.rect), border_w, None, false, inv, vp.scale, window, cx);
            }
            let key = sel
                .slot_picks
                .iter()
                .find(|s| s.rect.x == p.rect.x && s.rect.y == p.rect.y)
                .map(|s| s.key);
            self.paint_phantom(at(p.rect), border_w, key, true, inv, vp.scale, window, cx);
        }

        // A live drag or resize: the cards moving, whether they overlap, and
        // the lines they align with.
        let moving: Vec<String> = match &self.gesture {
            Some(g) => match &g.kind {
                crate::GestureKind::MoveGroup(_) => {
                    g.start_rects.iter().map(|(id, _)| id.clone()).collect()
                }
                _ => vec![g.card.clone()],
            },
            None => vec![],
        };
        let overlapping = !moving.is_empty() && self.model.gesture_overlaps(&moving);

        // The cards being dragged paint LAST, over everything: a card behind
        // another while you move it is the thing the drop rule exists for.
        let mut cards: Vec<_> = self
            .model
            .cards
            .iter()
            .filter(|c| c.workspace_id == ws)
            .cloned()
            .collect();
        cards.sort_by_key(|c| moving.contains(&c.id));
        let visible = Bounds::new(origin, bounds.size);
        for card in &cards {
            let rect = self.drawn_rect(&card.id, card.rect, now);
            let b = at(rect);
            if !visible.intersects(&b) {
                continue;
            }
            let focused = sel.focused_id.as_deref() == Some(&card.id);
            let selected = sel.extra.contains(&card.id);
            if let Some(body) = self.bodies.get_mut(&card.id) {
                body.paint(b, vp.scale, focused, now, window, cx);
            }
            // The state border: agent state, else the card's resting colour.
            let state_color = match card.agent {
                _ if overlapping && moving.contains(&card.id) => chrome.remote_bg,
                AgentState::Working => chrome.agent_working,
                AgentState::Idle => chrome.agent_idle,
                AgentState::None => chrome.card_border,
            };
            window.paint_quad(outline(b, state_color, BorderStyle::Solid).border_widths(border_w));
            // The ring sits OUTSIDE the border so the border stays free for agent state.
            if focused || selected {
                let ring = px(focus_ring_screen_px(vp.scale) as f32);
                let alpha = focus_ring_alpha(vp.scale) as f32 * if focused { 1. } else { 0.5 };
                let rb = Bounds::new(
                    point(b.origin.x - ring, b.origin.y - ring),
                    size(b.size.width + ring * 2., b.size.height + ring * 2.),
                );
                window.paint_quad(
                    outline(
                        rb,
                        crate::chrome::with_alpha(chrome.focus_ring, alpha),
                        BorderStyle::Solid,
                    )
                    .border_widths(ring),
                );
            }
            let mid = vp.scale >= MID_ZOOM_MIN && vp.scale < MID_ZOOM_MAX;
            self.paint_labels(card, b, vp.scale, mid, window, cx);
            if let Some(hint) = sel.hints.get(&card.id) {
                // Big enough to read from across the canvas, over the body
                // rather than in a corner.
                let hp = px(48. * inv * vp.scale as f32);
                let line = crate::text::shape(
                    window,
                    &hint.to_string(),
                    hp,
                    &chrome.mono_font,
                    chrome.sel_fg,
                );
                let hb = Bounds::new(
                    point(
                        b.origin.x + b.size.width / 2. - line.width,
                        b.origin.y + b.size.height / 2. - hp,
                    ),
                    size(line.width * 2., hp * 2.),
                );
                window.paint_quad(fill(hb, chrome.sel_bg));
                crate::text::paint_in(window, cx, &line, hb, line.width / 2.);
            }
        }

        // Alignment guides, over everything: one screen pixel, the focus
        // ring's colour, where a moving edge or centre lines up with another
        // card's.
        if let Some(first) = moving.first().and_then(|id| self.model.card(id)) {
            let others: Vec<Rect> = cards
                .iter()
                .filter(|c| !moving.contains(&c.id))
                .map(|c| c.rect)
                .collect();
            for g in infiniterm_core::alignment::guides(first.rect, &others) {
                let sx = |x: f64| origin.x + px(((x - vp.x) * vp.scale) as f32);
                let sy = |y: f64| origin.y + px(((y - vp.y) * vp.scale) as f32);
                let line = match g.axis {
                    infiniterm_core::alignment::Axis::Vertical => Bounds::new(
                        point(sx(g.at), sy(g.from)),
                        size(px(1.), sy(g.to) - sy(g.from)),
                    ),
                    infiniterm_core::alignment::Axis::Horizontal => Bounds::new(
                        point(sx(g.from), sy(g.at)),
                        size(sx(g.to) - sx(g.from), px(1.)),
                    ),
                };
                window.paint_quad(fill(
                    line,
                    crate::chrome::with_alpha(chrome.focus_ring, 0.8),
                ));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_phantom(
        &self,
        b: Bounds<Pixels>,
        border_w: Pixels,
        key: Option<char>,
        current: bool,
        inv: f32,
        scale: f64,
        window: &mut Window,
        cx: &mut App,
    ) {
        let chrome = &self.chrome;
        let color = if current {
            chrome.focus_ring
        } else {
            chrome.phantom
        };
        window.paint_quad(outline(b, color, BorderStyle::Solid).border_widths(border_w));
        let text = match key {
            Some(k) if current => format!("{k}  or Enter"),
            Some(k) => k.to_string(),
            None => "+ New card".to_string(),
        };
        let fp = px(self.model.config.ui.card_label_size as f32 * 1.4 * inv * scale as f32);
        let line = crate::text::shape(window, &text, fp, &chrome.mono_font, color);
        let tb = Bounds::new(
            point(
                b.origin.x + b.size.width / 2. - line.width / 2.,
                b.origin.y + b.size.height / 2. - fp,
            ),
            size(line.width, fp * 2.),
        );
        crate::text::paint_in(window, cx, &line, tb, px(0.));
    }

    /// The name (top-right), the kind badges beside it, the remote badge
    /// (top-left, red) and the mid-zoom label. Pointer-inert, over the body.
    fn paint_labels(
        &self,
        card: &infiniterm_core::model::Card,
        b: Bounds<Pixels>,
        scale: f64,
        mid: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let chrome = &self.chrome;
        let inv = inverse_scale(scale, self.model.ui_scale) as f32;
        let label_px = px(self.model.config.ui.card_label_size as f32 * inv * scale as f32);
        let border = px(CARD_BORDER_SCREEN_PX as f32);
        let label = self.model.label_of(card);
        let identity = label_color(&card.id, chrome.theme.as_ref()).and_then(crate::chrome::hex);
        let (label_bg, label_fg) = match (identity, label_color(&card.id, chrome.theme.as_ref())) {
            (Some(bg), Some(hex_)) => (
                bg,
                crate::chrome::hex(readable_on(hex_)).unwrap_or(chrome.text_bright),
            ),
            _ => (chrome.control_bg, chrome.card_label_fg),
        };
        let h = label_px * 1.5;
        let mut right = b.origin.x + b.size.width - border;
        if !label.is_empty() {
            // The head ellipsises from the left, the tail never does: the last
            // segment is what tells cards apart, and half the directories
            // anyone works in are called `src`.
            let (head, tail) = split_label(&label);
            let room = b.size.width - border * 2. - label_px;
            let mut text = format!("{head}{tail}");
            let mut line = crate::text::shape(window, &text, label_px, &chrome.ui_font, label_fg);
            let mut keep = head.chars().count();
            while line.width > room && keep > 0 {
                keep /= 2;
                let cut: String = head
                    .chars()
                    .rev()
                    .take(keep)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                text = format!("\u{2026}{cut}{tail}");
                line = crate::text::shape(window, &text, label_px, &chrome.ui_font, label_fg);
            }
            let w = line.width + label_px;
            let lb = Bounds::new(point(right - w, b.origin.y + border), size(w, h));
            window.paint_quad(fill(lb, label_bg));
            crate::text::paint_in(window, cx, &line, lb, label_px / 2.);
            right -= w;
        }
        // What kind of card, as its own badge in the chrome's muted colour:
        // the name says WHICH, this says WHAT. Terminals carry no badge.
        let mut badges: Vec<String> = vec![];
        match card.kind {
            CardKind::Editor => {
                badges.push(card.language.clone().unwrap_or_else(|| "editor".into()));
                if card.read_only {
                    badges.push("read-only".into());
                }
                if card.dirty {
                    badges.push("unsaved".into());
                }
            }
            CardKind::Diff => badges.push("diff".into()),
            CardKind::Browser => badges.push("browser".into()),
            CardKind::Transcript => badges.push("transcript".into()),
            CardKind::Terminal => {}
        }
        for badge in badges {
            let line =
                crate::text::shape(window, &badge, label_px, &chrome.ui_font, chrome.text_muted);
            let w = line.width + label_px;
            let bb = Bounds::new(point(right - w, b.origin.y + border), size(w, h));
            window.paint_quad(fill(bb, chrome.badge_bg));
            crate::text::paint_in(window, cx, &line, bb, label_px / 2.);
            right -= w;
        }
        // Red: the one thing on a card that changes what a keystroke does.
        if let Some(remote) = &card.remote {
            let line =
                crate::text::shape(window, remote, label_px, &chrome.ui_font, chrome.remote_fg);
            let rb = Bounds::new(
                point(b.origin.x + border, b.origin.y + border),
                size(line.width + label_px, h),
            );
            window.paint_quad(fill(rb, chrome.remote_bg));
            crate::text::paint_in(window, cx, &line, rb, label_px / 2.);
        }
        if mid && !label.is_empty() {
            let big = px(self.model.config.ui.card_label_size as f32 * 2.4 * inv * scale as f32);
            let line = crate::text::shape(window, &label, big, &chrome.ui_font, chrome.text_bright);
            let mb = Bounds::new(
                point(
                    b.origin.x + b.size.width / 2. - line.width / 2.,
                    b.origin.y + b.size.height / 2. - big,
                ),
                size(line.width, big * 2.),
            );
            crate::text::paint_in(window, cx, &line, mb, px(0.));
        }
    }
}
