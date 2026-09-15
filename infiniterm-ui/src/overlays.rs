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
use crate::{AppView, STATUSBAR_H, TITLEBAR_H};
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
        let sections = sectionise(
            &ranked.items,
            &recent,
            |item| use_key(source.id(), &item.id),
            "",
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
        if self.model.palette.query != self.query_field.text {
            self.query_field = crate::field::Field::open(&self.model.palette.query, false);
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
        let prompt = self.model.prompt.is_open().then(|| self.render_prompt());
        let shortcuts = self.model.shortcuts_open.then(|| self.render_shortcuts());

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(chrome.canvas_bg)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                this.key_down(e, cx);
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
                                entity.update(cx, |this, cx| this.frame(bounds, window, cx));
                                window.request_animation_frame();
                            },
                        )
                        .size_full(),
                    )
                    .children(palette)
                    .children(prompt)
                    .children(shortcuts),
            )
            .child(status_bar)
    }
}

impl AppView {
    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                .text_size(px(12.))
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
                        .w(px(6.))
                        .h(px(6.))
                        .rounded_full()
                        .bg(gpui::rgb(0x5dcd97)),
                );
            }
            if working > 0 {
                tab = tab.child(
                    div()
                        .w(px(6.))
                        .h(px(6.))
                        .rounded_full()
                        .bg(crate::chrome::with_alpha(chrome.agent_working, 0.6)),
                );
            }
            tabs = tabs.child(tab);
        }
        tabs = tabs.child(
            div()
                .id("tab-new")
                .px_2()
                .py_1()
                .text_size(px(12.))
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
            .h(px(TITLEBAR_H))
            .w_full()
            .flex()
            .items_center()
            .pl(px(84.)) // past the traffic lights
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
        let fps_color = if self.fps < 50. && self.fps > 0. {
            chrome.agent_idle
        } else {
            chrome.text_faint
        };
        div()
            .h(px(STATUSBAR_H))
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
            if let Some((title, _)) = heads.iter().find(|(_, at)| *at == i) {
                list = list.child(
                    div()
                        .px_3()
                        .pt_2()
                        .pb_1()
                        .text_size(px(10.))
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
                    keys = keys.child(
                        div()
                            .px_1()
                            .rounded_sm()
                            .bg(chrome.control_bg)
                            .border_1()
                            .border_color(chrome.control_border)
                            .text_size(px(10.))
                            .text_color(chrome.text_mid)
                            .child(key.to_string()),
                    );
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
            .pt(px(80.))
            .bg(chrome.overlay_backdrop)
            .child(
                div().w(px(640.)).h(px(0.)).flex().flex_col().child(
                    div()
                        .flex()
                        .flex_col()
                        .bg(chrome.bar_bg)
                        .border_1()
                        .border_color(chrome.control_border)
                        .rounded_md()
                        .font_family("Menlo")
                        .text_size(px(13.))
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

    fn render_prompt(&self) -> impl IntoElement {
        let chrome = &self.chrome;
        let p = &self.model.prompt;
        let selected = self.prompt_field.selected && !p.confirm;
        let field = if p.confirm {
            "Enter for yes, Escape for no".to_string()
        } else if selected {
            p.value.clone()
        } else {
            format!("{}▏", p.value)
        };
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .justify_center()
            .pt(px(120.))
            .child(
                div().w(px(480.)).h(px(0.)).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_3()
                        .bg(chrome.bar_bg)
                        .border_1()
                        .border_color(chrome.control_border)
                        .rounded_md()
                        .font_family("Menlo")
                        .text_size(px(13.))
                        .child(
                            div()
                                .text_color(chrome.text_muted)
                                .text_size(px(11.))
                                .child(p.label.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .px_2()
                                .py_1()
                                .bg(chrome.control_bg)
                                .text_color(chrome.text_bright)
                                .child(
                                    div()
                                        .when(selected, |d| {
                                            d.bg(chrome.sel_bg).text_color(chrome.sel_fg)
                                        })
                                        .child(field),
                                ),
                        ),
                ),
            )
    }

    fn render_shortcuts(&self) -> impl IntoElement {
        let chrome = &self.chrome;
        let labels = self.command_labels();
        let labels_ref: Vec<(&str, &str)> = labels
            .iter()
            .map(|(i, l)| (i.as_str(), l.as_str()))
            .collect();
        let sections = filter_shortcuts(&shortcut_sections(&self.model.keymap, &labels_ref), "");
        let mut body = div().flex().flex_col().gap_2();
        for s in sections {
            let mut sec = div().flex().flex_col().gap_0p5().child(
                div()
                    .text_color(chrome.text_faint)
                    .text_size(px(10.))
                    .child(s.title.to_uppercase()),
            );
            for sc in s.shortcuts {
                let mut row = div()
                    .flex()
                    .justify_between()
                    .child(div().text_color(chrome.text).child(sc.label));
                let mut keys = div().flex().gap_2();
                for chord in sc.chords {
                    let mut k = div().flex().gap_1();
                    for key in chord.split(' ') {
                        k = k.child(
                            div()
                                .px_1()
                                .rounded_sm()
                                .bg(chrome.control_bg)
                                .border_1()
                                .border_color(chrome.control_border)
                                .text_size(px(10.))
                                .text_color(chrome.text_mid)
                                .child(key.to_string()),
                        );
                    }
                    keys = keys.child(k);
                }
                row = row.child(keys);
                sec = sec.child(row);
            }
            body = body.child(sec);
        }
        let mut gestures = div().flex().flex_col().gap_0p5().child(
            div()
                .text_color(chrome.text_faint)
                .text_size(px(10.))
                .child("GESTURES"),
        );
        for (keys, label) in GESTURES {
            gestures = gestures.child(
                div()
                    .flex()
                    .justify_between()
                    .child(div().text_color(chrome.text).child(label))
                    .child(div().text_color(chrome.text_mid).child(keys)),
            );
        }
        body = body.child(gestures);
        div()
            .absolute()
            .top_0()
            .right_0()
            .h_full()
            .w(px(520.))
            .overflow_hidden()
            .p_3()
            .bg(chrome.bar_bg)
            .border_l_1()
            .border_color(chrome.control_border)
            .font_family("Menlo")
            .text_size(px(12.))
            .child(body)
    }
}
