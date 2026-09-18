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
    /// Whether the key was taken. A key the field ignored is left for
    /// macOS's input context, so a dead key can begin a composition.
    pub fn omni_key(&mut self, k: &Keystroke, cx: &mut gpui::App) -> bool {
        let mut taken = true;
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
                let edit = self.omni_field.key(k, paste.as_deref());
                if let Some(text) = edit.clipboard() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.to_string()));
                }
                if edit.changed() {
                    let text = self.omni_field.text.clone();
                    self.model.omni_type(&text);
                }
                taken = !matches!(edit, crate::field::Edit::Ignored);
            }
        }
        self.perform_effects();
        taken
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
        let placeholder = query.is_empty() && self.model.omni.scope.is_none();
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
        // The caret goes AFTER the completion, and the completion is drawn
        // as a selection, which is what a browser does: the tail is text you
        // are about to accept, not text sitting past the cursor. Putting the
        // caret between the two read as "exam| ple.com".
        field = field.child(
            div()
                .flex()
                .items_center()
                .gap_0()
                .text_color(if query.is_empty() {
                    chrome.text_faint
                } else {
                    chrome.text_bright
                })
                // No caret against the placeholder: the box is empty, and a
                // caret there suggests those words are text you typed.
                .when(placeholder, |d| {
                    d.child("search or enter address".to_string())
                })
                .when(!placeholder, |d| {
                    d.child(self.omni_field.inline(chrome.sel_bg, chrome.sel_fg))
                })
                .when(!tail.is_empty(), |d| {
                    d.child(
                        div()
                            .bg(chrome.sel_bg)
                            .text_color(chrome.sel_fg)
                            .child(tail.clone()),
                    )
                }),
        );

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
                        // The row is exactly the dialog's width; without this
                        // a long url painted past its own bounds and over the
                        // border, since gpui does not clip by default.
                        .overflow_hidden()
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
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(chrome.text_faint)
                                .child(mark(result.kind)),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(chrome.text_bright)
                                .child(result.title.clone()),
                        )
                        .child(
                            // The title keeps its width; the url takes what
                            // is left and elides rather than pushing the row
                            // past the dialog's edge.
                            div()
                                .flex_1()
                                .truncate()
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
                            .bg(chrome.overlay_bg)
                            .border_1()
                            .border_color(chrome.overlay_border)
                            .rounded_md()
                            .shadow_lg()
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

/// The find bar's width. Narrower than the omnibox: it holds a short query
/// and a count, not a list.
pub const FIND_BAR_WIDTH_PX: f32 = 320.;
/// Clear of the title bar, on the right, where every browser puts it.
pub const FIND_BAR_TOP_PX: f32 = 12.;
pub const FIND_BAR_RIGHT_PX: f32 = 16.;

impl AppView {
    /// The find bar's keys. Enter steps forward, Shift+Enter back, Escape
    /// closes and clears the highlights. Everything else is the field.
    pub fn find_key(&mut self, k: &Keystroke, cx: &mut gpui::App) -> bool {
        let mut taken = true;
        match k.key.as_str() {
            "escape" => {
                self.model.close_find();
                self.find_field = crate::field::Field::default();
            }
            "enter" => self.model.find_step(!k.modifiers.shift),
            _ => {
                let paste = (k.modifiers.platform && k.key == "v")
                    .then(|| cx.read_from_clipboard().and_then(|c| c.text()))
                    .flatten();
                let edit = self.find_field.key(k, paste.as_deref());
                if let Some(text) = edit.clipboard() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.to_string()));
                }
                if edit.changed() {
                    let text = self.find_field.text.clone();
                    self.model.find_type(&text);
                }
                taken = !matches!(edit, crate::field::Edit::Ignored);
            }
        }
        self.perform_effects();
        taken
    }

    /// No backdrop and no dimming: the whole point is reading the page while
    /// this is open, which is also why it does not join `overlay_open`.
    pub fn render_find_bar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let ui = self.model.ui_scale as f32;
        let chrome = &self.chrome;
        if !self.model.find.open {
            return div();
        }
        let query = self.model.find.query.clone();
        let (matches, active) = (self.model.find.matches, self.model.find.active);
        // Nothing to say before a query; "0/0" against an empty box reads
        // as a failure rather than as a prompt.
        let count = if query.is_empty() {
            String::new()
        } else if matches == 0 {
            "no matches".to_string()
        } else if active == 0 {
            // Chromium reports the active match as 0 until it has selected
            // one, which is every new search; "0/2" reads like a failure.
            format!("{matches} match{}", if matches == 1 { "" } else { "es" })
        } else {
            format!("{active}/{matches}")
        };
        div()
            .absolute()
            .top(px(FIND_BAR_TOP_PX * ui))
            .right(px(FIND_BAR_RIGHT_PX * ui))
            .w(px(FIND_BAR_WIDTH_PX * ui))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .bg(chrome.overlay_bg)
            .border_1()
            .border_color(chrome.overlay_border)
            .rounded_md()
            .shadow_lg()
            .font_family("Menlo")
            .text_size(px(OVERLAY_BODY_FONT_PX * ui))
            .child(
                div()
                    .flex_1()
                    .text_color(if query.is_empty() {
                        chrome.text_faint
                    } else {
                        chrome.text_bright
                    })
                    .when(query.is_empty(), |d| d.child("find in page".to_string()))
                    .when(!query.is_empty(), |d| {
                        d.child(self.find_field.inline(chrome.sel_bg, chrome.sel_fg))
                    }),
            )
            .child(
                div()
                    .text_size(px(PALETTE_SECTION_FONT_PX * ui))
                    .text_color(if matches == 0 && !query.is_empty() {
                        chrome.warn
                    } else {
                        chrome.text_muted
                    })
                    .child(count),
            )
            .child(
                div()
                    .id("find-close")
                    .text_color(chrome.text_faint)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            this.model.close_find();
                            this.find_field = crate::field::Field::default();
                            this.perform_effects();
                            cx.notify();
                        }),
                    )
                    .child("✕"),
            )
    }
}
