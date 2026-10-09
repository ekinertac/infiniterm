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
    state_border_screen_px, CARD_BORDER_SCREEN_PX,
};
use infiniterm_core::grid::{grid_line_offsets, is_grid_visible, Rect, Size};
use infiniterm_core::label_colors::{label_color, readable_on};
use infiniterm_core::saved_layout::CardKind;
use infiniterm_core::viewport::{ease_out_cubic, Viewport};

/// A grid line snaps to the middle of its pixel, so a 1px stroke sits on a
/// pixel rather than straddling two.
const GRID_LINE_SNAP_OFFSET: f32 = 0.5;
/// A group's name tab is taller than its text, for a little air above the
/// frame it labels.
const GROUP_LABEL_TAB_HEIGHT_RATIO: f32 = 1.6;
/// A single card's ring when it is somehow selected without being focused.
/// A real multiple selection never uses this: see the ring in `paint_cards`,
/// where every member is drawn at one strength.
const SELECTED_UNFOCUSED_RING_ALPHA_SCALE: f32 = 0.5;
/// A slot-pick hint (the letter you press) is big enough to read from
/// across the canvas, not just up close.
const SLOT_HINT_FONT_PX: f32 = 48.;
/// A label centred vertically on a point sits in a box twice its font
/// size tall, half above the point and half below.
const CENTERED_LABEL_HEIGHT_SCALE: f32 = 2.;
/// The slot hint's badge is also widened to twice its text, for a click
/// target bigger than the letter itself.
const HINT_BOX_WIDTH_PAD_SCALE: f32 = 2.;
/// The hovered frame band's strength: a hint, under the focus ring's.
const HOVER_BAND_ALPHA: f32 = 0.35;

/// The band `EDGE_HIT` wide along the hovered edge(s) of `b`, or all four
/// for the move band (`edge` None). Centred on the border, since the hit
/// band is.
fn paint_hover_band(
    b: Bounds<Pixels>,
    edge: Option<infiniterm_core::resize::Edge>,
    band: Pixels,
    color: gpui::Hsla,
    window: &mut Window,
) {
    use infiniterm_core::resize::Edge;
    let ink = crate::chrome::with_alpha(color, HOVER_BAND_ALPHA);
    let half = band / 2.;
    let top = Bounds::new(
        point(b.origin.x - half, b.origin.y - half),
        size(b.size.width + band, band),
    );
    let bottom = Bounds::new(
        point(b.origin.x - half, b.origin.y + b.size.height - half),
        size(b.size.width + band, band),
    );
    let left = Bounds::new(
        point(b.origin.x - half, b.origin.y - half),
        size(band, b.size.height + band),
    );
    let right = Bounds::new(
        point(b.origin.x + b.size.width - half, b.origin.y - half),
        size(band, b.size.height + band),
    );
    let sides: Vec<Bounds<Pixels>> = match edge {
        None => vec![top, bottom, left, right],
        Some(Edge::N) => vec![top],
        Some(Edge::S) => vec![bottom],
        Some(Edge::E) => vec![right],
        Some(Edge::W) => vec![left],
        Some(Edge::Ne) => vec![top, right],
        Some(Edge::Nw) => vec![top, left],
        Some(Edge::Se) => vec![bottom, right],
        Some(Edge::Sw) => vec![bottom, left],
    };
    for s in sides {
        window.paint_quad(fill(s, ink));
    }
}

/// The grid's slots drawn around a dragged ghost: there to be seen, never
/// louder than the cards.
const GHOST_SLOT_ALPHA: f32 = 0.35;
/// The ghost's fill: enough to read as a slot, not enough to hide what is
/// under it.
const GHOST_FILL_ALPHA: f32 = 0.08;

