//! Card commands: new, split, close, the editor and browser actions, move,
//! resize, swap, sidebar, maximise, place, clear, rename. Port of
//! `commands/cards.ts`. Labels read `Domain: what it does`; a command lives
//! in the module whose prefix it carries, and `card.clear` is `Terminal:`.
use super::palette_state::Source;
use super::palette_state::{CARD_ROW, SIZES, WORKSPACE_ROW};
use super::{
    BrowserAction, Card, EditorAction, Effect, LayoutSnapshot, Model, NewCard, Pending,
    TextTransform, UndoStep, LAYOUT_UNDO_DEPTH,
};
use crate::card_label::{card_label, tilde_path, Labelled};
use crate::cards::GUTTER;
use crate::config::{BROWSER_ZOOM_MAX, BROWSER_ZOOM_MIN};
use crate::grid::{snap_rect, Point, Rect, HALF_CELL};
use crate::ift::{diff_plan, open_plan, transcript_plan, PathKind};
use crate::layout::rects_overlap;
use crate::navigate::{nearest_to, Direction};
use crate::resize::{
    fill_from_corner, moved_by, resized_by, sized, Fraction, MOVE_STEP, RESIZE_STEP,
};
use crate::saved_layout::CardKind;
use crate::sidebar::{clamp_sidebar, sidebar_extent, sidebar_width};
use crate::split::{split_rect, SplitSide};
use crate::swap::swap_with_neighbour;

/// How many full slots around a dragged ghost show their slots: enough to
/// see where the card could go next, not the whole canvas.
const GHOST_SLOT_REACH: i64 = 2;

/// How long a Save from the unsaved-changes sheet may take before the close
/// it holds is dropped: a save is one frame when it works.
const SAVE_CLOSE_MS: f64 = 2000.;

pub const ZOOM_STEP: f64 = 1.1;

impl Model {
    /// The active card at a size chosen from the default's fractions, from
    /// its own top-left corner, if that space is free. Cards may not
    /// overlap, so a card in the way means nothing moves and the status
    /// bar says why; the space it needs is what Cmd+Ctrl+W leaves behind.
    /// Shrinking always fits and leaves the rest free. Either way the card
    /// stops being half of a split pair.
    pub fn resize_active(&mut self, w: Fraction, h: Fraction) {
        self.with_active_card(|m, id| {
            let Some(card) = m.card(&id).cloned() else {
                return;
            };
            let next = sized(card.rect, m.default_size(), w, h, GUTTER);
            if next == card.rect {
                return;
            }
            m.remember_layout();
            let mut taken: Vec<Rect> = m
                .here()
                .iter()
                .filter(|c| c.id != id)
                .map(|c| c.rect)
                .collect();
            taken.extend(m.other_frames(Some(&id), &card.workspace_id));
            if taken.iter().any(|r| rects_overlap(next, *r)) {
                m.notify("no room: a card is in the way");
                return;
            }
            m.mark_swap(std::slice::from_ref(&id));
            if let Some(c) = m.card_mut(&id) {
                c.rect = next;
                c.soft_group_id = None;
            }
            m.dirty_layout = true;
            m.reveal_focused();
        })
    }

    /// The label with the card's number ahead of it: what the corner chip,
    /// the palette and the status bar show, so "#7" is enough to name a
    /// card to somebody else. `label_of` is the bare one, for a name that
    /// becomes something else (a group's name, a rename prompt's default).
    pub fn numbered_label(&self, card: &Card) -> String {
        let lock = if card.protected { "\u{1f512} " } else { "" };
        if card.number == 0 {
            return format!("{lock}{}", self.label_of(card));
        }
        format!("{lock}#{} {}", card.number, self.label_of(card))
    }

    pub fn label_of(&self, card: &Card) -> String {
        card_label(
            &Labelled {
                title: &card.title,
                // Only an agent's title is trusted: a card running Claude
                // Code or Pi has a transcript or a saved session, and that
                // title IS the session's name. A plain shell's title is its
                // own idea of the directory or the last command, which is
                // what the label already says better.
                session: card
                    .runs_agent()
                    .then_some(card.osc_title.as_deref())
                    .flatten(),
                proc: card.proc.as_deref(),
                cwd: &card.cwd,
                path: card.path.as_deref(),
                kind: Some(card.kind),
                url: card.url.as_deref(),
                root: card.root.as_deref(),
            },
            &self.home,
        )
    }

    /// `inherit` decides whether the new card joins the active card's group.
    /// The DIRECTORY is not inherited either way: a new card is a new place
    /// to work and starts at `startingDir`. A split is the opposite case
    /// and keeps inheriting, because it is carved out of the card it came
    /// from and is plainly about continuing there.
    pub fn new_card(&mut self, inherit: bool) {
        if self.fill_phantom(CardKind::Terminal, None) {
            return;
        }
        self.selection.maximized = false;
        // One beside each selected card; with nothing selected, one at the top.
        let mut froms: Vec<Option<Card>> = self.selected().into_iter().cloned().map(Some).collect();
        if froms.is_empty() {
            froms.push(None);
        }
        for from in froms {
            let group_id = if inherit {
                from.as_ref().and_then(|c| c.group_id.clone())
            } else {
                None
            };
            let ws = self.active_workspace.clone().unwrap_or_default();
            let id = self.add_card(
                &self.start_dir.clone(),
                NewCard {
                    avoid: self.other_frames(group_id.as_deref(), &ws),
                    group_id,
                    workspace_id: Some(ws),
                    ..Default::default()
                },
            );
            self.set_focus(Some(&id));
        }
        self.reveal_focused();
    }

    /// A new card of `kind` beside the active card, in its directory and
    /// group; `command` runs instead of the shell (the agents).
    pub(super) fn new_beside_active(
        &mut self,
        kind: CardKind,
        command: Option<&str>,
        url: Option<String>,
    ) {
        if kind == CardKind::Browser && self.browser_refused() {
            return;
        }
        let from = self.focused().cloned();
        let group_id = from.as_ref().and_then(|c| c.group_id.clone());
        let ws = self.active_workspace.clone().unwrap_or_default();
        self.selection.maximized = false;
        let id = self.add_card(
            &self.cwd_beside(from.as_ref()),
            NewCard {
                kind,
                command: command.map(String::from),
                url,
                avoid: self.other_frames(group_id.as_deref(), &ws),
                group_id,
                workspace_id: Some(ws),
                ..Default::default()
            },
        );
        self.set_focus(Some(&id));
        self.reveal_focused();
    }

    /// iTerm2's split keys: the active card gives up half of itself and the
    /// new card takes that half; two ordinary cards from then on, not a pane
    /// tree. The new card joins the old one's group because it sits inside
    /// the old one's footprint.
    pub fn split(&mut self, side: SplitSide) {
        self.for_selected(|m, id| {
            let Some(card) = m.card(&id).cloned() else {
                return;
            };
            let Some(split) = split_rect(card.rect, side, GUTTER) else {
                m.notify("too small to split");
                return;
            };
            // Armed before the rect changes, same as a swap, so the shrink glides.
            m.mark_swap(std::slice::from_ref(&id));
            let soft = card.soft_group_id.clone().unwrap_or_else(super::new_id);
            if let Some(c) = m.card_mut(&id) {
                c.rect = split.kept;
                c.soft_group_id = Some(soft.clone());
            }
            let new = m.add_card(
                &m.cwd_beside(Some(&card)),
                NewCard {
                    rect: Some(split.made),
                    group_id: card.group_id.clone(),
                    workspace_id: Some(card.workspace_id.clone()),
                    soft_group_id: Some(soft),
                    split_from: Some(id.clone()),
                    ..Default::default()
                },
            );
            m.set_focus(Some(&new));
            m.reveal_focused();
        });
    }

