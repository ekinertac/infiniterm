//! The template: the title bar with the workspace tabs, the canvas element,
//! the status bar, and the overlays (palette, name prompt, shortcuts panel).
//! Port of `App.svelte`'s markup and of `Palette.svelte`, `NamePrompt.svelte`,
//! `ShortcutPanel.svelte`, `WorkspaceTabs.svelte`, `StatusBar.svelte`,
//! `TitleBar.svelte`.
//!
//! Plain div trees. The palette and the prompt take the keys while open
//! (`input.rs` routes them here), so no card is listening and Enter, Escape
//! and the arrows are safe despite the cmd-only rule. Attention on a tab is
//! a dot, never a switch: green for `idle` (waiting on you), a dimmer orange
//! for `working`, never merged.
use crate::AppView;
use gpui::{
    canvas, div, prelude::*, px, Context, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Render, ScrollWheelEvent, Window,
};
use infiniterm_core::format_zoom::format_zoom;
use infiniterm_core::palette::{highlight, rank, sectionise, step_index, RankedItem, MAX_RESULTS};
use infiniterm_core::palette_usage::{recent_keys, usage_bonus, use_key, RECENT_LIMIT};
use infiniterm_core::shortcuts::{filter_shortcuts, shortcut_sections, GESTURES};
use infiniterm_core::workspaces::{waiting_count, working_count};

/// Rows the palette shows at once; the selection is kept inside the window.
const PALETTE_ROWS: usize = 14;

/// The workspace tabs' label size, shared by the "+" new-tab control.
const TAB_LABEL_FONT_PX: f32 = 12.;
/// A workspace's waiting/working dot: a hint, not a badge.
const TAB_DOT_PX: f32 = 6.;
/// The working dot is dimmer than the waiting one, so a card still running
/// doesn't visually shout as loud as one already asking for you.
const TAB_WORKING_DOT_ALPHA: f32 = 0.6;
/// The title bar's tabs start clear of the traffic lights.
const TITLE_BAR_TRAFFIC_LIGHT_INSET_PX: f32 = 84.;
/// Below this the status bar's fps reads as a stall, not a frame rate.
const LOW_FPS_THRESHOLD: f32 = 50.;
/// A key cap: the panel's own ground for ink on a near-white cap, which
/// inverts with the theme (a light theme's `text_bright` is dark, and the
/// cap goes dark with light ink). Shared by the palette's hints and the
/// shortcuts panel's rows.
pub fn key_cap_box(key: &str, chrome: &crate::chrome::Chrome, ui: f32) -> gpui::Div {
    gpui::div()
        .px(px(KEY_CAP_PAD_X_PX * ui))
        .py(px(KEY_CAP_PAD_Y_PX * ui))
        .rounded_sm()
        .bg(chrome.text_bright)
        .text_size(px(KEY_CAP_FONT_PX * ui))
        .text_color(chrome.bar_bg)
        .child(key.to_string())
}

/// What the palette calls everything that is not one of the five most
/// recently used entries.
const PALETTE_REST_TITLE: &str = "all";
/// A palette section heading ("recent", and so on).
pub const PALETTE_SECTION_FONT_PX: f32 = 10.;
/// The text inside a keycap badge, in a palette hint or the shortcuts
/// panel. A key cap is read, not scanned: at 10 px against the panel it was
/// the least legible thing in the app.
const KEY_CAP_FONT_PX: f32 = 12.;
/// A key cap's padding. Enough that a single letter is a key and not a
/// character that happens to have a border.
const KEY_CAP_PAD_X_PX: f32 = 6.;
const KEY_CAP_PAD_Y_PX: f32 = 1.;
/// A secondary caption below an overlay's title: the prompt's label, and
/// the shortcuts panel's section and gesture headings.
const CAPTION_FONT_PX: f32 = 11.;
/// An overlay's body text: the palette's rows, the prompt's field, the
/// shortcuts list.
pub const OVERLAY_BODY_FONT_PX: f32 = 13.;
/// The palette and shortcuts panel sit this far from the top, clear of the
/// title bar.
pub const OVERLAY_TOP_PAD_PX: f32 = 80.;
const PALETTE_WIDTH_PX: f32 = 640.;
/// The prompt sits lower than the palette: it interrupts one action, not a
/// search.
const PROMPT_TOP_PAD_PX: f32 = 120.;
const PROMPT_WIDTH_PX: f32 = 480.;
/// The dialog's insides: padding, the gap between its rows, and the buttons.
const DIALOG_PAD_PX: f32 = 16.;
const DIALOG_GAP_PX: f32 = 10.;
const DIALOG_BUTTON_GAP_PX: f32 = 8.;
const DIALOG_BUTTON_PAD_X_PX: f32 = 12.;
const DIALOG_BUTTON_PAD_Y_PX: f32 = 6.;
const DIALOG_KEY_PAD_PX: f32 = 5.;
/// The key cap's ground on a button: dark enough for white ink on orange.
const DIALOG_KEY_BG_ALPHA: f32 = 0.35;
/// The shortcuts list scrolls past this height rather than growing the
/// window to fit every command.
const SHORTCUTS_LIST_MAX_H_PX: f32 = 720.;
/// A shortcuts or gestures row's vertical padding.
const SHORTCUTS_ROW_PAD_PX: f32 = 4.;
const SHORTCUTS_WIDTH_PX: f32 = 680.;
/// The shortcuts panel's own title, larger than its rows: read first.
const PANEL_TITLE_FONT_PX: f32 = 15.;

