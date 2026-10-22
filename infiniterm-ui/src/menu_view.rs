//! The right-click menu the app draws itself where there is no `NSMenu`
//! (Linux and Windows, #464): the rows `Model::menu` holds, one panel per
//! open level, the selection lit, each row's shortcut on its right.
//!
//! Nothing here decides anything: which rows a click gets is
//! `infiniterm_core::context_menu`, walking them is `menu_nav.rs`, the keys
//! are `menus.rs::menu_key` (from `input.rs`), and the choice runs through
//! `menus.rs::close_menu`. Drawn by `overlays.rs`'s `render` over the whole
//! window, so a tab menu in the title bar and a card menu on the canvas
//! share one coordinate space: window pixels.
//!
//! The mouse: a row under the pointer is selected (a submenu opens), a click
//! chooses, a press anywhere outside the panels closes the menu and is
//! swallowed, so a right-click outside does not open another menu in the
//! same press. Sizes are screen pixels times `ui.scale`, like the palette.
use crate::AppView;
use gpui::{div, prelude::*, px, Context, IntoElement, MouseButton, MouseDownEvent, Window};
use infiniterm_core::context_menu::Row;
use infiniterm_core::menu_nav::{place, place_submenu, Outcome};
use infiniterm_core::shortcuts::{chord_keys, ModifierLabels};

/// One row's height and a separator's, before `ui.scale`.
const ROW_PX: f32 = 24.;
const SEPARATOR_PX: f32 = 9.;
/// Inside a panel: the space around the rows, and each row's side padding.
const PANEL_PAD_PX: f32 = 4.;
const ROW_PAD_PX: f32 = 12.;
/// The gap between a title and its shortcut, and the narrowest panel.
const CHORD_GAP_PX: f32 = 32.;
const MIN_WIDTH_PX: f32 = 180.;
/// The arrow a submenu row ends with.
const SUBMENU_ARROW: &str = "\u{203a}";

fn press_outside(
    this: &mut AppView,
    _: &MouseDownEvent,
    _: &mut Window,
    cx: &mut Context<AppView>,
) {
    this.close_menu(None);
    cx.stop_propagation();
    cx.notify();
}

fn row_height(row: &Row, ui: f32) -> f32 {
    match row {
        Row::Separator => SEPARATOR_PX * ui,
        _ => ROW_PX * ui,
    }
}

impl AppView {
    /// The text a row shows on its right: its shortcut as keys, or a
    /// submenu's arrow.
    fn menu_hint(&self, row: &Row) -> Option<String> {
        let labels = ModifierLabels::new(self.model.config.keyboard.command_modifier);
        match row {
            Row::Item { chord: Some(c), .. } => Some(chord_keys(c, labels).join("+")),
            Row::Submenu { .. } => Some(SUBMENU_ARROW.to_string()),
            _ => None,
        }
    }

    /// One panel's width: the widest title plus the widest hint, measured in
    /// the chrome font.
    fn menu_width(&self, rows: &[Row], font_px: f32, ui: f32, window: &Window) -> f32 {
        let font = &self.chrome.typography.regular;
        let measure = |t: &str| {
            f32::from(crate::text::shape(window, t, px(font_px), font, self.chrome.text).width)
        };
        let title_w = rows
            .iter()
            .filter_map(|r| match r {
                Row::Item { title, .. } | Row::Submenu { title, .. } => Some(measure(title)),
                Row::Separator => None,
            })
            .fold(0., f32::max);
        let hint_w = rows
            .iter()
            .filter_map(|r| self.menu_hint(r).map(|h| measure(&h)))
            .fold(0., f32::max);
        (title_w + hint_w + (CHORD_GAP_PX + 2. * ROW_PAD_PX + 2. * PANEL_PAD_PX) * ui)
            .max(MIN_WIDTH_PX * ui)
    }