    /// Every selected card, focused one last so focus hands off once. A
    /// config pair closes as one and returns focus to where it was opened
    /// from. An editor with unsaved changes closes on the SECOND press: two
    /// presses rather than a dialog, since a modal would stop a hand that was
    /// closing five cards in a row.
    pub fn close_selected(&mut self) {
        self.close_selected_with(true);
    }

    /// `reclaim` is whether a split partner grows into the closed card's
    /// space. Cmd+W says yes; `card.close.leave` says no, for the case of
    /// splitting a card down to a quarter and wanting only that quarter:
    /// the rest of the space is left free, for whatever comes next.
    pub fn close_selected_with(&mut self, reclaim: bool) {
        let ids = self.selected_ids();
        // A single card holding several tabs, browser or editor, is
        // several things at once, the same reasoning `workspace.close`
        // already applies to several shells: it asks first, unsaved or
        // not. Ekin closed an editor of tabs meaning to close one tab,
        // with the card not locked. A config pair never has tabs, so it
        // keeps its own two-press dirty flow below.
        if let [id] = ids.as_slice() {
            let is_pair = self.config_pairs.iter().any(|p| p.ids.contains(id));
            if !is_pair {
                if let Some(count) = self
                    .card(id)
                    .filter(|c| {
                        matches!(c.kind, CardKind::Browser | CardKind::Editor) && c.tabs.len() > 1
                    })
                    .map(|c| c.tabs.len())
                {
                    let label = format!("close this card and its {count} tabs?");
                    self.prompt.confirm(
                        &label,
                        "Close card",
                        Pending::CloseCard {
                            id: id.clone(),
                            reclaim,
                        },
                    );
                    return;
                }
            }
        }
        self.close_selected_confirmed(ids, reclaim, false);
    }

    /// `discard`: the unsaved-changes question was already answered yes.
    fn close_selected_confirmed(&mut self, mut ids: Vec<String>, reclaim: bool, discard: bool) {
        let pair = if ids.len() == 1 {
            self.config_pairs
                .iter()
                .find(|p| p.ids.contains(&ids[0]))
                .cloned()
        } else {
            None
        };
        if let Some(p) = &pair {
            ids = p
                .ids
                .iter()
                .filter(|id| self.card(id).is_some())
                .cloned()
                .collect();
        }
        // Unsaved changes get the macOS save sheet, not a second Cmd+W: a
        // double press discarded work without the second press being meant
        // (Ekin, 2026-09-26). Enter saves and closes; Don't Save (Cmd+D)
        // discards; a Cmd+W while it is up asks the same question again.
        if !discard {
            if let Some(dirty) = ids
                .iter()
                .find(|id| self.card(id).is_some_and(|c| c.dirty))
                .and_then(|id| self.card(id))
                .cloned()
            {
                let label = format!("save changes to {}?", self.label_of(&dirty));
                self.prompt.confirm3(
                    &label,
                    "Save",
                    "Don't Save",
                    Pending::UnsavedClose { ids, reclaim },
                );
                return;
            }
        }
        for id in ids.iter().skip(1).rev() {
            self.close_card_with(id, false, reclaim);
        }
        if let Some(first) = ids.first() {
            self.close_card_with(first, false, reclaim);
        }
        if let Some(p) = pair {
            self.config_pairs.retain(|q| q.ids != p.ids);
            if let Some(back) = p.return_to.filter(|id| self.card(id).is_some()) {
                self.set_focus(Some(&back));
                self.reveal_focused();
            }
        }
    }

    /// Save, then close once every card is clean. The save runs in the
    /// editor, a frame away, and can fail (a read-only file) or need a
    /// name (untitled), so the close waits for the dirty flags the editor
    /// mirrors back (`tick`), and gives up with a notice rather than
    /// closing on a save that did not happen.
    fn save_then_close(&mut self, ids: Vec<String>, reclaim: bool) {
        for id in &ids {
            let Some(card) = self.card(id) else { continue };
            if !card.dirty {
                continue;
            }
            // Untitled: name it first; the close follows the save.
            if card.path.is_none() {
                self.open_save_as(id.clone(), Some((ids.clone(), reclaim)));
                return;
            }
            self.effects.push(Effect::Editor {
                card_id: id.clone(),
                action: EditorAction::Save,
            });
        }
        self.close_after_save = Some((ids, reclaim, self.now_ms + SAVE_CLOSE_MS));
    }