impl AppView {
    fn command_labels(&self) -> Vec<(String, String)> {
        self.registry
            .all()
            .iter()
            .map(|c| (c.id.clone(), c.label.clone()))
            .collect()
    }

    /// The ranked list for the open source, sectioned into recent and rest.
    fn palette_rows(&self) -> (Vec<RankedItem>, usize, Vec<(String, usize)>) {
        let Some(source) = self.model.palette.source else {
            return (vec![], 0, vec![]);
        };
        let labels = self.command_labels();
        let labels_ref: Vec<(&str, &str)> = labels
            .iter()
            .map(|(i, l)| (i.as_str(), l.as_str()))
            .collect();
        let items = self.model.palette_items(source, &labels_ref);
        let usage = &self.model.usage;
        let ranked = rank(&items, &self.model.palette.query, MAX_RESULTS, |id| {
            usage_bonus(usage.get(&use_key(source.id(), id)))
        });
        let recent = recent_keys(usage, RECENT_LIMIT);
        // Both halves are titled: the five you actually use, then everything
        // else. Without a heading on the second one the list read as one run
        // of commands whose order nobody could explain.
        let sections = sectionise(
            &ranked.items,
            &recent,
            |item| use_key(source.id(), &item.id),
            PALETTE_REST_TITLE,
        );
        let mut flat = vec![];
        let mut heads = vec![];
        for s in sections {
            if !s.title.is_empty() {
                heads.push((s.title.clone(), flat.len()));
            }
            flat.extend(s.items);
        }
        (flat, ranked.total, heads)
    }

    pub fn palette_key(&mut self, k: &Keystroke, cx: &mut gpui::App) {
        let Some(source) = self.model.palette.source else {
            return;
        };
        let (flat, _, _) = self.palette_rows();
        let index = self.model.palette.index.min(flat.len().saturating_sub(1));
        match k.key.as_str() {
            "escape" => self.model.close_palette(false),
            "enter" => {
                if let Some(item) = flat.get(index) {
                    let id = item.item.id.clone();
                    self.model.note_use(source, &id);
                    self.model.close_palette(true);
                    self.model.palette_run(source, &id);
                }
            }
            "down" => self.model.palette.index = step_index(index, flat.len(), 1),
            "up" => self.model.palette.index = step_index(index, flat.len(), -1),
            _ => {
                let paste = (k.modifiers.platform && k.key == "v")
                    .then(|| cx.read_from_clipboard().and_then(|c| c.text()))
                    .flatten();
                if let crate::field::Edit::Changed = self.query_field.key(k, paste.as_deref()) {
                    self.model.palette.query = self.query_field.text.clone();
                    self.model.palette.index = 0;
                }
            }
        }
        // Live preview follows the highlight, which is the point for themes.
        if self.model.palette.source == Some(source) {
            let (flat, _, _) = self.palette_rows();
            let index = self.model.palette.index.min(flat.len().saturating_sub(1));
            let id = flat.get(index).map(|i| i.item.id.clone());
            self.model.palette_preview(source, id.as_deref());
        }
        self.perform_effects();
    }

