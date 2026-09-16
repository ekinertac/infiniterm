//! The address bar over the canvas: Cmd+L. Draws the field, the inline
//! completion, the scope chip and the ranked sections, and routes its keys.
//!
//! Wiring only. Every decision is in `infiniterm-core/src/omni/` and the
//! state is `Model::omni`; this file knows how a section looks and nothing
//! about what is in it. Modelled on `render_palette` in overlays.rs, which
//! it deliberately resembles on screen: the same sheet, the same width, the
//! same row rhythm, so the two read as one family.
//!
//! Related: model/omni_cmd.rs, omni/rank.rs, overlays.rs (the palette and
//! the key caps), input.rs (which hands keys here while the box is open).
use crate::overlays::key_cap_box;
use crate::overlays::{OVERLAY_BODY_FONT_PX, OVERLAY_TOP_PAD_PX, PALETTE_SECTION_FONT_PX};
use crate::AppView;
use gpui::prelude::*;
use gpui::{div, px, Context, Keystroke, MouseButton, MouseDownEvent};
use infiniterm_core::omni::OmniKind;

/// The same width as the palette: the two overlays are siblings and a
/// different width would read as a different app.
pub const OMNIBOX_WIDTH_PX: f32 = 560.;
/// Rows drawn before the list is cut. More than this and the sheet reaches
/// the bottom of a laptop screen.
pub const OMNIBOX_ROWS: usize = 12;
/// A result's subtitle sits beside its title rather than under it: the box
/// is read in one glance, and two lines a row halves how many fit.
pub const OMNIBOX_SUBTITLE_ALPHA: f32 = 0.75;

/// One character per kind, in place of icons. A glyph nobody has to load and
/// nobody has to theme.
fn mark(kind: OmniKind) -> &'static str {
    match kind {
        OmniKind::Address => "→",
        OmniKind::Search => "?",
        OmniKind::History => "◷",
        OmniKind::Card => "▣",
    }
}

impl AppView {
    /// Keys while the omnibox is open. Escape leaves a scope before it
    /// closes the box, the way Backspace does, because losing a whole
    /// typed query to a stray Escape is worse than one extra press.
    pub fn omni_key(&mut self, k: &Keystroke, cx: &mut gpui::App) {
        match k.key.as_str() {
            "escape" => {
                if self.model.omni.scope.is_some() {
                    self.model.omni_unscope();
                } else {
                    self.model.close_omnibox();
                    self.omni_field = crate::field::Field::default();
                }
            }
            "enter" => {
                self.model.omni_enter();
                self.omni_field = crate::field::Field::default();
            }
            "down" => self.model.omni_step(1),
            "up" => self.model.omni_step(-1),
            "tab" => {
                self.model.omni_tab();
                self.omni_field = crate::field::Field::open(&self.model.omni.query, false);
            }
            // Backspace on an empty scoped field leaves the scope instead of
            // doing nothing, which is where the hand goes.
            "backspace" if self.model.omni.query.is_empty() && self.model.omni.scope.is_some() => {
                self.model.omni_unscope();
            }
            _ => {
                let paste = (k.modifiers.platform && k.key == "v")
                    .then(|| cx.read_from_clipboard().and_then(|c| c.text()))
                    .flatten();
                if let crate::field::Edit::Changed = self.omni_field.key(k, paste.as_deref()) {
                    let text = self.omni_field.text.clone();
                    self.model.omni_type(&text);
                }
            }
        }
        self.perform_effects();
    }