    /// The "save as" field for an untitled buffer, starting on its
    /// directory and `untitled.txt` with the stem selected (`save_as.rs`).
    pub fn open_save_as(&mut self, id: String, then_close: Option<(Vec<String>, bool)>) {
        // A card made from nothing may have no directory: home, not `/`.
        let dir = self
            .card(&id)
            .map(|c| c.cwd.clone())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| self.home.clone());
        let (text, select) = crate::save_as::suggestion(&dir, &self.home);
        self.prompt
            .ask_selecting("save as", &text, select, Pending::SaveAs { id, then_close });
    }

    /// The buffer gets its path and is written; a close the save sheet
    /// asked for follows once the editor reports it clean.
    fn save_as(&mut self, id: String, full: String, then_close: Option<(Vec<String>, bool)>) {
        if let Some(card) = self.card_mut(&id) {
            card.cwd = full
                .rsplit_once('/')
                .map(|(dir, _)| if dir.is_empty() { "/" } else { dir })
                .unwrap_or("/")
                .to_string();
            card.path = Some(full);
            self.dirty_layout = true;
        }
        self.effects.push(Effect::Editor {
            card_id: id,
            action: EditorAction::Save,
        });
        if let Some((ids, reclaim)) = then_close {
            self.close_after_save = Some((ids, reclaim, self.now_ms + SAVE_CLOSE_MS));
        }
    }

    /// The close `save_then_close` is waiting on, run from `tick`.
    pub(super) fn close_when_saved(&mut self) {
        let Some((ids, reclaim, until)) = self.close_after_save.clone() else {
            return;
        };
        let clean = ids.iter().all(|id| self.card(id).is_none_or(|c| !c.dirty));
        if clean {
            self.close_after_save = None;
            self.close_selected_confirmed(ids, reclaim, true);
        } else if self.now_ms >= until {
            self.close_after_save = None;
            self.notify("not saved, so not closed");
        }
    }

    fn editor_action(&mut self, action: EditorAction) {
        self.with_active_card(|m, id| {
            m.effects.push(Effect::Editor {
                card_id: id,
                action,
            })
        });
    }

    /// The typed path, resolved against the active card's directory with `~`
    /// for home. The caller has checked it exists.
    pub fn resolve_typed_path(&self, raw: &str, from: Option<&Card>) -> String {
        let home = self.home.trim_end_matches('/');
        let cwd = from
            .map(|c| c.cwd.clone())
            .unwrap_or_else(|| self.cwd_beside(None));
        if raw.starts_with('/') {
            raw.to_string()
        } else if raw == "~" {
            home.to_string()
        } else if let Some(rest) = raw.strip_prefix("~/") {
            format!("{home}/{rest}")
        } else {
            format!("{}/{raw}", cwd.trim_end_matches('/'))
        }
    }

    /// A prompt's answer, for the questions the card commands ask.
    pub fn answer(
        &mut self,
        pending: Pending,
        text: Option<String>,
        exists: impl Fn(&str) -> bool,
    ) {
        match pending {
            Pending::RenameCard(id) => {
                if let (Some(name), Some(card)) = (text, self.card_mut(&id)) {
                    card.title = name;
                    self.dirty_layout = true;
                }
            }
            Pending::NameGroup(ids) => {
                let Some(name) = text else { return };
                // A group starts with the selection in it rather than empty,
                // and the selection MOVES to a free block as one, keeping its
                // layout; grouping in place made the overlapping tangle.
                let group_id = self.add_group(&name);
                for id in &ids {
                    if let Some(c) = self.card_mut(id) {
                        c.group_id = Some(group_id.clone());
                    }
                }
                self.move_to_free_block(&ids);
                self.prune_empty_groups(); // it may have just left a group of one
                self.reveal_focused();
            }
            Pending::RenameGroup(id) => {
                if let (Some(name), Some(g)) = (text, self.groups.iter_mut().find(|g| g.id == id)) {
                    g.name = name;
                    self.dirty_layout = true;
                }
            }
            Pending::RenameWorkspace(id) => {
                if let (Some(name), Some(w)) =
                    (text, self.workspaces.iter_mut().find(|w| w.id == id))
                {
                    w.name = name;
                    self.dirty_layout = true;
                }
            }
            Pending::CloseWorkspace(id) => {
                if text.is_some() {
                    self.close_workspace_confirmed(&id);
                }
            }
            Pending::CloseCard { id, reclaim } => {
                if text.is_some() {
                    self.close_card_with(&id, false, reclaim);
                }
            }
            Pending::UnsavedClose { ids, reclaim } => match text.as_deref() {
                None => {}
                Some(crate::prompt::ALT) => self.close_selected_confirmed(ids, reclaim, true),
                Some(_) => self.save_then_close(ids, reclaim),
            },
            Pending::RemoteColor => self.remote_color_answer(text),
            Pending::About => {
                if text.is_some() {
                    self.effects.push(Effect::CheckForUpdate);
                }
            }
            Pending::NavigateBrowser(id) => {
                if let (Some(url), Some(card)) =
                    (Model::normalise_url(text.as_deref()), self.card_mut(&id))
                {
                    card.url = Some(url);
                    self.dirty_layout = true;
                }
            }
            // The typed path is relative to the card's directory, `~`
            // allowed, and the card becomes that file: its directory follows
            // so "beside" and the label do.
            Pending::SaveAs { id, then_close } => {
                let Some(raw) = text.filter(|t| !t.trim().is_empty()) else {
                    return;
                };
                let from = self.card(&id).cloned();
                let full = self.resolve_typed_path(raw.trim(), from.as_ref());
                match crate::save_as::target(&full, &exists) {
                    crate::save_as::Target::New => self.save_as(id, full, then_close),
                    crate::save_as::Target::Replace => {
                        let name = full.rsplit('/').next().unwrap_or(&full).to_string();
                        self.prompt.confirm(
                            &format!("{name} exists. Replace it?"),
                            "Replace",
                            Pending::SaveAsReplace {
                                id,
                                path: full,
                                then_close,
                            },
                        );
                    }
                    // Back to the field with what was typed, so the fix is
                    // one edit, not a retype.
                    crate::save_as::Target::NoDirectory(dir) => {
                        self.prompt
                            .ask("save as", raw.trim(), Pending::SaveAs { id, then_close });
                        self.notify(format!(
                            "no such directory: {}",
                            tilde_path(&dir, &self.home)
                        ));
                    }
                }
            }
            Pending::SaveAsReplace {
                id,
                path,
                then_close,
            } => {
                if text.is_some() {
                    self.save_as(id, path, then_close);
                }
            }
            Pending::GoToLine(id) => {
                let Some(n) = text
                    .and_then(|t| t.trim().parse::<u64>().ok())
                    .filter(|n| *n > 0)
                else {
                    return;
                };
                if let Some(card) = self.card_mut(&id) {
                    card.line = Some(n);
                }
                self.effects.push(Effect::Editor {
                    card_id: id,
                    action: EditorAction::GoToLine,
                });
            }
        }
    }

    /// The end of a drag or a resize. A card dropped over another goes back
    /// where it started: a card behind a card is a card you cannot see, and
    /// the placement rules already keep every card on its own ground. Other
    /// groups' frames count too, as they do for placement. Returns whether
    /// the move stood.
    pub fn end_gesture(&mut self, ids: &[String], start: &[(String, Rect)]) -> bool {
        let ws = self.active_workspace.clone().unwrap_or_default();
        // The frames of every group a moved card belongs to follow the
        // move, so none of them is in the way: a selection dragged across
        // two groups' cards bounced back on its own frames when only the
        // first card's group was spared.
        let moved_groups: Vec<String> = ids
            .iter()
            .filter_map(|id| self.card(id).and_then(|c| c.group_id.clone()))
            .collect();
        let mut occupied: Vec<Rect> = self
            .cards
            .iter()
            .filter(|c| c.workspace_id == ws && !ids.contains(&c.id))
            .map(|c| c.rect)
            .collect();
        occupied.extend(
            self.groups
                .iter()
                .filter(|g| !moved_groups.contains(&g.id))
                .filter_map(|g| self.group_frame(&g.id, &ws)),
        );
        let moved: Vec<Rect> = ids
            .iter()
            .filter_map(|id| self.card(id))
            .map(|c| snap_rect(c.rect))
            .collect();
        if moved
            .iter()
            .any(|m| occupied.iter().any(|o| rects_overlap(*m, *o)))
        {
            for (id, rect) in start {
                if let Some(c) = self.card_mut(id) {
                    c.rect = *rect;
                }
            }
            self.notify("cards cannot overlap");
            self.dirty_layout = true;
            return false;
        }
        // Backstop: realigns a card whose stored rect was off-grid before the drag.
        for id in ids {
            if let Some(c) = self.card_mut(id) {
                c.rect = snap_rect(c.rect);
            }
        }
        self.dirty_layout = true;
        true
    }

    /// While a gesture is live: whether the moving cards overlap anything.
    /// The active canvas's rects, kept before a change so Cmd+Z can put
    /// them back. Every command that moves or resizes a card calls this
    /// first; a new change forgets the redo trail.
    pub fn remember_layout(&mut self) {
        let ws = self.active_workspace.clone().unwrap_or_default();
        let snap = LayoutSnapshot {
            workspace_id: ws.clone(),
            rects: self
                .cards
                .iter()
                .filter(|c| c.workspace_id == ws)
                .map(|c| (c.id.clone(), c.rect))
                .collect(),
        };
        if self.layout_undo.last() == Some(&UndoStep::Rects(snap.clone())) {
            return;
        }
        self.record_undo(UndoStep::Rects(snap));
    }

    /// One step onto the trail, unless an undo or redo is what caused it.
    pub fn record_undo(&mut self, step: UndoStep) {
        if self.undoing {
            return;
        }
        self.layout_undo.push(step);
        if self.layout_undo.len() > LAYOUT_UNDO_DEPTH {
            self.layout_undo.remove(0);
        }
        self.layout_redo.clear();
    }

    /// A change that already happened, with the rects from before it (a
    /// resize or group drag reports its start rects at the drop): the
    /// snapshot is the present with those put back.
    pub fn remember_layout_from(&mut self, before: &[(String, Rect)]) {
        let ws = self.active_workspace.clone().unwrap_or_default();
        let snap = LayoutSnapshot {
            workspace_id: ws.clone(),
            rects: self
                .cards
                .iter()
                .filter(|c| c.workspace_id == ws)
                .map(|c| {
                    let was = before.iter().find(|(id, _)| *id == c.id).map(|(_, r)| *r);
                    (c.id.clone(), was.unwrap_or(c.rect))
                })
                .collect(),
        };
        if snap
            .rects
            .iter()
            .all(|(id, r)| self.card(id).is_some_and(|c| c.rect == *r))
        {
            return;
        }
        self.record_undo(UndoStep::Rects(snap));
    }

    /// Cmd+Z: the last step back; Cmd+Shift+Z the other way. A move glides
    /// back, a closed card comes back to its slot, a card made goes.
    pub fn undo_layout(&mut self, redo: bool) {
        let popped = if redo {
            self.layout_redo.pop()
        } else {
            self.layout_undo.pop()
        };
        let Some(step) = popped else {
            self.notify(if redo {
                "nothing to redo"
            } else {
                "nothing to undo"
            });
            return;
        };
        self.undoing = true;
        let inverse = match step {
            UndoStep::Rects(snap) => self.apply_rects(snap),
            // Reopened, and nothing to redo: redoing a close would close
            // a card you may already be working in again.
            UndoStep::Closed(card) => {
                let card = *card;
                let id = card.id.clone();
                // Out of the reopen ring too, or Cmd+Ctrl+T brings a twin.
                self.closed.retain(|c| c.id != id);
                self.reopen_card(card);
                None
            }
        };
        self.undoing = false;
        if let Some(inverse) = inverse {
            if redo {
                self.layout_undo.push(inverse);
            } else {
                self.layout_redo.push(inverse);
            }
        }
    }

    /// Puts a snapshot's rects back and returns the step that reverses it.
    fn apply_rects(&mut self, snap: LayoutSnapshot) -> Option<UndoStep> {
        let present = LayoutSnapshot {
            workspace_id: snap.workspace_id.clone(),
            rects: self
                .cards
                .iter()
                .filter(|c| c.workspace_id == snap.workspace_id)
                .map(|c| (c.id.clone(), c.rect))
                .collect(),
        };
        let moved: Vec<String> = snap
            .rects
            .iter()
            .filter(|(id, r)| self.card(id).is_some_and(|c| c.rect != *r))
            .map(|(id, _)| id.clone())
            .collect();
        self.mark_swap(&moved);
        for (id, r) in &snap.rects {
            if let Some(c) = self.card_mut(id) {
                c.rect = *r;
            }
        }
        if self.active_workspace.as_deref() != Some(&snap.workspace_id) {
            self.show_workspace(&snap.workspace_id);
        }
        self.dirty_layout = true;
        self.reveal_focused();
        Some(UndoStep::Rects(present))
    }

    /// A closed card back on the canvas: where it was if that space is
    /// still free, else beside whatever is there now (the old rect is a
    /// preference, not a claim). `card.reopen` and an undone close share it.
    pub fn reopen_card(&mut self, mut card: Card) {
        // Already back (Alt+T reopened it, then an undo step names it too):
        // a second copy would be a twin on one session.
        if self.card(&card.id).is_some() {
            return;
        }
        let ws = self.active_workspace.clone().unwrap_or_default();
        card.workspace_id = ws.clone();
        let taken: Vec<Rect> = self
            .cards
            .iter()
            .filter(|c| c.workspace_id == ws)
            .map(|c| c.rect)
            .collect();
        if taken.iter().any(|r| rects_overlap(*r, card.rect)) {
            card.rect = self.next_slot(None, &[], &ws);
        }
        let id = card.id.clone();
        // A group that was dissolved while the card was away is not a group.
        if card
            .group_id
            .as_ref()
            .is_some_and(|g| self.group(g).is_none())
        {
            card.group_id = None;
        }
        // Its number may have gone to a newer card while it was away
        // (numbers are reused, `take_number`); two #7s would make the
        // number useless, so it takes a free one.
        if card.number == 0 || self.cards.iter().any(|c| c.number == card.number) {
            card.number = self.take_number();
        }
        self.unpark(&mut card);
        self.cards.push(card);
        self.set_focus(Some(&id));
        self.reveal_focused();
        self.dirty_layout = true;
    }

    /// The ghost of a dragged card, snapped to the grid's nearest slot of
    /// its own size (`slot_snap`), for the ui to draw and to drop.
    pub fn snap_ghost(&self, id: &str, free: Rect) -> Rect {
        if self.card(id).is_none() {
            return free;
        }
        crate::slot_snap::snap_ghost(
            free,
            self.default_size(),
            Point {
                x: HALF_CELL,
                y: HALF_CELL,
            },
            GUTTER,
        )
    }

    /// The slots of the ghost's size around it, for the drag to draw
    /// (`slot_snap::slots_near`).
    pub fn ghost_slots(&self, ghost: Rect) -> Vec<Rect> {
        crate::slot_snap::slots_near(
            ghost,
            self.default_size(),
            Point {
                x: HALF_CELL,
                y: HALF_CELL,
            },
            GUTTER,
            GHOST_SLOT_REACH,
        )
    }

    /// Whether `rect` is free for `id` on its canvas: no other card, no
    /// other group's frame. What the drag's ghost is coloured by.
    pub fn rect_free_for(&self, id: &str, rect: Rect) -> bool {
        let Some(card) = self.card(id) else {
            return false;
        };
        let ws = card.workspace_id.clone();
        let mut occupied: Vec<Rect> = self
            .cards
            .iter()
            .filter(|c| c.workspace_id == ws && c.id != id && !self.paired(id, &c.id))
            .map(|c| c.rect)
            .collect();
        occupied.extend(self.other_frames(card.group_id.as_deref(), &ws));
        !occupied.iter().any(|o| rects_overlap(rect, *o))
    }

    /// The end of a single card's drag. Onto free space: the card goes to
    /// where the ghost was, gliding. Onto another card (the ghost's centre
    /// inside it): the two swap rects, the way Cmd+Alt+Shift+Arrow swaps
    /// neighbours. Half over a card and half over space: nothing moves and
    /// the status bar says why.
    pub fn drop_card(&mut self, id: &str, ghost: Rect) -> bool {
        let ghost = snap_rect(ghost);
        let Some(card) = self.card(id).cloned() else {
            return false;
        };
        if ghost == card.rect {
            return true;
        }
        if self.rect_free_for(id, ghost) {
            self.remember_layout();
            self.mark_swap(std::slice::from_ref(&id.to_string()));
            if let Some(c) = self.card_mut(id) {
                c.rect = ghost;
                // Moved away from whatever it was split from.
                c.soft_group_id = None;
            }
            self.dirty_layout = true;
            return true;
        }
        let centre = Point {
            x: ghost.x + ghost.w / 2.,
            y: ghost.y + ghost.h / 2.,
        };
        let under = self
            .cards
            .iter()
            .find(|c| {
                c.workspace_id == card.workspace_id
                    && c.id != id
                    && !self.paired(id, &c.id)
                    && centre.x >= c.rect.x
                    && centre.x < c.rect.x + c.rect.w
                    && centre.y >= c.rect.y
                    && centre.y < c.rect.y + c.rect.h
            })
            .cloned();
        let Some(other) = under else {
            self.notify("cards cannot overlap");
            return false;
        };
        self.remember_layout();
        self.mark_swap(&[id.to_string(), other.id.clone()]);
        if let Some(c) = self.card_mut(id) {
            c.rect = other.rect;
            c.soft_group_id = None;
        }
        if let Some(o) = self.card_mut(&other.id) {
            o.rect = card.rect;
            o.soft_group_id = None;
        }
        self.dirty_layout = true;
        true
    }

    pub fn gesture_overlaps(&self, ids: &[String]) -> bool {
        let ws = self.active_workspace.clone().unwrap_or_default();
        let group = ids
            .first()
            .and_then(|id| self.card(id))
            .and_then(|c| c.group_id.clone());
        let mut occupied: Vec<Rect> = self
            .cards
            .iter()
            .filter(|c| {
                c.workspace_id == ws
                    && !ids.contains(&c.id)
                    && !ids.iter().any(|i| self.paired(i, &c.id))
            })
            .map(|c| c.rect)
            .collect();
        occupied.extend(self.other_frames(group.as_deref(), &ws));
        ids.iter()
            .filter_map(|id| self.card(id))
            .any(|c| occupied.iter().any(|o| rects_overlap(c.rect, *o)))
    }

    fn page_zoom(&self, card: &Card) -> f64 {
        card.zoom.unwrap_or(self.config.browser.zoom)
    }

    /// Page zoom per browser card, Safari's steps; the config's `browser.zoom`
    /// is the starting point and what reset returns to.
    /// Back and forward go to the page, which is the only thing that knows
    /// where it has been: the model keeps no browser history of its own.
    /// A card that is not a browser says so rather than doing nothing.
    pub fn browser_history(&mut self, action: BrowserAction) {
        self.with_active_card(|m, id| {
            let Some(card) = m.card(&id).cloned() else {
                return;
            };
            if card.kind != CardKind::Browser {
                m.notify("not a browser card");
                return;
            }
            m.effects.push(Effect::Browser {
                card_id: id,
                action,
            });
        });
    }

    pub fn browser_zoom(&mut self, factor: Option<f64>) {
        self.with_active_card(|m, id| {
            let Some(card) = m.card(&id).cloned() else {
                return;
            };
            if card.kind != CardKind::Browser {
                return;
            }
            let next = factor.map(|f| {
                ((m.page_zoom(&card) * f * 100.).round() / 100.)
                    .clamp(BROWSER_ZOOM_MIN, BROWSER_ZOOM_MAX)
            });
            if let Some(c) = m.card_mut(&id) {
                c.zoom = next;
            }
            m.dirty_layout = true;
        });
    }

    /// Swaps the active card with its neighbour, trading whole rects. Focus
    /// stays on the card that moved, so pressing the same key twice walks
    /// one card past two neighbours.
    pub fn swap(&mut self, dir: Direction) {
        self.with_active_card(|m, id| {
            let Some(card) = m.card(&id).cloned() else {
                return;
            };
            m.remember_layout();
            let here = m.here();
            let placed = m.placed(&here);
            let ws = m.active_workspace.clone().unwrap_or_default();
            let Some(swap) = swap_with_neighbour(
                &placed,
                &id,
                dir,
                GUTTER,
                &m.other_frames(card.group_id.as_deref(), &ws),
            ) else {
                return;
            };
            // Armed BEFORE the rects change, so the move animates.
            let mut ids = vec![id.clone()];
            if let Some(b) = &swap.b {
                ids.push(b.id.clone());
            }
            m.mark_swap(&ids);
            if let Some(c) = m.card_mut(&id) {
                c.rect = swap.a.rect;
            }
            if let Some(b) = swap.b {
                if let Some(other) = m.card_mut(&b.id) {
                    other.rect = b.rect;
                }
            }
            m.dirty_layout = true;
            m.reveal_focused(); // the card may have swapped off screen
        });
    }

    fn sidebar_by(&mut self, delta: f64) {
        self.with_active_card(|m, id| {
            let Some(c) = m.card_mut(&id) else { return };
            if matches!(c.kind, CardKind::Terminal | CardKind::Browser) {
                return;
            }
            let extent = sidebar_extent(
                crate::grid::Size {
                    w: c.rect.w,
                    h: c.rect.h,
                },
                c.sidebar_top,
            );
            c.sidebar = Some(clamp_sidebar(
                sidebar_width(c.sidebar, extent) + delta,
                extent,
            ));
            m.dirty_layout = true;
        });
    }

    /// The placement menu's `run`, and the slot menu's.
    /// The palette's answer to "take me to that card": switch to its
    /// workspace if it is in another, focus it, and fit it, which is what
    /// Cmd+1 does once you are there.
    pub fn go_to_card(&mut self, card_id: &str) {
        let Some(card) = self.card(card_id).cloned() else {
            return;
        };
        if self.active_workspace.as_deref() != Some(card.workspace_id.as_str()) {
            self.show_workspace(&card.workspace_id);
        }
        self.set_focus(Some(&card.id));
        self.effects
            .push(Effect::RunCommand("canvas.zoom.fitCard".into()));
    }

    /// A snippet row chosen: the text goes to the focused card as a paste;
    /// the edit row opens the snippets folder in an editor card beside it,
    /// its tree listing the files, or focuses the card already showing it.
    fn paste_snippet(&mut self, id: &str) {
        if id == crate::snippets::EDIT_ROW {
            let dir = crate::config_files::snippets_dir()
                .to_string_lossy()
                .into_owned();
            let existing = self
                .here()
                .into_iter()
                .find(|c| c.kind == CardKind::Editor && c.root.as_deref() == Some(dir.as_str()))
                .map(|c| c.id.clone());
            match existing {
                Some(id) => self.set_focus(Some(&id)),
                None => {
                    let after = self.focused().map(|c| c.id.clone());
                    self.open_in_card(open_plan(&dir, PathKind::Directory, None), after.as_deref());
                }
            }
            return;
        }
        let Some(text) = self
            .snippets
            .iter()
            .find(|s| s.name == id)
            .map(|s| s.text.clone())
        else {
            return;
        };
        let Some(card_id) = self.selection.focused_id.clone() else {
            return;
        };
        self.effects.push(Effect::PasteText { card_id, text });
    }

    pub fn palette_run(&mut self, source: Source, id: &str) {
        match source {
            Source::Commands => {
                if let Some(card_id) = id.strip_prefix(CARD_ROW) {
                    self.go_to_card(card_id);
                } else if let Some(ws) = id.strip_prefix(WORKSPACE_ROW) {
                    self.show_workspace(ws);
                } else {
                    self.effects.push(Effect::RunCommand(id.to_string()));
                }
            }
            // Already applied by the preview; this makes it outlive the
            // session, in the settings file where `theme` already lives.
            Source::Themes => self.effects.push(Effect::SaveSetting {
                path: "theme".into(),
                value: serde_json::Value::String(id.into()),
            }),
            Source::Placement => match id {
                "slot" => self.pick_slot(),
                "here" => self
                    .effects
                    .push(Effect::RunCommand("card.new.terminal".into())),
                "file" => self
                    .effects
                    .push(Effect::RunCommand("card.open.file".into())),
                "untitled" => self
                    .effects
                    .push(Effect::RunCommand("card.new.editor".into())),
                "diff" => self
                    .effects
                    .push(Effect::RunCommand("card.open.diff".into())),
                "browser" => self
                    .effects
                    .push(Effect::RunCommand("card.new.browser".into())),
                "claude" => self
                    .effects
                    .push(Effect::RunCommand("card.new.claude".into())),
                "pi" => self.effects.push(Effect::RunCommand("card.new.pi".into())),
                _ => self
                    .effects
                    .push(Effect::RunCommand("card.new.ungrouped".into())),
            },
            Source::Sizes => {
                if let Some((_, _, w, h)) = SIZES.iter().find(|(sid, ..)| *sid == id) {
                    self.resize_active(*w, *h);
                }
            }
            Source::Snippets => self.paste_snippet(id),
            Source::MoveTo => self.move_selection_to(id),
            Source::RemoteColor => self.remote_color_run(id),
            Source::SlotKind => match id {
                "terminal" => {
                    self.fill_phantom(CardKind::Terminal, None);
                }
                "editor" => {
                    self.fill_phantom(CardKind::Editor, None);
                }
                _ => {
                    self.open_omnibox_for_new_card(true);
                }
            },
        }
    }
}