    /// The panel takes the keys while open: Escape closes, typing filters.
    pub fn shortcuts_key(&mut self, k: &Keystroke, cx: &mut gpui::App) {
        if k.key == "escape" {
            self.model.shortcuts_open = false;
            self.shortcuts_field = crate::field::Field::default();
            return;
        }
        let paste = (k.modifiers.platform && k.key == "v")
            .then(|| cx.read_from_clipboard().and_then(|c| c.text()))
            .flatten();
        self.shortcuts_field.key(k, paste.as_deref());
    }

    pub fn prompt_key(&mut self, k: &Keystroke, cx: &mut gpui::App) {
        let confirm = self.model.prompt.confirm;
        let settled = match k.key.as_str() {
            "escape" => self.model.prompt.settle(None),
            "enter" => {
                let v = self.model.prompt.value.clone();
                self.model.prompt.settle(Some(&v))
            }
            _ if !confirm => {
                let paste = (k.modifiers.platform && k.key == "v")
                    .then(|| cx.read_from_clipboard().and_then(|c| c.text()))
                    .flatten();
                if let crate::field::Edit::Changed = self.prompt_field.key(k, paste.as_deref()) {
                    self.model.prompt.value = self.prompt_field.text.clone();
                }
                None
            }
            _ => None,
        };
        if let Some((pending, text)) = settled {
            self.model
                .answer(pending, text, |path| std::path::Path::new(path).exists());
        }
        self.perform_effects();
    }
}

impl AppView {
    /// Keeps the two fields in step with the model's strings: a prompt that
    /// just opened gets its suggestion selected; a palette query that the
    /// model reset (it always resets on open) empties the field.
    fn sync_fields(&mut self) {
        let open = self.model.prompt.is_open();
        if open && !self.prompt_was_open {
            self.prompt_field = crate::field::Field::open(&self.model.prompt.value, true);
        }
        self.prompt_was_open = open;
        if !self.model.shortcuts_open && !self.shortcuts_field.text.is_empty() {
            self.shortcuts_field = crate::field::Field::default();
        }
        if self.model.palette.query != self.query_field.text {
            self.query_field = crate::field::Field::open(&self.model.palette.query, false);
        }
        // The command opens the box with the card's address already in it,
        // SELECTED, so the first character typed replaces the whole URL the
        // way it does in a browser.
        if self.model.omni.open && self.model.omni.query != self.omni_field.text {
            self.omni_field = crate::field::Field::open(&self.model.omni.query, true);
        }
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_fields();
        let chrome = self.chrome.clone();
        let entity = cx.entity();
        let title_bar = self.render_title_bar(cx);
        let status_bar = self.render_status_bar();
        let palette = self.model.palette_open().then(|| self.render_palette(cx));
        let prompt = self.model.prompt.is_open().then(|| self.render_prompt(cx));
        let shortcuts = self.model.shortcuts_open.then(|| self.render_shortcuts());
        let omnibox = self.model.omni.open.then(|| self.render_omnibox(cx));
        let find_bar = self.model.find.open.then(|| self.render_find_bar(cx));

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(chrome.canvas_bg)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if this.key_down(e, cx) {
                    // Marks the key equivalent handled, or macOS sends
                    // the same key again as a key down.
                    cx.stop_propagation();
                }
                cx.notify();
            }))
            .child(title_bar)
            .child(
                div()
                    .flex_1()
                    .relative()
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, e: &MouseDownEvent, window, cx| {
                            window.focus(&this.focus);
                            this.mouse_down(e);
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(|this, e: &MouseDownEvent, _, _| this.mouse_down(e)),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, e: &MouseUpEvent, _, _| this.mouse_up(e)),
                    )
                    .on_mouse_up(
                        MouseButton::Middle,
                        cx.listener(|this, e: &MouseUpEvent, _, _| this.mouse_up(e)),
                    )
                    .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, _| this.mouse_move(e)))
                    .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, _| this.wheel(e)))
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, _, window, cx| {
                                let more = entity.update(cx, |this, cx| {
                                    this.frame(bounds, window, cx);
                                    this.needs_frame()
                                });
                                // Only while something moves or arrives; an idle
                                // canvas draws nothing until the poll task or an
                                // event wakes it.
                                if more {
                                    window.request_animation_frame();
                                }
                            },
                        )
                        .size_full(),
                    )
                    .children(palette)
                    .children(prompt)
                    .children(shortcuts)
                    .children(omnibox)
                    .children(find_bar),
            )
            .child(status_bar)
    }
}

