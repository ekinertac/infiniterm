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
        let area = match self.hit(self.to_content(e.position)) {
            Hit::CardBody { id, .. } => {
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
                        Area::Editor
                    }
                    // A page's right-click is Chromium's (browsers.rs).
                    CardKind::Browser => return,
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
        self.show_menu(area, cx);
    }

    /// The menu for a workspace tab, switching to it first so its commands act
    /// on it.
    pub fn open_tab_menu(&mut self, workspace_id: &str, cx: &mut Context<Self>) {
        self.model.show_workspace(workspace_id);
        self.perform_effects();
        self.show_menu(Area::Tab, cx);
    }

    /// The right-clicked card is the one the menu's commands act on.
    fn focus_for_menu(&mut self, id: &str) {
        if self.model.selection.focused_id.as_deref() != Some(id) {
            self.model.set_focus(Some(id));
        }
    }

    fn show_menu(&mut self, area: Area, cx: &mut Context<Self>) {
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
            return;
        }
        let mut ids = vec![];
        let native = to_native(&rows, &mut ids);
        // Not here: see the header. The choice is run once the menu is gone.
        cx.spawn(async move |view, cx| {
            let chosen = native_menu::pop_up(&native);
            if let Some(id) = chosen.and_then(|tag| ids.get(tag as usize).cloned()) {
                let _ = view.update(cx, |this, cx| {
                    this.run_command(&id);
                    cx.notify();
                });
            }
        })
        .detach();
        self.redraw = true;
    }
}
