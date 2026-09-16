//! Card commands: new, split, close, the editor and browser actions, move,
//! resize, swap, sidebar, maximise, place, clear, rename. Port of
//! `commands/cards.ts`. Labels read `Domain: what it does`; a command lives
//! in the module whose prefix it carries, and `card.clear` is `Terminal:`.
use super::palette_state::Source;
use super::{BrowserAction, Card, EditorAction, Effect, Model, NewCard, Pending};
use crate::card_label::{card_label, Labelled};
use crate::cards::GUTTER;
use crate::config::{BROWSER_ZOOM_MAX, BROWSER_ZOOM_MIN};
use crate::grid::{snap_rect, Rect};
use crate::ift::{diff_plan, open_plan, transcript_plan, PathKind};
use crate::layout::rects_overlap;
use crate::navigate::{nearest_to, Direction};
use crate::resize::{moved_by, resized_by, MOVE_STEP, RESIZE_STEP};
use crate::saved_layout::CardKind;
use crate::sidebar::{clamp_sidebar, sidebar_extent, sidebar_width};
use crate::split::{split_rect, SplitSide};
use crate::swap::swap_with_neighbour;

/// How long the second close press has to arrive.
const DISCARD_ARM_MS: f64 = 3000.;

pub const ZOOM_STEP: f64 = 1.1;

impl Model {
    pub fn label_of(&self, card: &Card) -> String {
        card_label(
            &Labelled {
                title: &card.title,
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
    /// The directory is inherited either way: wanting a card outside the
    /// group is a statement about grouping, not about where you work.
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
                &self.cwd_beside(from.as_ref()),
                NewCard {
                    avoid: self.other_frames(group_id.as_deref(), &ws),
                    group_id,
                    workspace_id: Some(ws),
                    after: from.as_ref().map(|c| c.rect),
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
                after: from.as_ref().map(|c| c.rect),
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
        let mut ids = self.selected_ids();
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
        if let Some(dirty) = ids
            .iter()
            .find(|id| self.card(id).is_some_and(|c| c.dirty))
            .cloned()
        {
            if self.discard_armed.as_ref().map(|(id, _)| id) != Some(&dirty) {
                self.discard_armed = Some((dirty, self.now_ms + DISCARD_ARM_MS));
                self.notify("unsaved changes: save first, or close again to discard");
                return;
            }
        }
        self.discard_armed = None;
        for id in ids.iter().skip(1).rev() {
            self.close_card(id, false);
        }
        if let Some(first) = ids.first() {
            self.close_card(first, false);
        }
        if let Some(p) = pair {
            self.config_pairs.retain(|q| q.ids != p.ids);
            if let Some(back) = p.return_to.filter(|id| self.card(id).is_some()) {
                self.set_focus(Some(&back));
                self.reveal_focused();
            }
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
    /// `exists` is asked of the filesystem by the caller for `OpenFile`.
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
            Pending::OpenFile { from } => {
                let Some(raw) = text else { return };
                let from_card = from.as_deref().and_then(|id| self.card(id)).cloned();
                let full = self.resolve_typed_path(&raw, from_card.as_ref());
                if !exists(&full) {
                    self.notify(format!("{raw}: no such file"));
                    return;
                }
                self.open_in_card(open_plan(&full, PathKind::File, None), from.as_deref());
            }
            Pending::NewBrowserUrl { from, fill_phantom } => {
                let Some(url) = Model::normalise_url(text.as_deref()) else {
                    return;
                };
                if fill_phantom {
                    self.fill_phantom(CardKind::Browser, Some(url));
                } else {
                    let _ = from;
                    self.new_beside_active(CardKind::Browser, None, Some(url));
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
            Pending::SaveAs(id) => {
                let Some(raw) = text.filter(|t| !t.trim().is_empty()) else {
                    return;
                };
                let from = self.card(&id).cloned();
                let full = self.resolve_typed_path(raw.trim(), from.as_ref());
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
        let group = ids
            .first()
            .and_then(|id| self.card(id))
            .and_then(|c| c.group_id.clone());
        let mut occupied: Vec<Rect> = self
            .cards
            .iter()
            .filter(|c| c.workspace_id == ws && !ids.contains(&c.id))
            .map(|c| c.rect)
            .collect();
        occupied.extend(self.other_frames(group.as_deref(), &ws));
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
    pub fn gesture_overlaps(&self, ids: &[String]) -> bool {
        let ws = self.active_workspace.clone().unwrap_or_default();
        let group = ids
            .first()
            .and_then(|id| self.card(id))
            .and_then(|c| c.group_id.clone());
        let mut occupied: Vec<Rect> = self
            .cards
            .iter()
            .filter(|c| c.workspace_id == ws && !ids.contains(&c.id))
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
    pub fn palette_run(&mut self, source: Source, id: &str) {
        match source {
            Source::Commands => self.effects.push(Effect::RunCommand(id.to_string())),
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
            Source::SlotKind => match id {
                "terminal" => {
                    self.fill_phantom(CardKind::Terminal, None);
                }
                "editor" => {
                    self.fill_phantom(CardKind::Editor, None);
                }
                _ => {
                    self.prompt.ask(
                        "url",
                        "https://",
                        Pending::NewBrowserUrl {
                            from: None,
                            fill_phantom: true,
                        },
                    );
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
                m.prompt.ask("save as", "", Pending::SaveAs(id));
                return;
            }
            m.effects.push(Effect::Editor {
                card_id: id,
                action: EditorAction::Save,
            })
        })
    });
    r.register("card.open.file", "Editor: open a file", |m| {
        let from = m.selection.focused_id.clone();
        m.prompt.ask("file to open", "", Pending::OpenFile { from });
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
    // you typed, and `card.new.browser` stays for the palette's plain ask.
    r.register("card.omnibox", "Browser: address bar", Model::open_omnibox);
    r.register("card.new.browser", "Browser: open a URL", |m| {
        let from = m.selection.focused_id.clone();
        m.prompt.ask(
            "url",
            "https://",
            Pending::NewBrowserUrl {
                from,
                fill_phantom: false,
            },
        );
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
        let Some(mut card) = m.closed.pop() else {
            m.notify("nothing to reopen");
            return;
        };
        // Back where it was if that space is still free, else beside
        // whatever is there now: the old rect is a preference, not a claim.
        let ws = m.active_workspace.clone().unwrap_or_default();
        card.workspace_id = ws.clone();
        let taken: Vec<Rect> = m
            .cards
            .iter()
            .filter(|c| c.workspace_id == ws)
            .map(|c| c.rect)
            .collect();
        if taken.iter().any(|r| rects_overlap(*r, card.rect)) {
            card.rect = m.next_slot(None, &[], &ws, Some(card.rect));
        }
        let id = card.id.clone();
        // A group that was dissolved while the card was away is not a group.
        if card.group_id.as_ref().is_some_and(|g| m.group(g).is_none()) {
            card.group_id = None;
        }
        m.cards.push(card);
        m.set_focus(Some(&id));
        m.reveal_focused();
        m.dirty_layout = true;
    });
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
                    if let Some(c) = m.card_mut(&id) {
                        c.rect = moved_by(c.rect, dx, dy);
                    }
                    m.dirty_layout = true;
                    m.reveal_focused(); // a card nudged past the edge should not vanish
                })
            },
        );
    }
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