impl AppView {
    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // The interface multiplier (Cmd+Shift+= / -) reaches the tabs and the panels.
        let ui = self.model.ui_scale as f32;
        let chrome = &self.chrome;
        let active = self.model.active_workspace.clone();
        let mut tabs = div().flex().items_center().gap_1();
        for ws in &self.model.workspaces {
            let cards = self.model.cards_on(Some(&ws.id));
            let states: Vec<_> = cards.iter().map(|c| c.agent).collect();
            let waiting = waiting_count(&states);
            let working = working_count(&states);
            let is_active = active.as_deref() == Some(&ws.id);
            let id = ws.id.clone();
            let mut tab = div()
                .id(gpui::SharedString::from(format!("tab-{}", ws.id)))
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .py_1()
                .rounded_sm()
                .text_size(px(TAB_LABEL_FONT_PX * ui))
                .text_color(if is_active {
                    chrome.text_bright
                } else {
                    chrome.text_muted
                })
                .when(is_active, |d| d.bg(chrome.control_bg))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                        this.model.show_workspace(&id);
                        this.perform_effects();
                        cx.notify();
                    }),
                )
                .child(ws.name.clone());
            if waiting > 0 {
                tab = tab.child(
                    div()
                        .w(px(TAB_DOT_PX * ui))
                        .h(px(TAB_DOT_PX * ui))
                        .rounded_full()
                        .bg(gpui::rgb(0x5dcd97)),
                );
            }
            if working > 0 {
                tab = tab.child(
                    div()
                        .w(px(TAB_DOT_PX * ui))
                        .h(px(TAB_DOT_PX * ui))
                        .rounded_full()
                        .bg(crate::chrome::with_alpha(
                            chrome.agent_working,
                            TAB_WORKING_DOT_ALPHA,
                        )),
                );
            }
            tabs = tabs.child(tab);
        }
        tabs = tabs.child(
            div()
                .id("tab-new")
                .px_2()
                .py_1()
                .text_size(px(TAB_LABEL_FONT_PX * ui))
                .text_color(chrome.text_faint)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        this.run_command("workspace.new");
                        cx.notify();
                    }),
                )
                .child("+"),
        );
        div()
            .h(px(self.titlebar_h()))
            .w_full()
            .flex()
            .items_center()
            .pl(px(TITLE_BAR_TRAFFIC_LIGHT_INSET_PX))
            .pr_2()
            .bg(chrome.bar_bg)
            .border_b_1()
            .border_color(chrome.bar_border)
            .font_family("Menlo")
            .child(tabs)
    }

    fn render_status_bar(&self) -> impl IntoElement {
        let chrome = &self.chrome;
        let m = &self.model;
        let size = px((m.config.ui.status_bar_size * m.ui_scale) as f32);
        let mut left = format!(
            "{}   {} card{}",
            format_zoom(m.viewport.scale),
            m.cards.len(),
            if m.cards.len() == 1 { "" } else { "s" }
        );
        if let Some(c) = m.focused() {
            left.push_str(&format!("   {}", m.label_of(c)));
        }
        if m.selection.maximized {
            left.push_str("   maximised");
        }
        let notice = m.notice.clone().unwrap_or_default();
        let right = if m.config.ui.show_fps {
            format!("{:.0} fps", self.fps)
        } else {
            String::new()
        };
        let fps_color = if self.fps < LOW_FPS_THRESHOLD && self.fps > 0. {
            chrome.agent_idle
        } else {
            chrome.text_faint
        };
        div()
            .h(px(self.statusbar_h()))
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .px_2()
            .bg(chrome.bar_bg)
            .border_t_1()
            .border_color(chrome.bar_border)
            .font_family("Menlo")
            .text_size(size)
            .text_color(chrome.text_muted)
            .child(div().child(left))
            .child(div().text_color(chrome.text).child(notice))
            .child(div().text_color(fps_color).child(right))
    }

    fn render_palette(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // The interface multiplier (Cmd+Shift+= / -) reaches the tabs and the panels.
        let ui = self.model.ui_scale as f32;
        let chrome = &self.chrome;
        let Some(source) = self.model.palette.source else {
            return div();
        };
        let (flat, total, heads) = self.palette_rows();
        let index = self.model.palette.index.min(flat.len().saturating_sub(1));
        let start = index
            .saturating_sub(PALETTE_ROWS - 1)
            .min(flat.len().saturating_sub(PALETTE_ROWS));
        let query = self.model.palette.query.clone();
        let mut list = div().flex().flex_col();
        if flat.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .py_2()
                    .text_color(chrome.text_faint)
                    .child("nothing matches"),
            );
        }
        for (i, item) in flat.iter().enumerate().skip(start).take(PALETTE_ROWS) {
            if let Some((title, at)) = heads.iter().find(|(_, at)| *at == i) {
                list = list.child(
                    div()
                        .px_3()
                        .pt_2()
                        .pb_1()
                        // A rule above every heading but the first, so the
                        // split is visible before the words are read.
                        .when(*at > 0, |d| {
                            d.border_t_1().border_color(chrome.bar_border).mt_1()
                        })
                        .text_size(px(PALETTE_SECTION_FONT_PX * ui))
                        .text_color(chrome.text_faint)
                        .child(title.clone()),
                );
            }
            let selected = i == index;
            let id = item.item.id.clone();
            let mut label = div().flex().gap_0();
            for run in highlight(&item.item.label, &item.matched.matches) {
                label = label.child(
                    div()
                        .when(run.hit, |d| d.text_color(chrome.sel_bg))
                        .child(run.text),
                );
            }
            let mut row = div()
                .id(gpui::SharedString::from(format!("row-{id}")))
                .flex()
                .items_center()
                .justify_between()
                .px_3()
                .py_1()
                .when(selected, |d| d.bg(chrome.row_selected))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                        this.model.note_use(source, &id);
                        this.model.close_palette(true);
                        this.model.palette_run(source, &id);
                        this.perform_effects();
                        cx.notify();
                    }),
                )
                .child(label);
            if let Some(hint) = &item.item.hint {
                let mut keys = div().flex().gap_1();
                for key in hint.split(' ') {
                    keys = keys.child(key_cap_box(key, chrome, ui));
                }
                row = row.child(keys);
            }
            list = list.child(row);
        }
        let cut = total.saturating_sub(flat.len());
        if cut > 0 {
            // Said out loud: a list that silently stops looks like a list that ended.
            list = list.child(
                div()
                    .px_3()
                    .py_2()
                    .text_color(chrome.text_faint)
                    .child(format!("{cut} more, type to narrow")),
            );
        }
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .justify_center()
            .pt(px(OVERLAY_TOP_PAD_PX * ui))
            .bg(chrome.overlay_backdrop)
            .child(
                div()
                    .w(px(PALETTE_WIDTH_PX * ui))
                    .h(px(0.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .bg(chrome.overlay_bg)
                            .border_1()
                            .border_color(chrome.overlay_border)
                            .rounded_md()
                            .shadow_lg()
                            .font_family("Menlo")
                            .text_size(px(OVERLAY_BODY_FONT_PX * ui))
                            .text_color(chrome.text)
                            .child(
                                div()
                                    .px_3()
                                    .py_2()
                                    .border_b_1()
                                    .border_color(chrome.control_border)
                                    .text_color(if query.is_empty() {
                                        chrome.text_faint
                                    } else {
                                        chrome.text_bright
                                    })
                                    .child(if query.is_empty() {
                                        source.placeholder().to_string()
                                    } else {
                                        format!("{query}▏")
                                    }),
                            )
                            .child(list),
                    ),
            )
    }

    /// The dialog's verdict from a button: yes or no, then the answer.
    pub fn prompt_settle(&mut self, yes: bool) {
        let v = self.model.prompt.value.clone();
        let settled = self.model.prompt.settle(yes.then_some(v.as_str()));
        if let Some((pending, text)) = settled {
            self.model
                .answer(pending, text, |path| std::path::Path::new(path).exists());
        }
        self.perform_effects();
    }

    /// One dialog, three shapes: a text prompt (label, field, the two keys
    /// as a caption), a confirm (question, Cancel and the action as
    /// buttons) and an alert (message, OK). Centred, modal in look, and
    /// every button has its key beside it because the keyboard is how the
    /// app is used; the buttons are there so a mouse is not refused.
    fn render_prompt(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // The interface multiplier (Cmd+Shift+= / -) reaches the tabs and the panels.
        let ui = self.model.ui_scale as f32;
        let chrome = self.chrome.clone();
        let p = &self.model.prompt;
        let is_confirm = p.confirm && !p.alert;
        let is_alert = p.alert;
        let selected = self.prompt_field.selected && !p.confirm;
        let field = if selected {
            p.value.clone()
        } else {
            format!("{}▏", p.value)
        };
        // The key sits in a translucent cap so it reads on either button:
        // white ink on the dark Cancel, the same on the orange action.
        let key_cap = |label: &str, chrome: &crate::chrome::Chrome| {
            div()
                .px(px(DIALOG_KEY_PAD_PX * ui))
                .rounded_sm()
                .bg(crate::chrome::with_alpha(
                    gpui::black(),
                    DIALOG_KEY_BG_ALPHA,
                ))
                .text_color(chrome.text_bright)
                .text_size(px(KEY_CAP_FONT_PX * ui))
                .child(label.to_string())
        };
        let button = |label: String, key: &str, primary: bool, chrome: &crate::chrome::Chrome| {
            div()
                .flex()
                .items_center()
                .gap(px(DIALOG_BUTTON_GAP_PX * ui))
                .px(px(DIALOG_BUTTON_PAD_X_PX * ui))
                .py(px(DIALOG_BUTTON_PAD_Y_PX * ui))
                .rounded_md()
                .border_1()
                .border_color(if primary {
                    chrome.sel_bg
                } else {
                    chrome.control_border
                })
                .bg(if primary {
                    chrome.sel_bg
                } else {
                    chrome.control_bg
                })
                .text_color(if primary { chrome.sel_fg } else { chrome.text })
                .child(label)
                .child(key_cap(key, chrome))
        };
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(DIALOG_GAP_PX * ui))
            .p(px(DIALOG_PAD_PX * ui))
            .bg(chrome.overlay_bg)
            .border_1()
            .border_color(chrome.overlay_border)
            .rounded_lg()
            .shadow_lg()
            .font_family("Menlo")
            .text_size(px(OVERLAY_BODY_FONT_PX * ui));
        if is_confirm || is_alert {
            // The question, then the buttons: Cancel on the left, the verb on
            // the right in the selection colour, as macOS lays them out.
            body = body.child(div().text_color(chrome.text_bright).child(p.label.clone()));
            let mut row = div()
                .flex()
                .justify_end()
                .gap(px(DIALOG_BUTTON_GAP_PX * ui))
                .pt(px(DIALOG_GAP_PX * ui));
            if is_confirm {
                row = row.child(
                    button("Cancel".into(), "esc", false, &chrome).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            this.prompt_settle(false);
                            cx.notify();
                        }),
                    ),
                );
            }
            row = row.child(
                button(p.action.clone(), "enter", true, &chrome).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        this.prompt_settle(true);
                        cx.notify();
                    }),
                ),
            );
            body = body.child(row);
        } else {
            body = body
                .child(
                    div()
                        .text_color(chrome.text_muted)
                        .text_size(px(CAPTION_FONT_PX * ui))
                        .child(p.label.clone()),
                )
                .child(
                    div()
                        .flex()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(chrome.control_bg)
                        .border_1()
                        .border_color(chrome.control_border)
                        .text_color(chrome.text_bright)
                        .child(
                            div()
                                .when(selected, |d| d.bg(chrome.sel_bg).text_color(chrome.sel_fg))
                                .child(field),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(DIALOG_BUTTON_GAP_PX * ui))
                        .items_center()
                        .text_color(chrome.text_faint)
                        .text_size(px(KEY_CAP_FONT_PX * ui))
                        .child(key_cap("enter", &chrome))
                        .child("confirm")
                        .child(key_cap("esc", &chrome))
                        .child("cancel"),
                );
        }
        // A dim sheet over the canvas says "answer this first"; a click on
        // it is a cancel, the way a sheet's outside is.
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .justify_center()
            .items_start()
            .pt(px(PROMPT_TOP_PAD_PX * ui))
            .bg(chrome.overlay_backdrop)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.prompt_settle(false);
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(PROMPT_WIDTH_PX * ui))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()),
                    )
                    .child(body),
            )
    }

    /// The palette's shape and place, on purpose: the two are the same kind
    /// of thing, a searchable list of what the app can do. Taller, because
    /// this one is meant to be read as well as searched.
    fn render_shortcuts(&self) -> impl IntoElement {
        // The interface multiplier (Cmd+Shift+= / -) reaches the tabs and the panels.
        let ui = self.model.ui_scale as f32;
        let chrome = &self.chrome;
        let labels = self.command_labels();
        let labels_ref: Vec<(&str, &str)> = labels
            .iter()
            .map(|(i, l)| (i.as_str(), l.as_str()))
            .collect();
        let query = self.shortcuts_field.text.clone();
        let sections =
            filter_shortcuts(&shortcut_sections(&self.model.keymap, &labels_ref), &query);
        let key_box = |key: &str| key_cap_box(key, chrome, ui);
        let mut list = div()
            .id("shortcuts-list")
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .max_h(px(SHORTCUTS_LIST_MAX_H_PX * ui))
            .pb_2();
        for s in sections {
            list = list.child(
                div()
                    .px_4()
                    .pt_3()
                    .pb_1()
                    .text_size(px(CAPTION_FONT_PX * ui))
                    .text_color(chrome.agent_idle)
                    .child(s.title.to_uppercase()),
            );
            for sc in s.shortcuts {
                let mut keys = div().flex().gap_2();
                for chord in sc.chords {
                    let mut k = div().flex().gap_1();
                    for key in chord.split(' ') {
                        k = k.child(key_box(key));
                    }
                    keys = keys.child(k);
                }
                list = list.child(
                    div()
                        .flex()
                        .justify_between()
                        .px_4()
                        .py(px(SHORTCUTS_ROW_PAD_PX * ui))
                        .child(div().text_color(chrome.text).child(sc.label))
                        .child(keys),
                );
            }
        }
        // Gestures are not commands, so they are filtered here by the same rule.
        let q = query.trim().to_lowercase();
        let gestures: Vec<_> = GESTURES
            .iter()
            .filter(|(keys, label)| {
                q.is_empty()
                    || label.to_lowercase().contains(&q)
                    || keys.to_lowercase().contains(&q)
            })
            .collect();
        if !gestures.is_empty() {
            list = list.child(
                div()
                    .px_4()
                    .pt_3()
                    .pb_1()
                    .text_size(px(CAPTION_FONT_PX * ui))
                    .text_color(chrome.agent_idle)
                    .child("GESTURES"),
            );
            for (keys, label) in gestures {
                list = list.child(
                    div()
                        .flex()
                        .justify_between()
                        .px_4()
                        .py(px(SHORTCUTS_ROW_PAD_PX * ui))
                        .child(div().text_color(chrome.text).child(*label))
                        .child(div().text_color(chrome.text_mid).child(*keys)),
                );
            }
        }
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .justify_center()
            .pt(px(OVERLAY_TOP_PAD_PX * ui))
            .bg(chrome.overlay_backdrop)
            .child(
                div().w(px(SHORTCUTS_WIDTH_PX * ui)).h(px(0.)).child(
                    div()
                        .flex()
                        .flex_col()
                        .bg(chrome.overlay_bg)
                        .border_1()
                        .border_color(chrome.overlay_border)
                        .rounded_md()
                        .shadow_lg()
                        .font_family("Menlo")
                        .text_size(px(OVERLAY_BODY_FONT_PX * ui))
                        .child(
                            div()
                                .px_4()
                                .py_3()
                                .border_b_1()
                                .border_color(chrome.card_border)
                                .text_size(px(PANEL_TITLE_FONT_PX * ui))
                                .text_color(if query.is_empty() {
                                    chrome.text_faint
                                } else {
                                    chrome.text_bright
                                })
                                .child(if query.is_empty() {
                                    "Filter shortcuts".to_string()
                                } else {
                                    format!("{query}\u{258f}")
                                }),
                        )
                        .child(list),
                ),
            )
    }
}
