//! Moving focus: the arrows, extending a selection, the phantom slot, slot
//! picking, hints, and the group and unit rings. Port of `commands/focus.ts`.
//!
//! Everything here answers "which card next", and most of it can land on an
//! empty slot rather than a card (the phantom), which is why the phantom
//! helpers live here. `handle_bare_key` is the one place a key WITHOUT Cmd
//! reaches the app, and only while one of these modes is up.
use super::palette_state::Source;
use super::{Effect, Model, NewCard, Phantom, Switcher};
use crate::cards::{CardRect, PlacedCard, GUTTER};
use crate::grid::{Point, Rect, Size, HALF_CELL};
use crate::groups::{canvas_units, step_ring, CanvasUnit, UnitKind, UNGROUPED};
use crate::layout::rects_overlap;
use crate::multi_select::extend_selection;
use crate::navigate::{
    empty_slot_beside, ensure_visible, nearest_in_direction, nearest_to, Direction, REVEAL_PADDING,
};
use crate::saved_layout::CardKind;
use crate::slots::{free_slots_around, Slot, SLOT_KEYS};
use crate::viewport::bounding_rect;

/// A key without Cmd, as the ui sees it: named (`enter`, `escape`) or one
/// character.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BareKey {
    Enter,
    Escape,
    Char(char),
    Other,
}

impl Model {
    /// The empty slot the arrows landed on, as a card you can make. Nothing
    /// is focused while it is up, so Enter means "make the card".
    pub(super) fn show_phantom(&mut self, rect: Rect, group_id: Option<String>, keep_extra: bool) {
        self.selection.maximized = false;
        self.selection.focused_id = None;
        if !keep_extra {
            self.selection.phantom_extra.clear();
        }
        self.selection.phantom = Some(Phantom { rect, group_id });
        let next = ensure_visible(rect, self.viewport, self.view_size, REVEAL_PADDING);
        if next.x != self.viewport.x || next.y != self.viewport.y {
            self.effects.push(Effect::AnimatePan {
                x: next.x,
                y: next.y,
            });
        }
    }

    /// Fills the phantom (and any extras) with a card of `kind`. Returns
    /// false when there was no phantom.
    pub fn fill_phantom(&mut self, kind: CardKind, url: Option<String>) -> bool {
        if kind == CardKind::Browser && self.browser_refused() {
            return true;
        }
        let Some(p) = self.selection.phantom.take() else {
            return false;
        };
        let mut all = std::mem::take(&mut self.selection.phantom_extra);
        all.push(p);
        self.selection.slot_picks.clear();
        let ws = self.active_workspace.clone();
        for ph in all {
            // Filling a phantom slot makes a NEW card, so it starts at
            // `startingDir` like any other. It used to borrow the nearest
            // card's directory, which meant where a card landed decided
            // where its shell started.
            let id = self.add_card(
                &self.start_dir.clone(),
                NewCard {
                    rect: Some(ph.rect),
                    group_id: ph.group_id,
                    workspace_id: ws.clone(),
                    kind,
                    url: url.clone(),
                    ..Default::default()
                },
            );
            self.set_focus(Some(&id));
        }
        self.reveal_focused();
        true
    }