    pub fn render_omnibox(&self, cx: &mut Context<Self>) -> gpui::Div {
        let ui = self.model.ui_scale as f32;
        let chrome = &self.chrome;
        if !self.model.omni.open {
            return div();
        }
        let response = self.model.omni_response();
        let query = self.model.omni.query.clone();
        let index = self.model.omni.index;

        // The field: what was typed, then the rest of the completion in
        // faint ink behind it, so Enter's destination is readable before it
        // is pressed.
        let tail = response
            .completion
            .as_ref()
            .and_then(|c| {
                c.to_lowercase()
                    .starts_with(&query.trim().to_lowercase())
                    .then(|| c[query.trim().len()..].to_string())
            })
            .unwrap_or_default();
        let mut field = div().flex().items_center().gap_1();
        if let Some(keyword) = &self.model.omni.scope {
            let name =
                infiniterm_core::omni::engines::resolve(keyword, &self.model.config.engines())
                    .map(|e| e.name.clone())
                    .unwrap_or_else(|| keyword.clone());
            field = field.child(
                div()
                    .px_2()
                    .rounded_sm()
                    .bg(chrome.sel_bg)
                    .text_color(chrome.sel_fg)
                    .child(format!("search {name}")),
            );
        }
        field = field
            .child(
                div()
                    .text_color(if query.is_empty() {
                        chrome.text_faint
                    } else {
                        chrome.text_bright
                    })
                    .child(if query.is_empty() && self.model.omni.scope.is_none() {
                        "search or enter address".to_string()
                    } else {
                        format!("{query}▏")
                    }),
            )
            .child(div().text_color(chrome.text_faint).child(tail));

        let mut list = div().flex().flex_col();
        let mut row_index = 0usize;
        for section in &response.sections {
            if row_index >= OMNIBOX_ROWS {
                break;
            }
            list = list.child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .when(row_index > 0, |d| {
                        d.border_t_1().border_color(chrome.bar_border).mt_1()
                    })
                    .text_size(px(PALETTE_SECTION_FONT_PX * ui))
                    .text_color(chrome.text_faint)
                    .child(section.heading.clone()),
            );
            for result in &section.results {
                if row_index >= OMNIBOX_ROWS {
                    break;
                }
                let selected = row_index == index;
                let at = row_index;
                let subtitle = result.subtitle.clone().unwrap_or_default();
                list = list.child(
                    div()
                        .id(gpui::SharedString::from(format!("omni-{}", result.id)))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .when(selected, |d| d.bg(chrome.row_selected))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                // A click chooses the row it landed on, so
                                // the selection goes there first.
                                this.model.omni.index = at;
                                this.model.omni_enter();
                                this.omni_field = crate::field::Field::default();
                                this.perform_effects();
                                cx.notify();
                            }),
                        )
                        .child(div().text_color(chrome.text_faint).child(mark(result.kind)))
                        .child(
                            div()
                                .text_color(chrome.text_bright)
                                .child(result.title.clone()),
                        )
                        .child(
                            div()
                                .text_color(crate::chrome::with_alpha(
                                    chrome.text_muted,
                                    OMNIBOX_SUBTITLE_ALPHA,
                                ))
                                .child(subtitle),
                        ),
                );
                row_index += 1;
            }
        }

        // The footer says what the two keys do, and the Tab offer only
        // appears when there is something to scope to.
        let mut footer = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .border_t_1()
            .border_color(chrome.control_border)
            .text_size(px(PALETTE_SECTION_FONT_PX * ui))
            .text_color(chrome.text_faint)
            .child(key_cap_box("enter", chrome, ui))
            .child(div().child("open"));
        if let Some((_, name)) = &response.offer {
            if self.model.omni.scope.is_none() {
                footer = footer
                    .child(key_cap_box("tab", chrome, ui))
                    .child(div().child(format!("search {name}")));
            }
        }
        footer = footer
            .child(key_cap_box("esc", chrome, ui))
            .child(div().child("close"));

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .justify_center()
            .pt(px(OVERLAY_TOP_PAD_PX * ui))
            .bg(chrome.overlay_backdrop)
            // A click outside closes, as it does on the dialog.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.model.close_omnibox();
                    this.omni_field = crate::field::Field::default();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(OMNIBOX_WIDTH_PX * ui))
                    .h(px(0.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .bg(chrome.bar_bg)
                            .border_1()
                            .border_color(chrome.control_border)
                            .rounded_md()
                            .font_family("Menlo")
                            .text_size(px(OVERLAY_BODY_FONT_PX * ui))
                            .text_color(chrome.text)
                            // The sheet swallows the click that would
                            // otherwise reach the backdrop and close it.
                            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                                cx.stop_propagation()
                            })
                            .child(
                                div()
                                    .px_3()
                                    .py_2()
                                    .border_b_1()
                                    .border_color(chrome.control_border)
                                    .child(field),
                            )
                            .child(list)
                            .child(footer),
                    ),
            )
    }
}