pub fn register(r: &mut crate::commands::CommandRegistry<Model>) {
    r.register(
        "card.split.right",
        "Split card, new card to the right",
        |m| m.split(SplitSide::Right),
    );
    r.register("card.split.down", "Split card, new card below", |m| {
        m.split(SplitSide::Down)
    });
    // Joins the active card's group: a new pane opens in the tab you were in.
    r.register("card.new.terminal", "Terminal: new card", |m| {
        m.new_card(true)
    });
    // Without this there is no keyboard way to escape a group.
    r.register(
        "card.new.ungrouped",
        "Terminal: new card outside any group",
        |m| m.new_card(false),
    );
    r.register("card.close", "Card: close", Model::close_selected);
    r.register(
        "card.close.leave",
        "Card: close, leave the space free",
        |m| m.close_selected_with(false),
    );
    r.register("card.save", "Editor: save the file", |m| {
        m.for_selected(|m, id| {
            let Some(card) = m.card(&id) else { return };
            if card.kind == CardKind::Diff {
                m.notify("a diff is read-only; Cmd+click the path to edit");
                return;
            }
            if card.kind != CardKind::Editor {
                return;
            }
            // Untitled: ask where first. The answer re-enters as a save.
            if card.path.is_none() {
                m.open_save_as(id, None);
                return;
            }
            m.effects.push(Effect::Editor {
                card_id: id,
                action: EditorAction::Save,
            })
        })
    });
    // The macOS open panel, not a typed path: typing paths was tiring and
    // the panel has search, recents and the sidebar (#64). `ift <path>`
    // still opens a typed one.
    r.register("card.open.file", "Editor: open a file", |m| {
        let from = m.selection.focused_id.clone();
        m.effects.push(Effect::PickFile { from });
    });
    // The agents with an adapter: each gets a new-card entry, in the active
    // card's directory like any new card.
    r.register(
        "card.new.claude",
        "Terminal: Claude Code in this directory",
        |m| m.new_beside_active(CardKind::Terminal, Some("claude"), None),
    );
    r.register("card.new.pi", "Terminal: Pi in this directory", |m| {
        m.new_beside_active(CardKind::Terminal, Some("pi"), None)
    });
    // The path comes from the agent's own hook events; a card that has not
    // run one says so instead of guessing a file.
    r.register(
        "card.transcript",
        "Terminal: show the agent's transcript",
        |m| {
            m.with_active_card(|m, id| {
                let Some(card) = m.card(&id).cloned() else {
                    return;
                };
                match card.transcript_path {
                    Some(path) => {
                        m.open_in_card(transcript_plan(&path, &card.cwd), Some(&id));
                    }
                    None => m.notify("no agent transcript for this card yet"),
                }
            })
        },
    );
    r.register("card.new.editor", "Editor: new untitled", |m| {
        m.new_beside_active(CardKind::Editor, None, None)
    });
    // Cmd+L. Not a prompt: the omnibox ranks history, open cards and what
    // you typed.
    r.register("card.omnibox", "Browser: address bar", Model::open_omnibox);
    // The placement menu's "browser": a NEW card beside the active one,
    // never editing it even when the active card is itself a browser
    // (`open_omnibox`'s own rule, and wrong here), so this is its own
    // entry point rather than `card.omnibox` with different arguments.
    r.register("card.new.browser", "Browser: open a URL", |m| {
        if m.browser_refused() {
            return;
        }
        m.open_omnibox_for_new_card(false);
    });
    r.register("browser.navigate", "Browser: go to a URL", |m| {
        m.with_active_card(|m, id| {
            let Some(card) = m.card(&id).cloned() else {
                return;
            };
            if card.kind != CardKind::Browser {
                m.notify("not a browser card");
                return;
            }
            let current = card.url.unwrap_or_default();
            m.prompt.ask(
                "url",
                if current.is_empty() {
                    "https://"
                } else {
                    &current
                },
                Pending::NavigateBrowser(id),
            );
        })
    });
    r.register(
        "browser.external",
        "Browser: open the page in the system browser",
        |m| {
            m.with_active_card(|m, id| {
                match m.card(&id).and_then(|c| {
                    (c.kind == CardKind::Browser)
                        .then(|| c.url.clone())
                        .flatten()
                }) {
                    Some(url) => m.effects.push(Effect::OpenUrl(url)),
                    None => m.notify("not a browser card"),
                }
            })
        },
    );
    // A focused browser card gives every key to the page; this is the one
    // chord that comes back out, to the nearest other card.
    r.register("browser.leave", "Browser: leave the page", |m| {
        m.with_active_card(|m, id| {
            let Some(card) = m.card(&id).cloned() else {
                return;
            };
            if card.kind != CardKind::Browser {
                return;
            }
            let others: Vec<crate::cards::PlacedCard> = m
                .here()
                .into_iter()
                .filter(|c| c.id != id)
                .map(|c| crate::cards::PlacedCard {
                    id: c.id.clone(),
                    rect: c.rect,
                    group_id: c.group_id.clone(),
                })
                .collect();
            let next = nearest_to(&others, card.rect).map(|c| c.id.clone());
            m.set_focus(next.as_deref());
        })
    });
    // The page's own history, not the canvas's: a misclick has to be
    // undoable or a card feels worse than a tab, which is the whole
    // argument for cards.
    r.register("browser.back", "Browser: back", |m| {
        m.browser_history(BrowserAction::Back)
    });
    r.register("browser.forward", "Browser: forward", |m| {
        m.browser_history(BrowserAction::Forward)
    });
    // The undo for a close. Without it, closing is a decision rather than a
    // reflex, and a canvas whose cards nobody dares close fills up exactly
    // the way a tab bar does.
    r.register("card.reopen", "Card: reopen the last closed one", |m| {
        let Some(card) = m.closed.pop() else {
            m.notify("nothing to reopen");
            return;
        };
        // Reopened by hand: the close leaves the trail, or a later Cmd+Z
        // would reopen it a second time.
        let id = card.id.clone();
        m.layout_undo
            .retain(|s| !matches!(s, UndoStep::Closed(c) if c.id == id));
        m.reopen_card(card);
    });
    r.register("browser.find", "Browser: find in page", Model::open_find);
    r.register("card.find", "Card: find in this card", Model::find_in_card);
    r.register("terminal.font.bigger", "Terminal: bigger font", |m| {
        m.step_terminal_font(1.)
    });
    r.register("terminal.font.smaller", "Terminal: smaller font", |m| {
        m.step_terminal_font(-1.)
    });
    r.register("terminal.font.reset", "Terminal: default font size", |m| {
        let d = crate::config::default_config().terminal.font_size;
        m.set_terminal_font(d)
    });
    r.register(
        "terminal.visual",
        "Terminal: visual mode, a cursor over the output",
        |m| {
            m.with_active_card(|m, id| {
                if m.card(&id).map(|c| c.kind) == Some(CardKind::Terminal) {
                    m.effects.push(Effect::Visual(id));
                } else {
                    m.notify("visual mode works in terminals");
                }
            })
        },
    );
    r.register(
        "card.findSelection",
        "Find: use selection for find",
        Model::find_selection,
    );
    r.register("browser.reload", "Browser: reload the page", |m| {
        m.browser_history(BrowserAction::Reload)
    });
    // The only way a page's address leaves the app without being retyped.
    r.register("browser.copyUrl", "Browser: copy the page's address", |m| {
        m.with_active_card(|m, id| {
            let url = m
                .card(&id)
                .filter(|c| c.kind == CardKind::Browser)
                .and_then(|c| c.url.clone());
            match url {
                Some(url) => {
                    m.notify("address copied");
                    m.effects.push(Effect::Copy(url));
                }
                None => m.notify("not a browser card"),
            }
        })
    });
    r.register("browser.zoom.in", "Browser: zoom the page in", |m| {
        m.browser_zoom(Some(ZOOM_STEP))
    });
    r.register("browser.zoom.out", "Browser: zoom the page out", |m| {
        m.browser_zoom(Some(1. / ZOOM_STEP))
    });
    r.register(
        "browser.zoom.reset",
        "Browser: page at the default zoom",
        |m| m.browser_zoom(None),
    );
    r.register("editor.find", "Editor: find", |m| {
        m.editor_action(EditorAction::Find)
    });
    r.register("editor.goToLine", "Editor: go to a line", |m| {
        m.with_active_card(|m, id| {
            if m.card(&id).is_some_and(|c| c.kind == CardKind::Editor) {
                m.prompt.ask("go to line", "", Pending::GoToLine(id));
            }
        })
    });
    // Palette-only: no chord of its own (Batch 1, 2026-09-24). Each runs
    // over the selection, or the whole document when there is none
    // (`EditorBody::apply_transform`).
    for (id, label, t) in [
        (
            "editor.transform.upper",
            "Editor: UPPERCASE",
            TextTransform::Upper,
        ),
        (
            "editor.transform.lower",
            "Editor: lowercase",
            TextTransform::Lower,
        ),
        (
            "editor.transform.title",
            "Editor: Title Case",
            TextTransform::Title,
        ),
        (
            "editor.transform.snake",
            "Editor: snake_case",
            TextTransform::Snake,
        ),
        (
            "editor.transform.kebab",
            "Editor: kebab-case",
            TextTransform::Kebab,
        ),
        (
            "editor.transform.camel",
            "Editor: camelCase",
            TextTransform::Camel,
        ),
        (
            "editor.transform.sortLines",
            "Editor: Sort Lines",
            TextTransform::SortLines,
        ),
        (
            "editor.transform.uniqueLines",
            "Editor: Unique Lines",
            TextTransform::UniqueLines,
        ),
        (
            "editor.transform.reverseLines",
            "Editor: Reverse Lines",
            TextTransform::ReverseLines,
        ),
        (
            "editor.transform.trimTrailingWhitespace",
            "Editor: Trim Trailing Whitespace",
            TextTransform::TrimTrailingWhitespace,
        ),
        (
            "editor.transform.indentTabsToSpaces",
            "Editor: Indentation to Spaces",
            TextTransform::IndentTabsToSpaces,
        ),
        (
            "editor.transform.indentSpacesToTabs",
            "Editor: Indentation to Tabs",
            TextTransform::IndentSpacesToTabs,
        ),
    ] {
        r.register(id, label, move |m| {
            m.editor_action(EditorAction::Transform(t))
        });
    }
    for (dir, dx, dy) in [
        (Direction::Left, -MOVE_STEP, 0.),
        (Direction::Right, MOVE_STEP, 0.),
        (Direction::Up, 0., -MOVE_STEP),
        (Direction::Down, 0., MOVE_STEP),
    ] {
        r.register(
            &format!("card.move.{}", dir_name(dir)),
            &format!("Card: nudge {}", super::context::where_(dir)),
            move |m| {
                m.with_active_card(|m, id| {
                    m.remember_layout();
                    if let Some(c) = m.card_mut(&id) {
                        c.rect = moved_by(c.rect, dx, dy);
                    }
                    m.dirty_layout = true;
                    m.reveal_focused(); // a card nudged past the edge should not vanish
                })
            },
        );
    }
    // The way back up from a quarter, without the picker: the card grows
    // from its corner into whatever is free beside and below it, up to
    // the default size. A quarter next to a half becomes the other half.
    r.register("card.size.reset", "Card: fill the free space", |m| {
        m.with_active_card(|m, id| {
            let Some(card) = m.card(&id).cloned() else {
                return;
            };
            let mut taken: Vec<Rect> = m
                .here()
                .iter()
                .filter(|c| c.id != id)
                .map(|c| c.rect)
                .collect();
            taken.extend(m.other_frames(Some(&id), &card.workspace_id));
            let next = fill_from_corner(card.rect, m.default_size(), &taken, GUTTER);
            if next == card.rect {
                m.notify("no room to grow");
                return;
            }
            m.remember_layout();
            m.mark_swap(std::slice::from_ref(&id));
            if let Some(c) = m.card_mut(&id) {
                c.rect = next;
                c.soft_group_id = None;
            }
            m.dirty_layout = true;
            m.reveal_focused();
        })
    });
    // One chord and one Enter to a quarter, instead of two splits and two
    // Cmd+Ctrl+W. Shrinking leaves the freed space free.
    r.register("layout.undo", "Layout: undo the last move or resize", |m| {
        m.undo_layout(false)
    });
    r.register("layout.redo", "Layout: redo", |m| m.undo_layout(true));
    // The snippet picker. The file is re-read on every open so an edit in
    // the editor card the last row opens is live on the next Cmd+Ctrl+S.
    r.register("snippet.paste", "Snippet: paste…", |m| {
        if m.palette.source == Some(Source::Snippets) {
            m.close_palette(false);
        } else {
            m.effects.push(Effect::RefreshSnippets);
            m.open_palette(Source::Snippets);
        }
    });
    r.register("card.size", "Card: resize to…", |m| {
        if m.palette.source == Some(Source::Sizes) {
            m.close_palette(false);
        } else if m.selection.focused_id.is_some() {
            m.open_palette(Source::Sizes);
        }
    });
    // Grows and shrinks from the bottom-right, so the top-left corner stays put.
    for (name, dw, dh) in [
        ("wider", RESIZE_STEP, 0.),
        ("narrower", -RESIZE_STEP, 0.),
        ("taller", 0., RESIZE_STEP),
        ("shorter", 0., -RESIZE_STEP),
    ] {
        r.register(
            &format!("card.resize.{name}"),
            &format!("Card: make {name}"),
            move |m| {
                m.with_active_card(|m, id| {
                    m.remember_layout();
                    if let Some(c) = m.card_mut(&id) {
                        c.rect = resized_by(c.rect, dw, dh);
                    }
                    m.dirty_layout = true;
                    m.reveal_focused();
                })
            },
        );
    }
    for dir in [
        Direction::Left,
        Direction::Right,
        Direction::Up,
        Direction::Down,
    ] {
        r.register(
            &format!("card.swap.{}", dir_name(dir)),
            &format!("Card: swap with the one {}", super::context::where_(dir)),
            move |m| m.swap(dir),
        );
    }
    r.register("card.sidebar.wider", "Card: sidebar wider", |m| {
        m.sidebar_by(RESIZE_STEP)
    });
    r.register("card.sidebar.narrower", "Card: sidebar narrower", |m| {
        m.sidebar_by(-RESIZE_STEP)
    });
    // Gmail's reading pane: the list above the message rather than beside it.
    r.register("card.sidebar.flip", "Card: sidebar beside or above", |m| {
        m.with_active_card(|m, id| {
            if let Some(c) = m.card_mut(&id) {
                if !matches!(c.kind, CardKind::Terminal | CardKind::Browser) {
                    c.sidebar_top = !c.sidebar_top;
                }
            }
            m.dirty_layout = true;
        })
    });
    r.register("card.maximize.toggle", "Card: maximise / restore", |m| {
        if m.selection.focused_id.is_some() {
            m.selection.maximized = !m.selection.maximized;
        }
    });
    // Somebody is reading your screen. The card keeps running under a decoy
    // that looks like work; Enter or Escape (input.rs) or the chord again
    // takes it away. Every selected card, so a split masks as one.
    // A maximised card is the one most worth masking, so not `for_selected`.
    // The card that must not go: a long job, a session you keep coming
    // back to. Every selected card, so a split pair locks as one.
    // You have seen it: the ring and the tab dot go grey until the card
    // has something new to say (the next hook, a long or failed command).
    // Palette only, as Ekin asked (2026-09-27).
    r.register("card.clearState", "Card: clear the state colour", |m| {
        let ids = m.selected_ids();
        let mut cleared = 0;
        for id in &ids {
            if let Some(c) = m.card_mut(id) {
                if c.agent != crate::agent_state::AgentState::None {
                    c.agent = crate::agent_state::AgentState::None;
                    cleared += 1;
                }
            }
        }
        if cleared > 0 {
            m.effects.push(Effect::AgentLog(format!(
                "{} card(s)  cleared by hand -> none",
                cleared
            )));
        }
    });
    r.register("card.protect", "Card: lock against closing / unlock", |m| {
        for id in m.selected_ids() {
            let on = !m.card(&id).is_some_and(|c| c.protected);
            if let Some(c) = m.card_mut(&id) {
                c.protected = on;
            }
            m.dirty_layout = true;
        }
    });
    r.register("card.mask", "Card: mask with a decoy / unmask", |m| {
        for id in m.selected_ids() {
            let on = !m.card(&id).is_some_and(|c| c.masked);
            m.set_mask(&id, on);
        }
    });
    r.register("card.place", "Card: new… (choose where)", |m| {
        if m.palette.source == Some(Source::Placement) {
            m.close_palette(false);
        } else {
            m.open_palette(Source::Placement);
        }
    });
    // iTerm2's clear: clears the emulator rather than writing `clear` to the
    // shell, which may be running something that would eat the input. In an
    // editor the same key shows or hides the file tree.
    r.register("card.clear", "Terminal: clear", |m| {
        m.for_selected(|m, id| {
            let Some(card) = m.card(&id) else { return };
            match (card.kind, card.pane_id) {
                (CardKind::Editor | CardKind::Diff, _) => m.effects.push(Effect::Editor {
                    card_id: id,
                    action: EditorAction::ToggleExplorer,
                }),
                (_, Some(pane)) => m.effects.push(Effect::ClearPane(pane)),
                _ => {}
            }
        })
    });
    // An editor diffs its file; anything else diffs its directory.
    r.register("card.open.diff", "Diff: this card against git HEAD", |m| {
        m.with_active_card(|m, id| {
            let Some(card) = m.card(&id).cloned() else {
                return;
            };
            let plan = match &card.path {
                Some(p) => diff_plan(p, PathKind::File),
                None => diff_plan(&card.cwd, PathKind::Directory),
            };
            m.open_in_card(plan, Some(&id));
        })
    });
    r.register("editor.blame", "Diff: show or hide git blame", |m| {
        m.with_active_card(|m, id| match m.card(&id).map(|c| c.kind) {
            Some(CardKind::Diff) => m.effects.push(Effect::Editor {
                card_id: id,
                action: EditorAction::ToggleBlame,
            }),
            _ => m.notify("blame lives in a diff card: ift diff <file>"),
        })
    });
    r.register(
        "editor.explorer",
        "Editor: show or hide the file tree",
        |m| m.editor_action(EditorAction::ToggleExplorer),
    );
    // The card is captured in the pending value, so a rename still lands on
    // the right one if focus moves while the prompt is open.
    r.register("card.rename", "Card: rename", |m| {
        let Some(card) = m.focused().cloned() else {
            return;
        };
        let label = m.label_of(&card);
        m.prompt
            .ask("card name", &label, Pending::RenameCard(card.id));
    });
}