    /// Every empty slot around the cards, lettered. Found per group, because
    /// what is "occupied" differs: a card's own frame must not block the slot
    /// beside it, every other frame must. Nothing to stand beside means one
    /// slot at the origin.
    pub fn pick_slot(&mut self) {
        let here: Vec<super::Card> = self.here().into_iter().cloned().collect();
        let ws = self.active_workspace.clone().unwrap_or_default();
        let rects: Vec<Rect> = here.iter().map(|c| c.rect).collect();
        let size = self.default_size();
        let mut groups: Vec<Option<String>> = vec![];
        for c in &here {
            if !groups.contains(&c.group_id) {
                groups.push(c.group_id.clone());
            }
        }
        let mut slots: Vec<Slot> = vec![];
        for g in groups {
            let members: Vec<PlacedCard> = here
                .iter()
                .filter(|c| c.group_id == g)
                .map(|c| PlacedCard {
                    id: c.id.clone(),
                    rect: c.rect,
                    group_id: c.group_id.clone(),
                })
                .collect();
            let mut occupied = rects.clone();
            occupied.extend(self.other_frames(g.as_deref(), &ws));
            for s in free_slots_around(&members, size, GUTTER, &occupied) {
                // Keep the first proposal for any ground already offered.
                if !slots.iter().any(|k| rects_overlap(k.rect, s.rect)) {
                    slots.push(s);
                }
            }
        }
        slots.sort_by(|a, b| {
            a.rect
                .y
                .total_cmp(&b.rect.y)
                .then(a.rect.x.total_cmp(&b.rect.x))
        });
        if slots.is_empty() {
            slots.push(Slot {
                rect: Rect {
                    x: HALF_CELL,
                    y: HALF_CELL,
                    w: size.w,
                    h: size.h,
                },
                group_id: None,
                key: 'a',
            });
        }
        // Re-lettered after the merge, so the keys read in order across groups.
        let slots: Vec<Slot> = slots
            .into_iter()
            .zip(SLOT_KEYS.chars())
            .map(|(s, key)| Slot { key, ..s })
            .collect();
        // The current one: nearest to where you are, so Enter alone is the common case.
        let from = self
            .focused()
            .map(|c| c.rect)
            .or_else(|| self.selection.phantom.as_ref().map(|p| p.rect));
        let as_placed: Vec<PlacedCard> = slots
            .iter()
            .map(|s| PlacedCard {
                id: s.key.to_string(),
                rect: s.rect,
                group_id: None,
            })
            .collect();
        let nearest = from
            .and_then(|f| nearest_to(&as_placed, f))
            .map(|p| p.id.clone());
        let current = slots
            .iter()
            .find(|s| Some(s.key.to_string()) == nearest)
            .unwrap_or(&slots[0])
            .clone();
        self.selection.slot_picks = slots.clone();
        self.show_phantom(current.rect, current.group_id, false);
        // Fit everything, slots included: the letters are the point.
        let mut all = rects;
        all.extend(slots.iter().map(|s| s.rect));
        if let Some(bounds) = bounding_rect(&all) {
            self.apply_viewport(self.fit_viewport(bounds));
        }
    }

    /// A letter in slot-picking mode: that slot, now.
    fn pick_slot_by_key(&mut self, key: char) -> bool {
        let Some(slot) = self
            .selection
            .slot_picks
            .iter()
            .find(|s| s.key == key)
            .cloned()
        else {
            return false;
        };
        self.selection.slot_picks.clear();
        self.selection.phantom_extra.clear();
        self.selection.phantom = Some(Phantom {
            rect: slot.rect,
            group_id: slot.group_id,
        });
        self.fill_phantom(CardKind::Terminal, None)
    }

    /// A letter on every card, reading order, home row first: one keystroke
    /// to any of them instead of a walk with the arrows.
    fn show_hints(&mut self) {
        let mut here: Vec<(String, Rect)> = self
            .here()
            .into_iter()
            .map(|c| (c.id.clone(), c.rect))
            .collect();
        if here.is_empty() {
            return;
        }
        here.sort_by(|a, b| a.1.y.total_cmp(&b.1.y).then(a.1.x.total_cmp(&b.1.x)));
        self.selection.hints = here
            .iter()
            .zip(SLOT_KEYS.chars())
            .map(|((id, _), k)| (id.clone(), k))
            .collect();
        if let Some(bounds) = bounding_rect(&here.iter().map(|(_, r)| *r).collect::<Vec<_>>()) {
            self.apply_viewport(self.fit_viewport(bounds));
        }
    }

    fn focus_by_hint(&mut self, key: char) -> bool {
        let id = self
            .selection
            .hints
            .iter()
            .find(|(_, k)| **k == key)
            .map(|(id, _)| id.clone());
        self.selection.hints.clear();
        let Some(id) = id else { return false };
        self.set_focus(Some(&id));
        // A jump lands framed, as Cmd+1 would (Ekin, 2026-09-26): you
        // jumped to it to work in it.
        if let Some(card) = self.card(&id) {
            let (rect, group) = (card.rect, card.soft_group_id.clone());
            match self.slot_bounds(&[rect], &[group]) {
                Some(bounds) => self.frame_card(bounds),
                None => self.reveal_focused(),
            }
        }
        true
    }

