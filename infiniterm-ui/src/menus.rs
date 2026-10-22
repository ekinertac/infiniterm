//! The right-click menus: which menu a click opens, its rows, and running the
//! choice (#278).
//!
//! Called by `overlays.rs` (the canvas's right-click listener and each
//! workspace tab's). The menus are data in `infiniterm-core/src/context_menu.rs`
//! over registered commands; `native_menu.rs` presents them as an `NSMenu`.
//! This file connects the two to the app: it works out the area from what was
//! clicked, builds the rows from the live registry and keymap, and runs the
//! chosen command AFTER the menu is gone.
//!
//! Off macOS there is no `NSMenu` (#464): the same rows open as
//! `Model::menu`, drawn by `menu_view.rs` and walked with `menu_nav.rs`, and
//! `close_menu` runs the choice. Both paths end in `finish_menu`. The
//! browser page's own menu (`browsers.rs`) is not drawn this way: its rows
//! carry actions, not commands, and browser cards are not built on Linux.
//!
//! Constraints: the Mac popup blocks and re-enters gpui, so it runs from a
//! spawned task (a handler holds the app borrowed: `native_menu.rs`). A
//! terminal whose program asked for the mouse keeps its right-click unless
//! Shift is held, as `TerminalBody::mouse_down` already decides. A browser
//! card keeps its own menu (browsers.rs) for now.

use crate::input::Hit;
#[cfg(target_os = "macos")]
use crate::native_menu::{self, Row as NativeRow};
use crate::AppView;
use gpui::{Context, MouseDownEvent};
use infiniterm_core::context_menu::{self, Area, Row, Source};
use infiniterm_core::menu_nav::Outcome;
use infiniterm_core::saved_layout::CardKind;

/// The rows as the presenter takes them, each command row tagged with its
/// index in the returned list of command ids.
#[cfg(target_os = "macos")]
fn to_native(rows: &[Row], ids: &mut Vec<String>) -> Vec<NativeRow> {
    rows.iter()
        .map(|row| match row {
            Row::Separator => NativeRow::Separator,
            Row::Submenu { title, rows } => NativeRow::Submenu {
                title: title.clone(),
                rows: to_native(rows, ids),
            },
            Row::Item {
                command,
                title,
                chord,
                enabled,
            } => {
                ids.push(command.clone());
                NativeRow::Item {
                    tag: ids.len() as i64 - 1,
                    title: title.clone(),
                    enabled: *enabled,
                    chord: chord.clone(),
                }
            }
        })
        .collect()
}

impl AppView {
    /// A right-click on the canvas: the menu for what is under the pointer, or
    /// nothing when the click belongs to a program or a browser page.
    pub fn open_context_menu(&mut self, e: &MouseDownEvent, cx: &mut Context<Self>) {
        self.menu_anchor = (f32::from(e.position.x), f32::from(e.position.y));
        let mut lit: Option<String> = None;
        let hit = self.hit(self.to_content(e.position));
        // A card of a selection of several: the menu of what acts on all of
        // them, whichever body the pointer is over (#347).
        if let Hit::CardBody { id, .. } | Hit::CardEdge { id, .. } = &hit {
            if self.model.in_multi_selection(id) {
                self.show_menu(Area::Frame, None, None, cx);
                return;
            }
        }
        let area = match hit {
            Hit::CardBody { id, local } => {
                let Some(kind) = self.model.card(&id).map(|c| c.kind) else {
                    return;
                };
                match kind {
                    CardKind::Terminal => {
                        let program_has_mouse = self
                            .terminal_body(&id)
                            .is_some_and(|b| b.grid.wants_mouse());
                        if program_has_mouse && !e.modifiers.shift {
                            return;
                        }
                        self.focus_for_menu(&id);
                        Area::Terminal
                    }
                    CardKind::Editor => {
                        self.focus_for_menu(&id);
                        // A tab of the strip: switch to it, then its menu.
                        if let Some(tab) = self.editor_tabs_for(&id).and_then(|t| t.tab_at(local)) {
                            self.model.browser_tab_jump(&id, tab);
                            self.show_menu(Area::TabStrip, None, None, cx);
                            return;
                        }
                        // On a tree row: the file's menu, with the row lit
                        // until the menu closes.
                        let row = self.editor_tabs_for(&id).and_then(|t| t.tree_row_at(local));
                        match row {
                            Some(row) => {
                                if let Some(t) = self.editor_tabs_for(&id) {
                                    t.set_menu_row(Some(row));
                                }
                                lit = Some(id.clone());
                                Area::Tree
                            }
                            None => Area::Editor,
                        }
                    }
                    // A tab of the strip: its menu. A page's right-click is
                    // Chromium's (browsers.rs).
                    CardKind::Browser => {
                        let Some(tab) = self.browser_for(&id).and_then(|b| b.tab_at(local)) else {
                            return;
                        };
                        self.focus_for_menu(&id);
                        self.model.browser_tab_jump(&id, tab);
                        Area::TabStrip
                    }
                    _ => {
                        self.focus_for_menu(&id);
                        Area::Frame
                    }
                }
            }
            // The frame and the label open the same menu as the body: a card
            // is a card, and a label or border click is hard to discover.
            Hit::CardEdge { id, .. } | Hit::HeaderButton { id, .. } => {
                let kind = self.model.card(&id).map(|c| c.kind);
                self.focus_for_menu(&id);
                kind.map_or(Area::Frame, infiniterm_core::context_menu::card_area)
            }
            Hit::GroupTab(_) => return,
            Hit::Nothing => Area::Canvas,
        };
        self.show_menu(area, lit, None, cx);
    }

