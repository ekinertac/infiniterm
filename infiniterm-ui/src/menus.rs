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
//! Constraints: the popup blocks and re-enters gpui, so it runs from a
//! spawned task (a handler holds the app borrowed: `native_menu.rs`). A
//! terminal whose program asked for the mouse keeps its right-click unless
//! Shift is held, as `TerminalBody::mouse_down` already decides. A browser
//! card keeps its own menu (browsers.rs) for now.

use crate::input::Hit;
use crate::native_menu::{self, Row as NativeRow};
use crate::AppView;
use gpui::{Context, MouseDownEvent};
use infiniterm_core::context_menu::{self, Area, Row, Source};
use infiniterm_core::saved_layout::CardKind;

/// The rows as the presenter takes them, each command row tagged with its
/// index in the returned list of command ids.
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
        let mut lit: Option<String> = None;
        let area = match self.hit(self.to_content(e.position)) {
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
                            self.show_menu(Area::TabStrip, None, cx);
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
            Hit::CardEdge { id, .. } => {
                self.focus_for_menu(&id);
                Area::Frame
            }
            Hit::GroupTab(_) => return,
            Hit::Nothing => Area::Canvas,
        };
        self.show_menu(area, lit, cx);
    }

    /// The menu for a workspace tab, switching to it first so its commands act
    /// on it.
    pub fn open_tab_menu(&mut self, workspace_id: &str, cx: &mut Context<Self>) {
        self.model.show_workspace(workspace_id);
        self.perform_effects();
        self.show_menu(Area::Tab, None, cx);
    }

    fn clear_lit_row(&mut self, card: Option<String>) {
        if let Some(t) = card.and_then(|id| self.editor_tabs_for(&id)) {
            t.set_menu_row(None);
        }
        self.redraw = true;
    }

    /// The right-clicked card is the one the menu's commands act on.
    fn focus_for_menu(&mut self, id: &str) {
        if self.model.selection.focused_id.as_deref() != Some(id) {
            self.model.set_focus(Some(id));
        }
    }

    /// `lit`: an editor card whose tree row is lit for this menu; cleared when it closes.
    fn show_menu(&mut self, area: Area, lit: Option<String>, cx: &mut Context<Self>) {
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
        let mut ids = vec![];
        let native = to_native(&rows, &mut ids);
        // Not here: see the header. The choice is run once the menu is gone.
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
                // The command first: it acts on the lit row.
                if let Some(id) = chosen.and_then(|tag| ids.get(tag as usize).cloned()) {
                    this.run_command(&id);
                }
                this.clear_lit_row(lit);
                cx.notify();
            });
        })
        .detach();
        self.redraw = true;
    }
}