    fn occupied_here(&self) -> Vec<Rect> {
        let mut occupied: Vec<Rect> = self.here().iter().map(|c| c.rect).collect();
        occupied.extend(self.other_frames(None, self.active_workspace.as_deref().unwrap_or("")));
        occupied
    }

    /// Extend from a phantom: one more empty slot, the way it is one more
    /// card from a card. Reversing onto a slot already selected drops the one
    /// being left. Only slots: a selection that mixed cards and holes would
    /// have to answer what closing means for a hole.
    fn extend_phantom(&mut self, dir: Direction) {
        let Some(p) = self.selection.phantom.clone() else {
            return;
        };
        // The phantom's own size, so a chain of slots beside a quarter card
        // is a chain of quarters, the way `card.swap` moves the card as is.
        let size = Size {
            w: p.rect.w,
            h: p.rect.h,
        };
        let back_rect = empty_slot_beside(p.rect, dir, GUTTER, &[], size);
        if let Some(i) = self
            .selection
            .phantom_extra
            .iter()
            .position(|q| Some(q.rect) == back_rect)
        {
            let back = self.selection.phantom_extra.remove(i);
            self.show_phantom(back.rect, back.group_id, true);
            return;
        }
        let mut occupied = self.occupied_here();
        occupied.extend(self.selection.phantom_extra.iter().map(|q| q.rect));
        let Some(hole) = empty_slot_beside(p.rect, dir, GUTTER, &occupied, size) else {
            return;
        };
        self.selection.phantom_extra.push(p.clone());
        self.show_phantom(hole, p.group_id, true);
    }

    /// Only the cards you can see: every workspace starts placing at the same
    /// origin, so a card on another canvas sits exactly where a neighbour
    /// here would.
    pub fn focus_neighbour(&mut self, dir: Direction) {
        let here = self.here();
        let placed = self.placed(&here);
        let occupied = self.occupied_here();
        // A slot beside a card is the card's size, not the default: beside a
        // quarter card (a split kept after `card.close.leave`) it is the
        // quarter that was freed, which a default-sized slot could not
        // reach, and `card.swap` already measures the same way.
        let beside = |r: Rect| Size { w: r.w, h: r.h };

        // Picking a slot: the arrows walk the lettered slots, nothing else.
        if let (Some(p), false) = (
            self.selection.phantom.clone(),
            self.selection.slot_picks.is_empty(),
        ) {
            let picks: Vec<PlacedCard> = self
                .selection
                .slot_picks
                .iter()
                .map(|s| PlacedCard {
                    id: s.key.to_string(),
                    rect: s.rect,
                    group_id: None,
                })
                .collect();
            let current = self
                .selection
                .slot_picks
                .iter()
                .find(|s| s.rect.x == p.rect.x && s.rect.y == p.rect.y)
                .map(|s| s.key.to_string())
                .unwrap_or_default();
            let next = nearest_in_direction(&picks, &current, dir).map(|n| n.id.clone());
            if let Some(slot) = next
                .and_then(|k| {
                    self.selection
                        .slot_picks
                        .iter()
                        .find(|s| s.key.to_string() == k)
                })
                .cloned()
            {
                self.show_phantom(slot.rect, slot.group_id, false);
            }
            return;
        }

        // From a phantom: the same rules, with the phantom standing in for a card.
        if let Some(p) = self.selection.phantom.clone() {
            if let Some(hole) = empty_slot_beside(p.rect, dir, GUTTER, &occupied, beside(p.rect)) {
                self.show_phantom(hole, p.group_id, false);
                return;
            }
            let mut with_ghost = vec![PlacedCard {
                id: String::new(),
                rect: p.rect,
                group_id: None,
            }];
            with_ghost.extend(placed.iter().cloned());
            let next = nearest_in_direction(&with_ghost, "", dir)
                .map(|n| n.id.clone())
                .or_else(|| nearest_to(&placed, p.rect).map(|n| n.id.clone()));
            self.selection.phantom = None;
            self.set_focus(next.as_deref());
            self.reveal_focused();
            return;
        }

        // With nothing active, the first arrow adopts a card rather than doing nothing.
        let Some(focused) = self.selection.focused_id.clone() else {
            let first = here.first().map(|c| c.id.clone());
            self.set_focus(first.as_deref());
            if first.is_some() {
                self.reveal_focused();
            }
            return;
        };
        if let Some(card) = self.card(&focused).cloned() {
            if let Some(hole) =
                empty_slot_beside(card.rect, dir, GUTTER, &occupied, beside(card.rect))
            {
                self.show_phantom(hole, card.group_id, false);
                return;
            }
        }
        let Some(next) = nearest_in_direction(&placed, &focused, dir).map(|n| n.id.clone()) else {
            return;
        };
        // Walked into, not chosen: the visit reaches the switcher's list
        // only if it is stayed in (`Model::focus_traversing`).
        self.focus_traversing(Some(&next));
        self.reveal_focused();
    }