/// An alignment guide is a hint, not a border: visible but not shouting.
const ALIGNMENT_GUIDE_ALPHA: f32 = 0.8;
/// A phantom's key label is larger than the corner labels, since it is the
/// one thing on an empty slot to read.
const PHANTOM_LABEL_SCALE: f32 = 1.4;
/// A corner label's badge is taller than its text, for click room.
const LABEL_BADGE_HEIGHT_RATIO: f32 = 1.5;

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
        self.frame_rate.frame(now);
        // Cleared first: anything drained below that changes the chrome sets
        // it again, and `needs_frame` is asked right after this returns.
        self.redraw = false;
        self.model.view_size = Size {
            w: f32::from(bounds.size.width) as f64,
            h: f32::from(bounds.size.height) as f64,
        };
        self.model.app_active = window.is_window_active();
        self.model.tick(now);
        self.last_paint_ms = now;
        self.note_window(window.window_bounds(), now);
        self.drain_backend();
        self.drain_find();
        if let Some(text) = self.clipboard_out.take() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
        self.maybe_sweep(now);
        self.animator
            .step(&mut self.model.viewport, self.model.view_size, now);
        self.model.pending_scale = self.animator.pending_scale();
        let mut seeded = self.seeded;
        self.model.seed_first_card(&mut seeded);
        self.seeded = seeded;
        self.perform_effects();
        if std::mem::take(&mut self.show_character_palette) {
            window.show_character_palette();
        }
        self.start_glides(now);
        self.reconcile_bodies(window);
        self.reconcile_terminals(window);
        self.reconcile_decoys(window);
        self.reconcile_editors(window);
        self.reconcile_browsers();
        let t0 = std::time::Instant::now();
        self.feed_terminals(now, cx);
        let t1 = std::time::Instant::now();
        self.paint_world(bounds, now, window, cx);
        // Marked text over the focused card's caret, while an input method
        // composes; the fields draw theirs inline.
        self.paint_composing(window, cx);
        self.apply_hover_cursor(window);
        // Over a frame band the cursor says what a press would do: arrows
        // on an edge or corner, a hand on the band that moves the card.
        if let Some((_, edge)) = self
            .hover_edge
            .as_ref()
            .filter(|_| !self.model.modal_open())
        {
            use infiniterm_core::resize::Edge;
            let style = match edge {
                None => gpui::CursorStyle::OpenHand,
                Some(Edge::N | Edge::S) => gpui::CursorStyle::ResizeUpDown,
                Some(Edge::E | Edge::W) => gpui::CursorStyle::ResizeLeftRight,
                Some(Edge::Nw | Edge::Se) => gpui::CursorStyle::ResizeUpLeftDownRight,
                Some(Edge::Ne | Edge::Sw) => gpui::CursorStyle::ResizeUpRightDownLeft,
            };
            window.set_window_cursor_style(style);
        }
        // Holding something: a card, a group, a selection, or the canvas
        // itself while a Cmd+drag or a middle-button drag pans it (#60; a
        // pan kept the arrow, as if nothing were held). A press that has
        // not moved yet is not a pan: a Cmd+click is still a link click.
        if self.gesture.is_some() || matches!(self.pan, Some(crate::Pan::Dragging(_))) {
            window.set_window_cursor_style(gpui::CursorStyle::ClosedHand);
        }
        let t2 = std::time::Instant::now();
        self.schedule_save(now);
        self.schedule_history(now);
        // Where a frame goes, once a second, for the stress numbers.
        if std::env::var_os("INFINITERM_KEYLOG").is_some() {
            self.timing.0 += (t1 - t0).as_secs_f64() * 1000.;
            self.timing.1 += (t2 - t1).as_secs_f64() * 1000.;
            self.timing.2 += 1;
            if now - self.timing.3 >= 1000. {
                let n = self.timing.2.max(1) as f64;
                let b = crate::terminal_body::timing_take();
                eprintln!(
                    "[paint] {} frames ({}): feed {:.1} ms, paint {:.1} ms per frame (frame {:.1}, links {:.1}, shape {:.1}, glyphs {:.1})",
                    self.timing.2,
                    self.frame_reason(),
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

    /// Every card gets a body once; a closed card's body goes with it. A
    /// browser body's last painted frame gets a chance to leave gpui's
    /// sprite atlas here too: nothing else evicts a dead frame's tile
    /// (gpui has no atlas LRU), so a closed card would otherwise leave it
    /// allocated forever (`browser_body::BrowserBody::drop_texture`).
    fn reconcile_bodies(&mut self, window: &mut Window) {
        let ids: Vec<String> = self.model.cards.iter().map(|c| c.id.clone()).collect();
        self.bodies.retain(|id, body| {
            if ids.contains(id) {
                return true;
            }
            if let Some(browser) = body
                .as_any_mut()
                .downcast_mut::<crate::browser_body::BrowserBody>()
            {
                browser.drop_texture(window);
            }
            false
        });
        self.body_sizes.retain(|id, _| ids.contains(id));
        for c in &self.model.cards {
            self.bodies.entry(c.id.clone()).or_insert_with(|| {
                Box::new(crate::body::Blank {
                    color: self.chrome.card_bg,
                })
            });
            // A drag, a resize, a split: the body learns its new size once.
            let size = Size {
                w: c.rect.w,
                h: c.rect.h,
            };
            if self.body_sizes.get(&c.id) != Some(&size) {
                self.body_sizes.insert(c.id.clone(), size);
                if let Some(body) = self.bodies.get_mut(&c.id) {
                    body.resized(size);
                }
            }
        }
    }

    /// `ui.cardRadius` in screen pixels at this zoom.
    fn card_radius(&self, scale: f64) -> Pixels {
        px((self.model.config.ui.card_radius * scale) as f32)
    }

    fn paint_world(&mut self, bounds: Bounds<Pixels>, now: f64, window: &mut Window, cx: &mut App) {
        self.label_hits.clear();
        // Bars off (`ui.textAsBars`): no glyph is too small to draw.
        let ui = &self.model.config.ui;
        crate::chrome::set_legible_device_px(if ui.text_as_bars {
            ui.min_text_px as f32
        } else {
            0.
        });
        let origin = bounds.origin;
        let vp = self.model.viewport;
        let view = self.model.view_size;
        let chrome = self.chrome.clone();
        // With a `ui.backgroundImage` the fill is the first layer under this
        // element (`background_show.rs`), so the pictures can sit over it.
        if !self.background.active() {
            window.paint_quad(fill(bounds, self.window_fill(chrome.canvas_bg)));
        }
        let opacity = crate::chrome::CardOpacity(self.model.config.ui.card_opacity as f32);
        if cx.try_global::<crate::chrome::CardOpacity>() != Some(&opacity) {
            cx.set_global(opacity);
        }
        // Another app is in front: a glance must say keys are going
        // elsewhere. Every card ends up washed ONCE: the canvas here, under
        // the cards; the focused card over its body, below; an unfocused
        // card already wears its own `ui.inactiveDim` scrim, and a wash over
        // the whole window on top of that dimmed those cards twice (Ekin,
        // 2026-09-24). gpui refreshes on every activation change, so this
        // needs no observer of its own.
        let away_dim = self.model.config.ui.unfocused_dim;
        let away = (away_dim > 0. && !window.is_window_active())
            .then(|| crate::chrome::with_alpha(chrome.canvas_bg, away_dim as f32));

        let maximized = self.model.selection.maximized && self.model.selection.focused_id.is_some();
        if maximized {
            let Some(card) = self.model.focused().cloned() else {
                return;
            };
            let focused = true;
            let body = if card.masked {
                self.decoys
                    .get_mut(&card.id)
                    .map(|d| d as &mut dyn crate::body::CardBody)
            } else {
                self.bodies.get_mut(&card.id).map(|b| b.as_mut())
            };
            if let Some(body) = body {
                window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                    body.paint(bounds, 1., focused, now, window, cx)
                });
            }
            if let Some(wash) = away {
                window.paint_quad(fill(bounds, wash));
            }
            self.paint_labels(&card, bounds, 1., window, cx);
            return;
        }

        // The grid, one line per world coordinate, +0.5 so a 1 px stroke sits
        // on a pixel rather than straddling two.
        if self.model.config.ui.show_grid && is_grid_visible(vp.scale) {
            let hairline = px(crate::chrome::HAIRLINE_PX as f32);
            for x in grid_line_offsets(vp.x, view.w, vp.scale) {
                let sx = origin.x + px(x.round() as f32 + GRID_LINE_SNAP_OFFSET);
                window.paint_quad(fill(
                    Bounds::new(point(sx, origin.y), size(hairline, bounds.size.height)),
                    chrome.grid_line,
                ));
            }
            for y in grid_line_offsets(vp.y, view.h, vp.scale) {
                let sy = origin.y + px(y.round() as f32 + GRID_LINE_SNAP_OFFSET);
                window.paint_quad(fill(
                    Bounds::new(point(origin.x, sy), size(bounds.size.width, hairline)),
                    chrome.grid_line,
                ));
            }
        }

        if let Some(wash) = away {
            window.paint_quad(fill(bounds, wash));
        }

        let ws = self.model.active_workspace.clone().unwrap_or_default();
        let at = |r: Rect| -> Bounds<Pixels> {
            let b = screen_rect(r, vp);
            Bounds::new(point(origin.x + b.origin.x, origin.y + b.origin.y), b.size)
        };
        let inv = inverse_scale(vp.scale, self.model.ui_scale) as f32;
        let border_w = px((card_border_world_px(vp.scale) * vp.scale) as f32);
        // Rounded cards (`ui.cardRadius`, 0 = square): every outline of a
        // card takes it, and the bodies read it from the global.
        let radius = self.card_radius(vp.scale);
        if cx.try_global::<crate::chrome::CardRadius>()
            != Some(&crate::chrome::CardRadius(f32::from(radius)))
        {
            cx.set_global(crate::chrome::CardRadius(f32::from(radius)));
        }

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
            // A frame says WHICH CARDS BELONG TOGETHER and nothing else. It
            // deliberately carries no agent state: a group holds several
            // sessions, and a frame in one colour cannot say which of them
            // wants you. Colouring it only drew the eye to the box instead
            // of to the card inside it that is asking.
            let active = members
                .iter()
                .any(|c| Some(&c.id) == self.model.selection.focused_id.as_ref());
            let color = if active {
                chrome.text_faint
            } else {
                chrome.group_border
            };
            let b = at(rect);
            window.paint_quad(
                outline(b, color, BorderStyle::Solid)
                    .border_widths(border_w)
                    .corner_radii(radius + px(self.model.group_pad() as f32 * vp.scale as f32)),
            );
            // The name tab above the frame's top-left corner.
            let label_px = px(self.model.config.ui.group_label_size as f32 * inv * vp.scale as f32);
            let line = crate::text::shape(
                window,
                &group.name,
                label_px,
                &chrome.typography.bold,
                chrome.group_label_fg,
            );
            let h = label_px * GROUP_LABEL_TAB_HEIGHT_RATIO;
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
        // the lines they align with. A single card's drag moves its GHOST
        // (`Gesture::ghost`), so the card itself is not "moving" here; the
        // ghost is drawn after the cards and the guides follow it.
        let ghost: Option<(String, Rect)> = self
            .gesture
            .as_ref()
            .and_then(|g| g.ghost.map(|r| (g.card.clone(), r)));
        let moving: Vec<String> = match &self.gesture {
            Some(g) => match &g.kind {
                crate::GestureKind::MoveGroup(_) | crate::GestureKind::MoveSelection => {
                    g.start_rects.iter().map(|(id, _)| id.clone()).collect()
                }
                _ if ghost.is_some() => vec![],
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
        // The frame's glyph budget: what every visible card together costs
        // in glyphs, counting only the part of each card the window shows
        // (the painter skips the rows outside it). Over it, every card
        // draws bars (`chrome::GLYPH_BUDGET_CELLS`), because the cost is
        // the total, not any one card's size. Decided before a body paints,
        // so every card this frame agrees.
        let on_screen: Vec<(String, f32)> = cards
            .iter()
            .filter_map(|c| {
                let b = at(self.drawn_rect(&c.id, c.rect, now));
                let shown = b.intersect(&visible);
                let area = f32::from(b.size.width) * f32::from(b.size.height);
                (visible.intersects(&b) && area > 0.).then(|| {
                    let part = f32::from(shown.size.width) * f32::from(shown.size.height);
                    (c.id.clone(), part / area)
                })
            })
            .collect();
        let cells: usize = on_screen
            .iter()
            .filter_map(|(id, part)| {
                self.bodies
                    .get(id)
                    .map(|b| (b.text_cells() as f32 * part) as usize)
            })
            .sum();
        // What the window would hold at 100%, from the densest visible
        // card: its cells per screen pixel now, times the window's area,
        // times the zoom squared (a card's grid does not change with zoom,
        // its pixels do).
        let density = on_screen
            .iter()
            .filter_map(|(id, _)| {
                let card = cards.iter().find(|c| &c.id == id)?;
                let b = at(self.drawn_rect(id, card.rect, now));
                let area = f32::from(b.size.width) * f32::from(b.size.height);
                let cells = self.bodies.get(id)?.text_cells() as f32;
                (area > 0.).then_some(cells / area)
            })
            .fold(0., f32::max);
        let window_area = f32::from(visible.size.width) * f32::from(visible.size.height);
        let scale = vp.scale as f32;
        let window_cells = (density * window_area * scale * scale) as usize;
        let crowded = self.model.config.ui.text_as_bars
            && crate::chrome::over_glyph_budget(
                cells,
                window_cells,
                self.model.config.ui.glyph_budget,
            );
        self.crowded = crowded;
        for (id, _) in &on_screen {
            if let Some(b) = self.bodies.get_mut(id) {
                b.set_crowded(crowded);
            }
        }
        for card in &cards {
            let rect = self.drawn_rect(&card.id, card.rect, now);
            let b = at(rect);
            if !visible.intersects(&b) {
                continue;
            }
            let focused = sel.focused_id.as_deref() == Some(&card.id);
            let selected = sel.extra.contains(&card.id);
            // A masked card shows its decoy in its place (decoys.rs).
            let body = if card.masked {
                self.decoys
                    .get_mut(&card.id)
                    .map(|d| d as &mut dyn crate::body::CardBody)
            } else {
                self.bodies.get_mut(&card.id).map(|b| b.as_mut())
            };
            if let Some(body) = body {
                // Clipped to the card: a long line or a wash must not
                // paint over the neighbour.
                window.with_content_mask(Some(gpui::ContentMask { bounds: b }), |window| {
                    body.paint(b, vp.scale, focused, now, window, cx)
                });
            }
            // The focused card has no scrim of its own, so the away wash is
            // laid on it here; the rest are already dimmed once.
            if let (Some(wash), true) = (away, focused) {
                window.paint_quad(fill(b, wash));
            }
            // The state border: agent state, else the card's resting colour.
            let state_color = match card.agent {
                _ if overlapping && moving.contains(&card.id) => chrome.remote_bg,
                AgentState::Working => chrome.agent_working,
                AgentState::Waiting => chrome.agent_waiting,
                AgentState::Failed => chrome.agent_failed,
                AgentState::Done => chrome.agent_done,
                AgentState::None => chrome.card_border,
            };
            // A resting card keeps the hairline; one with an agent in it gets
            // the heavier, zoom-stepped border, because at a distance the
            // area of colour is what is read rather than the colour.
            let state_w = if card.agent == AgentState::None {
                border_w
            } else {
                px(state_border_screen_px(vp.scale) as f32)
            };
            window.paint_quad(
                outline(b, state_color, BorderStyle::Solid)
                    .border_widths(state_w)
                    .corner_radii(radius),
            );
            // The frame band under the pointer lights up: the whole band
            // for a move, the one edge or the two edges of a corner for a
            // resize, in the focus ring's colour at a hint's strength.
            if let Some((hid, edge)) = &self.hover_edge {
                if *hid == card.id {
                    paint_hover_band(
                        b,
                        *edge,
                        px(crate::EDGE_HIT as f32),
                        chrome.focus_ring,
                        window,
                    );
                }
            }
            // The ring sits OUTSIDE the border so the border stays free for agent state.
            if focused || selected {
                let ring = px(focus_ring_screen_px(vp.scale) as f32);
                // One card is a FOCUS and wears the white ring. Several are
                // a SELECTION, and every member wears the same blue at the
                // same strength: halving the ring on the cards you did not
                // touch last made it unreadable which ones a command was
                // about to close, split or group.
                // The active card keeps the white ring even inside a
                // selection, so you can still see where you are and which
                // card the next extend grows from. The rest take blue, all
                // at one strength.
                let multi = !sel.extra.is_empty();
                // A locked browser card's ring says the keyboard means
                // something different than it did a keystroke ago, the
                // same fact the status bar's "locked: browser" reads off
                // `card.locked` — the ring is the one place on the card
                // itself that shows it, since nothing else changes there.
                let locked = focused
                    && matches!(card.kind, CardKind::Browser | CardKind::Editor)
                    && card.locked;
                let ring_color = if locked {
                    chrome.warn
                } else if multi && !focused {
                    chrome.selection_ring
                } else {
                    chrome.focus_ring
                };
                let alpha = focus_ring_alpha(vp.scale) as f32
                    * if focused || multi {
                        1.
                    } else {
                        SELECTED_UNFOCUSED_RING_ALPHA_SCALE
                    };
                // Off the border by a strip of canvas, so the ring keeps
                // its own edge against an agent border of any colour.
                let off = ring + ring * infiniterm_core::chrome::RING_GAP_RATIO as f32;
                let rb = Bounds::new(
                    point(b.origin.x - off, b.origin.y - off),
                    size(b.size.width + off * 2., b.size.height + off * 2.),
                );
                window.paint_quad(
                    outline(
                        rb,
                        crate::chrome::with_alpha(ring_color, alpha),
                        BorderStyle::Solid,
                    )
                    .border_widths(ring)
                    .corner_radii(radius + off),
                );
            }
            self.paint_labels(card, b, vp.scale, window, cx);
            if let Some(hint) = sel.hints.get(&card.id) {
                // Big enough to read from across the canvas, over the body
                // rather than in a corner.
                let hp = px(chrome.typography.size(SLOT_HINT_FONT_PX) * inv * vp.scale as f32);
                let line = crate::text::shape(
                    window,
                    &hint.to_string(),
                    hp,
                    &chrome.typography.bold,
                    chrome.sel_fg,
                );
                let hb = Bounds::new(
                    point(
                        b.origin.x + b.size.width / 2. - line.width,
                        b.origin.y + b.size.height / 2. - hp,
                    ),
                    size(
                        line.width * HINT_BOX_WIDTH_PAD_SCALE,
                        hp * CENTERED_LABEL_HEIGHT_SCALE,
                    ),
                );
                window.paint_quad(fill(hb, chrome.sel_bg));
                crate::text::paint_in(window, cx, &line, hb, line.width / 2.);
            }
        }

        // The ghost: an outline where the dragged card would land, in the
        // focus ring's colour when the space is free and the warning colour
        // when a card is in the way, so the answer is known before the drop.
        if let Some((id, r)) = &ghost {
            // The grid's slots for a card this size, faint, so where it
            // belongs is visible before the drop (slot_snap.rs).
            let slot_color = crate::chrome::with_alpha(chrome.focus_ring, GHOST_SLOT_ALPHA);
            for s in self.model.ghost_slots(*r) {
                window.paint_quad(
                    outline(at(s), slot_color, BorderStyle::Dashed)
                        .border_widths(px(crate::chrome::HAIRLINE_PX as f32)),
                );
            }
            let free = self
                .model
                .rect_free_for(id, infiniterm_core::grid::snap_rect(*r));
            let color = if free { chrome.focus_ring } else { chrome.warn };
            let b = at(*r);
            window.paint_quad(
                outline(b, color, BorderStyle::Solid)
                    .border_widths(border_w)
                    .corner_radii(radius),
            );
            window.paint_quad(
                fill(b, crate::chrome::with_alpha(color, GHOST_FILL_ALPHA)).corner_radii(radius),
            );
        }

        // The selection rectangle being dragged: the selection's blue, a
        // hairline and a faint fill, like the ghost.
        if let Some(mq) = self.marquee.as_ref().filter(|m| m.active) {
            let b = at(mq.rect());
            let blue = chrome.selection_ring;
            window.paint_quad(fill(b, crate::chrome::with_alpha(blue, GHOST_FILL_ALPHA)));
            window.paint_quad(
                outline(b, blue, BorderStyle::Solid)
                    .border_widths(px(crate::chrome::HAIRLINE_PX as f32)),
            );
        }

        // Alignment guides, over everything: one screen pixel, the focus
        // ring's colour, where a moving edge or centre lines up with another
        // card's (or the ghost's).
        let guided: Option<(String, Rect)> = ghost.clone().or_else(|| {
            moving
                .first()
                .and_then(|id| self.model.card(id))
                .map(|c| (c.id.clone(), c.rect))
        });
        if let Some((moving_id, moving_rect)) = guided {
            let others: Vec<Rect> = cards
                .iter()
                .filter(|c| c.id != moving_id && !moving.contains(&c.id))
                .map(|c| c.rect)
                .collect();
            let first_rect = moving_rect;
            let hairline = px(crate::chrome::HAIRLINE_PX as f32);
            for g in infiniterm_core::alignment::guides(first_rect, &others) {
                let sx = |x: f64| origin.x + px(((x - vp.x) * vp.scale) as f32);
                let sy = |y: f64| origin.y + px(((y - vp.y) * vp.scale) as f32);
                let line = match g.axis {
                    infiniterm_core::alignment::Axis::Vertical => Bounds::new(
                        point(sx(g.at), sy(g.from)),
                        size(hairline, sy(g.to) - sy(g.from)),
                    ),
                    infiniterm_core::alignment::Axis::Horizontal => Bounds::new(
                        point(sx(g.from), sy(g.at)),
                        size(sx(g.to) - sx(g.from), hairline),
                    ),
                };
                window.paint_quad(fill(
                    line,
                    crate::chrome::with_alpha(chrome.focus_ring, ALIGNMENT_GUIDE_ALPHA),
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
        window.paint_quad(
            outline(b, color, BorderStyle::Solid)
                .border_widths(border_w)
                .corner_radii(self.card_radius(scale)),
        );
        let text = match key {
            Some(k) if current => format!("{k}  or Enter"),
            Some(k) => k.to_string(),
            None => "+ New card".to_string(),
        };
        let fp = px(self.model.config.ui.card_label_size as f32
            * PHANTOM_LABEL_SCALE
            * inv
            * scale as f32);
        let line = crate::text::shape(window, &text, fp, &chrome.typography.regular, color);
        let tb = Bounds::new(
            point(
                b.origin.x + b.size.width / 2. - line.width / 2.,
                b.origin.y + b.size.height / 2. - fp,
            ),
            size(line.width, fp * CENTERED_LABEL_HEIGHT_SCALE),
        );
        crate::text::paint_in(window, cx, &line, tb, px(0.));
    }

    /// The name (top-right), the kind badges beside it, the remote badge
    /// (top-left, red) and the mid-zoom label. Pointer-inert, over the body.
    fn paint_labels(
        &mut self,
        card: &infiniterm_core::model::Card,
        b: Bounds<Pixels>,
        scale: f64,
        window: &mut Window,
        cx: &mut App,
    ) {
        // A browser or editor card wears no label: its tab strip carries
        // the title and the number, and a chip over a page covered the
        // corner of every site. The frame band still drags and fits.
        if matches!(
            card.kind,
            infiniterm_core::saved_layout::CardKind::Browser
                | infiniterm_core::saved_layout::CardKind::Editor
        ) {
            return;
        }
        let ui = &self.model.config.ui;
        // A maximised card fills the window and the status bar names it, so
        // its chips can go (#71). The ssh warning below them stays.
        let hide = ui.hide_label_when_maximised
            && self.model.selection.maximized
            && self.model.selection.focused_id.as_deref() == Some(card.id.as_str());
        let corner = ui.card_label_position;
        let chrome = &self.chrome;
        let inv = inverse_scale(scale, self.model.ui_scale) as f32;
        // Screen-sized, so it never shrinks with the canvas, and stepped
        // up as you zoom out so the name keeps its share of a card that is
        // getting smaller. This is what replaced the separate label the
        // mid zoom used to draw across the middle of the card.
        let label_px = px(self.model.config.ui.card_label_size as f32
            * infiniterm_core::chrome::corner_label_scale(scale) as f32
            * inv
            * scale as f32);
        let border = px(CARD_BORDER_SCREEN_PX as f32);
        let label = self.model.numbered_label(card);
        let identity = label_color(&card.id, chrome.theme.as_ref()).and_then(crate::chrome::hex);
        // While an agent is in the card the chip carries its STATE rather
        // than the card's identity colour. The chip is the largest piece of
        // colour on a card, and it was spending it on a hash of the card id
        // while the thing you actually scan for lived in the border: a
        // card whose identity colour happened to be yellow read as waiting.
        // The identity colour comes back when the session ends.
        let state_chip = match card.agent {
            AgentState::Working => Some(chrome.agent_working),
            AgentState::Waiting => Some(chrome.agent_waiting),
            AgentState::Failed => Some(chrome.agent_failed),
            AgentState::Done => Some(chrome.agent_done),
            AgentState::None => None,
        };
        let (label_bg, label_fg) = match (identity, label_color(&card.id, chrome.theme.as_ref())) {
            (Some(bg), Some(hex_)) => (
                bg,
                crate::chrome::hex(readable_on(hex_)).unwrap_or(chrome.text_bright),
            ),
            _ => (chrome.control_bg, chrome.card_label_fg),
        };
        // All three state colours are light; the card's own ground is the
        // ink that reads on every one of them.
        let (label_bg, label_fg) = match state_chip {
            Some(bg) => (bg, chrome.card_bg),
            None => (label_bg, label_fg),
        };
        // A protected card's chip is the warning colour: the one colour in
        // the chrome that says "not like the others", beside the lock in
        // the text.
        let (label_bg, label_fg) = if card.protected {
            (chrome.warn, chrome.card_bg)
        } else {
            (label_bg, label_fg)
        };
        let h = label_px * LABEL_BADGE_HEIGHT_RATIO;
        // The chips run inward from `ui.cardLabelPosition`'s corner (#71):
        // the name first, then the kind badges, leftward from a right corner
        // and rightward from a left one, along the top or the bottom edge.
        let top = if corner.bottom() {
            b.origin.y + b.size.height - border - h
        } else {
            b.origin.y + border
        };
        let mut edge = if corner.left() {
            b.origin.x + border
        } else {
            b.origin.x + b.size.width - border
        };
        let mut place = |w: Pixels| -> Pixels {
            if corner.left() {
                let x = edge;
                edge += w;
                x
            } else {
                edge -= w;
                edge
            }
        };
        // With rounded cards the chip that sits in the card's corner is
        // rounded to the border's inner edge there, or its square corner
        // covers the card's. Only that one corner of that one chip.
        let inner = (self.card_radius(scale) - border).max(px(0.));
        let corner_radii = |left: bool| gpui::Corners {
            top_left: if left && !corner.bottom() {
                inner
            } else {
                px(0.)
            },
            top_right: if !left && !corner.bottom() {
                inner
            } else {
                px(0.)
            },
            bottom_left: if left && corner.bottom() {
                inner
            } else {
                px(0.)
            },
            bottom_right: if !left && corner.bottom() {
                inner
            } else {
                px(0.)
            },
        };
        let mut in_corner = true;
        if !label.is_empty() && !hide {
            // The head ellipsises from the left, the tail never does: the last
            // segment is what tells cards apart, and half the directories
            // anyone works in are called `src`.
            let (head, tail) = split_label(&label);
            let room = b.size.width - border * 2. - label_px;
            let mut text = format!("{head}{tail}");
            let mut line =
                crate::text::shape(window, &text, label_px, &chrome.typography.bold, label_fg);
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
                line =
                    crate::text::shape(window, &text, label_px, &chrome.typography.bold, label_fg);
            }
            // The tail alone can still be wider than the card: a label with
            // no slash in it is ALL tail, and an agent's session name has
            // none. Before this it ran off the card and over its neighbours.
            if line.width > room {
                text = crate::text::elide(&text, f32::from(room), |t| {
                    f32::from(
                        crate::text::shape(window, t, label_px, &chrome.typography.bold, label_fg)
                            .width,
                    )
                });
                line =
                    crate::text::shape(window, &text, label_px, &chrome.typography.bold, label_fg);
            }
            let w = line.width + label_px;
            let lb = Bounds::new(point(place(w), top), size(w, h));
            // Remembered for the hit test, in the content coordinates the
            // mouse arrives in (the title bar is above the canvas).
            self.label_hits.push((
                card.id.clone(),
                infiniterm_core::grid::Rect {
                    x: f32::from(lb.origin.x) as f64,
                    y: f32::from(lb.origin.y) as f64 - self.titlebar_h() as f64,
                    w: f32::from(lb.size.width) as f64,
                    h: f32::from(lb.size.height) as f64,
                },
            ));
            let radii = if std::mem::take(&mut in_corner) {
                corner_radii(corner.left())
            } else {
                gpui::Corners::default()
            };
            window.paint_quad(fill(lb, label_bg).corner_radii(radii));
            crate::text::paint_in(window, cx, &line, lb, label_px / 2.);
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
            CardKind::Page => badges.push("page".into()),
            CardKind::Terminal => {}
        }
        for badge in badges.into_iter().filter(|_| !hide) {
            let line = crate::text::shape(
                window,
                &badge,
                label_px,
                &chrome.typography.regular,
                chrome.text_muted,
            );
            let w = line.width + label_px;
            let bb = Bounds::new(point(place(w), top), size(w, h));
            let radii = if std::mem::take(&mut in_corner) {
                corner_radii(corner.left())
            } else {
                gpui::Corners::default()
            };
            window.paint_quad(fill(bb, chrome.badge_bg).corner_radii(radii));
            crate::text::paint_in(window, cx, &line, bb, label_px / 2.);
        }
        // Red: the one thing on a card that changes what a keystroke does.
        if let Some(remote) = &card.remote {
            let line = crate::text::shape(
                window,
                remote,
                label_px,
                &chrome.typography.bold,
                chrome.remote_fg,
            );
            // The other end of the label's edge, so the two never overlap.
            let w = line.width + label_px;
            let x = if corner.left() {
                b.origin.x + b.size.width - border - w
            } else {
                b.origin.x + border
            };
            let rb = Bounds::new(point(x, top), size(w, h));
            window
                .paint_quad(fill(rb, chrome.remote_bg).corner_radii(corner_radii(!corner.left())));
            crate::text::paint_in(window, cx, &line, rb, label_px / 2.);
        }
    }
}