    pub fn render_menu(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = self.model.ui_scale as f32;
        let chrome = &self.chrome;
        let font_px = chrome
            .typography
            .size(crate::overlays::OVERLAY_BODY_FONT_PX)
            * ui;
        let viewport = window.viewport_size();
        let win = (f32::from(viewport.width), f32::from(viewport.height));
        // A press outside the panels closes the menu, whichever button, and
        // goes no further.
        let mut root = div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(press_outside))
            .on_mouse_down(MouseButton::Right, cx.listener(press_outside))
            .on_mouse_down(MouseButton::Middle, cx.listener(press_outside));
        let Some(menu) = &self.model.menu else {
            return root;
        };
        let levels = menu.nav.levels();
        let mut origin = (0., 0.);
        let mut parent_w = 0.;
        for (level, (rows, lit)) in levels.iter().enumerate() {
            let width = self.menu_width(rows, font_px, ui, window);
            let height: f32 =
                rows.iter().map(|r| row_height(r, ui)).sum::<f32>() + 2. * PANEL_PAD_PX * ui;
            origin = if level == 0 {
                place(menu.at, (width, height), win)
            } else {
                // Level with the row that opened it, in the panel above.
                let (above, open) = &levels[level - 1];
                let row_y = origin.1
                    + PANEL_PAD_PX * ui
                    + above
                        .iter()
                        .take(open.unwrap_or(0))
                        .map(|r| row_height(r, ui))
                        .sum::<f32>();
                place_submenu(origin.0, parent_w, row_y, (width, height), win)
            };
            parent_w = width;
            let mut panel = div()
                .absolute()
                .left(px(origin.0))
                .top(px(origin.1))
                .w(px(width))
                .py(px(PANEL_PAD_PX * ui))
                .flex()
                .flex_col()
                .bg(chrome.overlay_bg)
                .border_1()
                .border_color(chrome.overlay_border)
                .rounded_md()
                .shadow_lg()
                .occlude()
                .font_family(chrome.typography.family.clone())
                .font_weight(chrome.typography.regular.weight)
                .text_size(px(font_px))
                .text_color(chrome.text)
                // A press on the panel itself (a separator, the padding) is
                // not a press outside it.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation());
            for (i, row) in rows.iter().enumerate() {
                if let Row::Separator = row {
                    panel = panel.child(
                        div()
                            .h(px(SEPARATOR_PX * ui))
                            .flex()
                            .items_center()
                            .child(div().w_full().h(px(1.)).bg(chrome.overlay_border)),
                    );
                    continue;
                }
                let (title, enabled) = match row {
                    Row::Item { title, enabled, .. } => (title.clone(), *enabled),
                    Row::Submenu { title, .. } => (title.clone(), true),
                    Row::Separator => unreachable!(),
                };
                let hint = self.menu_hint(row);
                panel = panel.child(
                    div()
                        .id(("menu-row", level * 1000 + i))
                        .h(px(ROW_PX * ui))
                        .px(px(ROW_PAD_PX * ui))
                        .mx(px(PANEL_PAD_PX * ui))
                        .rounded_sm()
                        .flex()
                        .items_center()
                        .justify_between()
                        .when(*lit == Some(i) && enabled, |d| d.bg(chrome.row_selected))
                        .when(!enabled, |d| d.text_color(chrome.text_faint))
                        .on_mouse_move(cx.listener(move |this, _, _, cx| {
                            let Some(menu) = this.model.menu.as_mut() else {
                                return;
                            };
                            let before = menu.nav.clone();
                            menu.nav.hover(level, i);
                            if menu.nav != before {
                                cx.notify();
                            }
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                let Some(menu) = this.model.menu.as_mut() else {
                                    return;
                                };
                                if let Outcome::Chosen(id) = menu.nav.click(level, i) {
                                    this.close_menu(Some(id));
                                }
                                cx.notify();
                            }),
                        )
                        .child(div().truncate().child(title))
                        .children(hint.map(|h| div().text_color(chrome.text_faint).child(h))),
                );
            }
            root = root.child(panel);
        }
        root
    }
}