    pub fn extend(&mut self, dir: Direction) {
        if self.selection.phantom.is_some() {
            self.extend_phantom(dir);
            return;
        }
        let Some(focused) = self.selection.focused_id.clone() else {
            return;
        };
        if self.selection.maximized {
            return;
        }
        let here = self.here();
        let placed = self.placed(&here);
        let Some(next) = extend_selection(&placed, &focused, &self.selection.extra, dir) else {
            return;
        };
        self.focus_extended(&next.focused_id, next.extra);
        self.reveal_focused();
    }

    /// Shift+click: onto an unselected card, focus moves there and the card
    /// it left stays selected; onto a card already in the selection, that
    /// card drops out.
    pub fn extend_to(&mut self, id: &str) {
        let Some(focused) = self.selection.focused_id.clone() else {
            self.set_focus(Some(id));
            return;
        };
        if focused == id {
            self.set_focus(Some(id));
            return;
        }
        if let Some(i) = self.selection.extra.iter().position(|x| x == id) {
            self.selection.extra.remove(i);
            return;
        }
        let mut extra = self.selection.extra.clone();
        extra.push(focused);
        self.focus_extended(id, extra);
    }

    /// A rectangle dragged on bare canvas (2026-09-29): every card of this
    /// workspace it touches is selected, the one nearest where the drag
    /// began taking the focus. `kept` is the selection from before a
    /// Shift+drag, which stays in; `None` replaces it. Set directly, not
    /// through the focus trail: this runs on every mouse move, and
    /// `marquee_done` lands the result once at the release.
    pub fn marquee_select(&mut self, rect: Rect, anchor: Point, kept: Option<&[String]>) {
        let dist = |r: &Rect| {
            let (cx, cy) = (r.x + r.w / 2., r.y + r.h / 2.);
            (cx - anchor.x).hypot(cy - anchor.y)
        };
        let mut touched: Vec<(f64, String)> = self
            .here()
            .iter()
            .filter(|c| crate::layout::rects_overlap(rect, c.rect))
            .map(|c| (dist(&c.rect), c.id.clone()))
            .collect();
        touched.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut ids: Vec<String> = kept.map(<[String]>::to_vec).unwrap_or_default();
        for (_, id) in touched {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        // The nearest touched card leads; with nothing touched, whatever
        // Shift kept.
        let first = kept.map_or(0, |k| k.len());
        let focus = ids.get(first).or_else(|| ids.first()).cloned();
        self.selection.extra = ids
            .into_iter()
            .filter(|id| Some(id) != focus.as_ref())
            .collect();
        self.selection.focused_id = focus;
    }

    /// The drag let go: the selection it made becomes a real focus change
    /// (the trail, the phantom dismissed), once.
    pub fn marquee_done(&mut self) {
        let extra = self.selection.extra.clone();
        match self.selection.focused_id.clone() {
            Some(id) => self.focus_extended(&id, extra),
            None => self.set_focus(None),
        }
    }

    /// Cmd+click on a card: out of the selection when it is in it, into it
    /// when it is not (Finder's rule, Ekin 2026-09-29). A lone focused card
    /// is not a selection to toggle: that click is the body's (a link).
    /// Returns whether the click was taken.
    pub fn toggle_selected(&mut self, id: &str) -> bool {
        let focused = self.selection.focused_id.clone();
        if let Some(i) = self.selection.extra.iter().position(|x| x == id) {
            self.selection.extra.remove(i);
            return true;
        }
        if focused.as_deref() == Some(id) {
            if self.selection.extra.is_empty() {
                return false;
            }
            let next = self.selection.extra.pop();
            let extra = self.selection.extra.clone();
            if let Some(next) = next {
                self.focus_extended(&next, extra);
            }
            return true;
        }
        self.extend_to(id);
        true
    }

    /// The top-level things on the canvas: whole groups, and every loose card
    /// as a single trailing stop. Scoped to the visible canvas.
    fn units(&self) -> Vec<CanvasUnit> {
        let here = self.here();
        let placed = self.placed(&here);
        let ws = self.active_workspace.clone().unwrap_or_default();
        let frames: Vec<CardRect> = self
            .groups
            .iter()
            .filter_map(|g| {
                self.group_frame(&g.id, &ws).map(|rect| CardRect {
                    id: g.id.clone(),
                    rect,
                })
            })
            .collect();
        canvas_units(&placed, &frames)
    }

    /// The unit the active card belongs to: its group, or the loose set.
    fn unit_of_active(&self) -> Option<String> {
        self.focused()
            .map(|c| c.group_id.clone().unwrap_or_else(|| UNGROUPED.to_string()))
    }

    /// Lands on the card you were last on there, falling back to the first in
    /// reading order, and frames the unit.
    fn go_to_unit(&mut self, unit: &CanvasUnit) {
        let remembered = self
            .last_focused
            .get(&unit.id)
            .filter(|r| unit.card_ids.contains(r))
            .cloned();
        let target = remembered.or_else(|| unit.card_ids.first().cloned());
        self.set_focus(target.as_deref());
        self.apply_viewport(self.fit_viewport(unit.rect));
    }

    fn step_units(&mut self, groups_only: bool, step: isize) {
        let ring: Vec<CanvasUnit> = self
            .units()
            .into_iter()
            .filter(|u| !groups_only || u.kind == UnitKind::Group)
            .collect();
        if ring.is_empty() {
            return;
        }
        let ids: Vec<String> = ring.iter().map(|u| u.id.clone()).collect();
        let current = self.unit_of_active().unwrap_or_default();
        let Some(next) = step_ring(&ids, &current, step).cloned() else {
            return;
        };
        if let Some(unit) = ring.into_iter().find(|u| u.id == next) {
            self.go_to_unit(&unit);
        }
    }

    /// A key without Cmd, while a mode that owns bare keys is up. True when
    /// consumed. Hints: a letter focuses a card. A phantom: Enter asks what
    /// goes in the slot, Escape leaves, a letter picks a slot. A
    /// multi-selection: Escape collapses it. Outside those, every bare key
    /// belongs to the shell. With the palette or a prompt up, Enter and
    /// Escape are theirs.
    pub fn handle_bare_key(&mut self, key: BareKey) -> bool {
        if self.palette_open() || self.prompt.is_open() {
            return false;
        }
        if !self.selection.hints.is_empty() {
            match key {
                BareKey::Escape => self.selection.hints.clear(),
                BareKey::Char(c) => {
                    self.focus_by_hint(c.to_ascii_lowercase());
                }
                _ => {}
            }
            return true;
        }
        if let Some(p) = self.selection.phantom.clone() {
            match key {
                BareKey::Enter => {
                    self.open_palette(Source::SlotKind);
                    return true;
                }
                BareKey::Escape => {
                    self.selection.phantom = None;
                    self.selection.phantom_extra.clear();
                    self.selection.slot_picks.clear();
                    let here = self.here();
                    let placed = self.placed(&here);
                    let next = nearest_to(&placed, p.rect).map(|n| n.id.clone());
                    self.set_focus(next.as_deref());
                    return true;
                }
                BareKey::Char(c) if !self.selection.slot_picks.is_empty() => {
                    return self.pick_slot_by_key(c.to_ascii_lowercase())
                }
                _ => {}
            }
        }
        if !self.selection.extra.is_empty() && key == BareKey::Escape {
            self.selection.extra.clear();
            return true;
        }
        false
    }
}

impl Model {
    /// Ctrl+Tab, and every press after it while Ctrl is still down: the
    /// first press freezes the rows so stepping cannot shuffle under the
    /// hand, the rest only move the selection. The ui commits when Ctrl
    /// comes up (`switcher_commit`), or Escape puts it back.
    pub fn switcher_step(&mut self, delta: isize) {
        if self.switcher.is_none() {
            let all: Vec<String> = if self.config.ui.workspace_isolation {
                self.here().iter().map(|c| c.id.clone()).collect()
            } else {
                self.cards.iter().map(|c| c.id.clone()).collect()
            };
            let list = crate::switcher::order(
                self.selection.focused_id.as_deref(),
                &self.focus_trail,
                &all,
            );
            if list.len() < 2 {
                return;
            }
            self.switcher = Some(Switcher { list, index: 0 });
        }
        let Some(s) = &mut self.switcher else { return };
        s.index = crate::switcher::step(s.list.len(), s.index, delta);
    }