pub fn dir_name(dir: Direction) -> &'static str {
    match dir {
        Direction::Left => "left",
        Direction::Right => "right",
        Direction::Up => "up",
        Direction::Down => "down",
    }
}

/// The smallest and largest terminal font a key can reach: `config.rs`
/// clamps `terminal.fontSize` to the same range.
const TERMINAL_FONT_MIN: f64 = 6.;
const TERMINAL_FONT_MAX: f64 = 96.;

impl Model {
    /// Cmd+= / Cmd+-: one point at a time, for every terminal card, written
    /// to settings.json so it survives a relaunch. The ui measures the cell
    /// from the config each frame, so the cards refit at once.
    pub fn step_terminal_font(&mut self, by: f64) {
        let next = (self.config.terminal.font_size + by).round();
        self.set_terminal_font(next);
    }

    fn set_terminal_font(&mut self, size: f64) {
        let size = size.clamp(TERMINAL_FONT_MIN, TERMINAL_FONT_MAX);
        if size == self.config.terminal.font_size {
            return;
        }
        self.config.terminal.font_size = size;
        self.effects.push(Effect::SaveSetting {
            path: "terminal.fontSize".into(),
            value: serde_json::json!(size),
        });
        self.notify(format!("terminal font {size} pt"));
    }
}

impl Model {
    /// What the open panel picked (`Effect::PickFile`): a file opens in an
    /// editor card, a folder in one with its tree, beside `from`, exactly as
    /// `ift <path>` would.
    pub fn open_picked(&mut self, path: &str, from: Option<&str>) -> Option<String> {
        let kind = if std::path::Path::new(path).is_dir() {
            PathKind::Directory
        } else {
            PathKind::File
        };
        self.open_in_card(open_plan(path, kind, None), from)
    }
}