    /// The menu for a workspace tab. The tab is not switched to on the click:
    /// only a command chosen from the menu takes you there, so its commands
    /// act on it.
    pub fn open_tab_menu(
        &mut self,
        workspace_id: &str,
        e: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        self.menu_anchor = (f32::from(e.position.x), f32::from(e.position.y));
        self.show_menu(Area::Tab, None, Some(workspace_id.to_string()), cx);
    }

    /// After the menu is gone, on either path: run the chosen command on the
    /// workspace the menu was for, and put out the lit tree row.
    pub fn finish_menu(
        &mut self,
        chosen: Option<String>,
        workspace: Option<String>,
        lit: Option<String>,
    ) {
        // The command first: it acts on the lit row.
        if let Some(id) = chosen {
            let before = self.model.active_workspace.clone();
            if let Some(ws) = &workspace {
                self.model.show_workspace(ws);
                self.perform_effects();
            }
            self.run_command(&id);
            // Moving a tab is no reason to leave the one you are in.
            if let (true, Some(prev)) = (id.starts_with("workspace.reorder"), before) {
                self.model.show_workspace(&prev);
                self.perform_effects();
            }
        }
        self.clear_lit_row(lit);
    }

    fn clear_lit_row(&mut self, card: Option<String>) {
        if let Some(t) = card.and_then(|id| self.editor_tabs_for(&id)) {
            t.set_menu_row(None);
        }
        self.redraw = true;
    }

    /// The right-clicked card is the one the menu's commands act on.
    fn focus_for_menu(&mut self, id: &str) {
        // One of several selected cards: the selection stays (#346).
        if self.model.in_multi_selection(id) {
            return;
        }
        if self.model.selection.focused_id.as_deref() != Some(id) {
            self.model.set_focus(Some(id));
        }
    }

    /// `lit`: an editor card whose tree row is lit for this menu; cleared when it closes.
    fn show_menu(
        &mut self,
        area: Area,
        lit: Option<String>,
        workspace: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.sync_ui_context();
        let ctx = self.model.key_context();
        let rows = {
            let registry = &self.registry;
            let keymap = &self.model.keymap;
            context_menu::rows(
                area,
                &Source {
                    context: &ctx,
                    label: &|id| registry.get(id).map(|c| c.label.clone()),
                    chord: &|id| {
                        keymap
                            .iter()
                            .find(|(_, bound)| bound == id)
                            .map(|(chord, _)| chord.clone())
                    },
                },
            )
        };
        if rows.is_empty() {
            self.clear_lit_row(lit);
            return;
        }
        self.present_menu(rows, workspace, lit, cx);
        self.redraw = true;
    }

    /// macOS: an `NSMenu`. Not here: see the header. The choice is run once
    /// the menu is gone.
    #[cfg(target_os = "macos")]
    fn present_menu(
        &mut self,
        rows: Vec<Row>,
        workspace: Option<String>,
        lit: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let mut ids = vec![];
        let native = to_native(&rows, &mut ids);
        let wait_for_frame = lit.is_some();
        cx.spawn(async move |view, cx| {
            // The popup blocks the main loop, so a lit tree row is only seen
            // if a frame is drawn first: give the loop a few before it opens.
            if wait_for_frame {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(80))
                    .await;
            }
            let chosen = native_menu::pop_up(&native);
            let _ = view.update(cx, |this, cx| {
                let id = chosen.and_then(|tag| ids.get(tag as usize).cloned());
                this.finish_menu(id, workspace, lit);
                cx.notify();
            });
        })
        .detach();
    }

    /// Everywhere else: the app draws the menu itself (`menu_view.rs`), so it
    /// is state, not a blocking call. `finish_menu` runs when a row is chosen
    /// or the menu is dismissed.
    #[cfg(not(target_os = "macos"))]
    fn present_menu(
        &mut self,
        rows: Vec<Row>,
        workspace: Option<String>,
        lit: Option<String>,
        _cx: &mut Context<Self>,
    ) {
        self.model.menu = Some(infiniterm_core::menu_nav::OpenMenu {
            nav: infiniterm_core::menu_nav::MenuNav::new(rows),
            at: self.menu_anchor,
        });
        self.menu_pending = Some((workspace, lit));
    }

    /// The drawn menu closes: `chosen` runs, if anything was. The caller
    /// asks for a frame.
    pub fn close_menu(&mut self, chosen: Option<String>) {
        self.model.menu = None;
        let (workspace, lit) = self.menu_pending.take().unwrap_or((None, None));
        self.finish_menu(chosen, workspace, lit);
        self.redraw = true;
    }

    /// A key while a drawn menu is up. Every key is the menu's: the arrows
    /// walk it, Enter chooses, Escape and any other key close it, and none
    /// reaches the canvas (a Cmd+W aimed at the menu must not close the card
    /// behind it).
    pub fn menu_key(&mut self, key: &str) {
        let Some(menu) = self.model.menu.as_mut() else {
            return;
        };
        let chosen = match key {
            "down" => {
                menu.nav.down();
                None
            }
            "up" => {
                menu.nav.up();
                None
            }
            "right" => {
                menu.nav.right();
                None
            }
            "left" => {
                menu.nav.left();
                None
            }
            "enter" | "space" => match menu.nav.enter() {
                Outcome::Chosen(id) => Some(Some(id)),
                Outcome::Stay => None,
            },
            _ => Some(None),
        };
        match chosen {
            Some(id) => self.close_menu(id),
            None => self.redraw = true,
        }
    }
}