    /// Ctrl came up: go to the row under the selection. A card in another
    /// workspace takes its workspace with it, the way the palette's card
    /// rows do.
    pub fn switcher_commit(&mut self) {
        let Some(s) = self.switcher.take() else {
            return;
        };
        let Some(id) = s.list.get(s.index).cloned() else {
            return;
        };
        self.go_to_card(&id);
    }

    pub fn switcher_cancel(&mut self) {
        self.switcher = None;
    }
}

pub fn register(r: &mut crate::commands::CommandRegistry<Model>) {
    use super::cards_cmd::dir_name;
    // Ctrl+Tab: the cards you have actually worked in, most recent first,
    // committed when Ctrl comes up (the ui watches the modifier, keycode.rs).
    for (id, label, delta) in [
        (
            "card.switcher.next",
            "Card: switch, most recent first",
            1isize,
        ),
        ("card.switcher.prev", "Card: switch, backwards", -1),
    ] {
        r.register(id, label, move |m| m.switcher_step(delta));
    }
    use super::context::{step_id, where_, which};
    let dirs = [
        Direction::Left,
        Direction::Right,
        Direction::Up,
        Direction::Down,
    ];
    // Moving focus scrolls the target into view only if it is off screen.
    for dir in dirs {
        r.register(
            &format!("focus.move.{}", dir_name(dir)),
            &format!("Focus: the card {}", where_(dir)),
            move |m| m.focus_neighbour(dir),
        );
    }
    r.register(
        "focus.hint",
        "Focus: a letter on every card, press one to go there",
        |m| {
            if m.selection.hints.is_empty() {
                m.show_hints();
            } else {
                m.selection.hints.clear();
            }
        },
    );
    for dir in dirs {
        r.register(
            &format!("focus.extend.{}", dir_name(dir)),
            &format!("Focus: extend the selection {}", where_(dir)),
            move |m| m.extend(dir),
        );
    }
    // Steps between groups only, fitting rather than just focusing: switching
    // groups is the moment you change what you are working on.
    for step in [1isize, -1] {
        r.register(
            &format!("group.focus.{}", step_id(step)),
            &format!("Group: go to the {}", which(step)),
            move |m| m.step_units(true, step),
        );
    }
    // The superset: every group and the loose cards, in reading order.
    for step in [1isize, -1] {
        r.register(
            &format!("focus.{}", step_id(step)),
            &format!("Focus: the {} group or the loose cards", which(step)),
            move |m| m.step_units(false, step),
        );
    }
}

/// The world point under a screen point, for callers that hit-test.
pub fn world_point(m: &Model, screen: Point) -> Point {
    crate::viewport::world_pos_of(screen, m.viewport)
}
