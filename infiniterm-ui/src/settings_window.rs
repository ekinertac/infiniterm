//! The settings window (#745): every settings.json key in a window of its
//! own, opened by Cmd+, when `ui.settings` is on.
//!
//! Wiring only. What a row says, what an edit writes, what a text field's
//! text means and what the search matches are `infiniterm_core::
//! settings_rows`, tested there; the kinds and sections are
//! `settings_schema`. This file draws them and turns clicks and keys into
//! those calls.
//!
//! settings.json stays the source (#745): the window holds the file's text,
//! writes it through `settings_rows::apply` (text patched, comments kept, a
//! value the file would refuse refused with the reason and nothing written)
//! and the app's watcher applies the file a moment later, as for a hand
//! edit. An edit to the file from outside reaches the window through
//! `AppView::settings_gen`, which the poll task in main.rs watches.
//!
//! Opened from the poll task, never from inside an `AppView` update (the
//! `own_window.rs` rule: a second window that touches the app while it is
//! borrowed aborts). One instance: Cmd+, again brings it forward, Cmd+W or
//! the red button closes it.
//!
//! First version (Ekin's #745 plan, split in two PRs with Tarik): checkbox,
//! choice list and text fields for numbers, text and colours. Fonts, the
//! two theme pickers, the folder picker, lists and search engines show
//! their value with a link to settings.json; their controls are the next
//! PR. No AppKit-only part, so it draws the same on every system. The
//! window's size and position are not remembered yet.
//!
//! Related: main.rs (the poll task that opens, raises and refreshes it),
//! runtime.rs (`Effect::OpenSettingsWindow`, `settings_gen`), field.rs (the
//! text field), own_window.rs (the other second window), chrome.rs.

use std::ops::Range;

use gpui::{
    div, prelude::*, px, size, App, Bounds, Context, Entity, EntityInputHandler, FocusHandle, Hsla,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, Render, TitlebarOptions,
    UTF16Selection, Window, WindowBounds, WindowHandle, WindowOptions,
};

use infiniterm_core::config_files::{config_read, config_write, ConfigFile};
use infiniterm_core::settings_rows::{self, Row, Write};
use infiniterm_core::settings_schema::{Kind, Section};
use serde_json::Value;

use crate::chrome::Chrome;
use crate::field::Field;
use crate::AppView;

/// The window's first size, in points: wide enough for a key, its summary
/// and a control on one line, tall enough for a section without scrolling.
const WINDOW_W_PX: f32 = 860.;
const WINDOW_H_PX: f32 = 640.;
/// The section menu's width.
const MENU_W_PX: f32 = 170.;
/// Space around the panes and between rows.
const PAD_PX: f32 = 16.;
const ROW_PAD_Y_PX: f32 = 8.;
const ROW_GAP_PX: f32 = 12.;
/// The control column: a field or a choice is this wide, so the rows line up.
const CONTROL_W_PX: f32 = 240.;
/// The checkbox's side and the "set" dot's diameter.
const CHECK_PX: f32 = 16.;
const SET_DOT_PX: f32 = 6.;
/// A colour swatch beside a colour's text.
const SWATCH_PX: f32 = 14.;
/// The key line, the summary line and the section headers.
const KEY_FONT_PX: f32 = 13.;
const SUMMARY_FONT_PX: f32 = 11.5;
/// The key beside its title, a step below the title.
const KEY_MONO_FONT_PX: f32 = 11.;
const HEADER_FONT_PX: f32 = 12.;
/// Settings files are small; a write that fails says so in the row.
const SETTINGS_EMPTY: &str = "{\n}\n";

pub struct SettingsWindow {
    app: Entity<AppView>,
    focus: FocusHandle,
    /// settings.json as last read or written.
    text: String,
    section: Section,
    search: Field,
    /// The row whose text field is open, and the field.
    editing: Option<(&'static str, Field)>,
    /// The row whose choice list is open.
    choosing: Option<&'static str>,
    /// The last refusal, shown under its row until the next edit.
    error: Option<(&'static str, String)>,
    /// Text an input method is composing (`ime.rs` does the same for the
    /// main window).
    composing: Option<String>,
}

impl SettingsWindow {
    /// Opens the window and returns its handle, which `AppView` keeps so
    /// Cmd+, raises it instead of opening a second one.
    pub fn open(app: Entity<AppView>, cx: &mut App) -> Option<WindowHandle<SettingsWindow>> {
        let bounds = Bounds::centered(None, size(px(WINDOW_W_PX), px(WINDOW_H_PX)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Settings".into()),
                appears_transparent: false,
                traffic_light_position: None,
            }),
            app_id: Some(crate::APP_ID.into()),
            ..Default::default()
        };
        cx.open_window(options, move |window, cx| {
            let focus = cx.focus_handle();
            let closing = app.clone();
            window.on_window_should_close(cx, move |_, cx| {
                closing.update(cx, |a, _| a.settings_window = None);
                true
            });
            let view = cx.new(|_| SettingsWindow {
                app,
                focus: focus.clone(),
                text: read_settings(),
                section: Section::General,
                search: Field::default(),
                editing: None,
                choosing: None,
                error: None,
                composing: None,
            });
            window.focus(&focus);
            view
        })
        .ok()
    }

    /// The file changed outside the window (an editor, `ift`, the palette):
    /// read it again. An open field is kept, since its text is the person's.
    pub fn reload(&mut self) {
        self.text = read_settings();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.app.update(cx, |a, _| a.settings_window = None);
        window.remove_window();
    }

    /// Writes one edit and keeps the new text, or keeps the reason it was
    /// refused for the row to show.
    fn write(&mut self, key: &'static str, write: Write) {
        match settings_rows::apply(&self.text, key, &write) {
            Ok(Some(next)) => match config_write(ConfigFile::Settings, &next) {
                Ok(()) => {
                    self.text = next;
                    self.error = None;
                }
                Err(e) => self.error = Some((key, format!("could not write settings.json: {e}"))),
            },
            Ok(None) => self.error = None,
            Err(e) => self.error = Some((key, e)),
        }
    }

    /// The open field's text goes in, or stays open with the reason.
    fn commit(&mut self) {
        let Some((key, field)) = self.editing.take() else {
            return;
        };
        let kind = infiniterm_core::settings_schema::kind_of(key).unwrap_or(Kind::Text);
        match settings_rows::parse_field(kind, &field.text) {
            Ok(value) => self.write(key, settings_rows::write_for(key, value)),
            Err(why) => {
                self.error = Some((key, format!("{key} should be {why}")));
                self.editing = Some((key, field));
            }
        }
    }

    /// A click anywhere settles what was open first: a field commits (a
    /// Mac field applies on blur) and a choice list closes.
    fn settle(&mut self) {
        self.commit();
        self.choosing = None;
    }

    fn click_check(&mut self, key: &'static str, on: bool, cx: &mut Context<Self>) {
        self.settle();
        self.write(key, settings_rows::write_for(key, Value::Bool(!on)));
        cx.notify();
    }

    fn click_choice(&mut self, key: &'static str, cx: &mut Context<Self>) {
        let was = self.choosing;
        self.settle();
        self.choosing = (was != Some(key)).then_some(key);
        cx.notify();
    }

    fn pick_choice(&mut self, key: &'static str, word: &'static str, cx: &mut Context<Self>) {
        self.choosing = None;
        self.write(key, settings_rows::write_for(key, Value::from(word)));
        cx.notify();
    }

    fn click_field(&mut self, key: &'static str, value: &Value, cx: &mut Context<Self>) {
        if self.editing.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        self.settle();
        if self.editing.is_none() {
            self.editing = Some((key, Field::open(&settings_rows::display(value), true)));
        }
        cx.notify();
    }

    fn click_reset(&mut self, key: &'static str, cx: &mut Context<Self>) {
        self.settle();
        if self.editing.as_ref().is_some_and(|(k, _)| *k == key) {
            self.editing = None;
        }
        self.write(key, Write::Remove);
        cx.notify();
    }

    fn click_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.settle();
        self.section = section;
        self.search = Field::default();
        cx.notify();
    }

    fn open_file(&mut self, cx: &mut Context<Self>) {
        self.settle();
        self.app.update(cx, |a, cx| {
            a.run_command("app.settings.file");
            cx.notify();
        });
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        crate::keycode::note_key_down();
        let k = &e.keystroke;
        let m = &k.modifiers;
        // A dead key is composed by macOS through `replace_text_in_range`.
        if crate::keycode::last_dead() && !m.platform && !m.control {
            return;
        }
        if m.platform && k.key == "w" {
            self.close(window, cx);
            cx.stop_propagation();
            return;
        }
        match k.key.as_str() {
            "escape" => {
                if self.choosing.take().is_none() && self.editing.take().is_none() {
                    self.search = Field::default();
                }
                self.error = None;
            }
            "enter" if self.editing.is_some() => self.commit(),
            _ => {
                let paste = (m.platform && k.key == "v")
                    .then(|| cx.read_from_clipboard().and_then(|c| c.text()))
                    .flatten();
                let field = match &mut self.editing {
                    Some((_, f)) => f,
                    None => &mut self.search,
                };
                let edit = field.key(k, paste.as_deref());
                if let Some(text) = edit.clipboard() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.to_string()));
                }
                if matches!(edit, crate::field::Edit::Ignored) {
                    return;
                }
            }
        }
        cx.notify();
        cx.stop_propagation();
    }

    fn active_field(&mut self) -> &mut Field {
        match &mut self.editing {
            Some((_, f)) => f,
            None => &mut self.search,
        }
    }
}

fn read_settings() -> String {
    config_read(ConfigFile::Settings).unwrap_or_else(|| SETTINGS_EMPTY.to_string())
}

/// The colour a `#rgb`, `#rrggbb` or `#rrggbbaa` text stands for.
fn swatch_color(text: &str) -> Option<Hsla> {
    crate::chrome::hex_alpha(text).or_else(|| crate::chrome::hex(text))
}

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (chrome, ui, mono) = {
            let a = self.app.read(cx);
            (
                a.chrome.clone(),
                a.model.ui_scale as f32,
                crate::terminals::family_of(&a.model.config.terminal.font_family),
            )
        };
        let t = &chrome.typography;
        let rows = settings_rows::rows(&self.text);
        let query = self.search.text.trim().to_string();
        let searching = !query.is_empty();

        let menu = Section::ALL.iter().fold(
            div()
                .flex()
                .flex_col()
                .w(px(MENU_W_PX * ui))
                .p(px(PAD_PX * ui))
                .gap(px(2. * ui))
                .bg(chrome.bar_bg)
                .border_r_1()
                .border_color(chrome.bar_border),
            |menu, &section| {
                let current = !searching && section == self.section;
                menu.child(
                    div()
                        .px(px(8. * ui))
                        .py(px(5. * ui))
                        .rounded_md()
                        .cursor_pointer()
                        .when(current, |d| {
                            d.bg(chrome.row_selected).text_color(chrome.text_bright)
                        })
                        .when(!current, |d| d.text_color(chrome.text_mid))
                        .child(section.title())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                this.click_section(section, cx)
                            }),
                        ),
                )
            },
        );

        let search_box = div()
            .flex()
            .items_center()
            .px(px(10. * ui))
            .py(px(6. * ui))
            .rounded_md()
            .border_1()
            .border_color(if self.editing.is_none() {
                chrome.focus_ring
            } else {
                chrome.control_border
            })
            .bg(chrome.control_bg)
            .text_color(chrome.text)
            .child(if self.search.text.is_empty() {
                Field::empty_with_placeholder("Search settings").text_color(chrome.text_faint)
            } else {
                self.search.inline_composing(
                    chrome.sel_bg,
                    chrome.sel_fg,
                    self.editing
                        .is_none()
                        .then_some(())
                        .and(self.composing.as_deref()),
                )
            });

        let shown: Vec<&Row> = rows
            .iter()
            .filter(|r| {
                if searching {
                    settings_rows::matches(r, &query)
                } else {
                    r.section == self.section
                }
            })
            .collect();
        let mut list = div().flex().flex_col();
        let mut last_section = None;
        for row in &shown {
            if searching && last_section != Some(row.section) {
                last_section = Some(row.section);
                list = list.child(
                    div()
                        .pt(px(PAD_PX * ui))
                        .pb(px(4. * ui))
                        .text_size(px(t.size(HEADER_FONT_PX) * ui))
                        .font_weight(t.bold.weight)
                        .text_color(chrome.text_muted)
                        .child(row.section.title()),
                );
            }
            list = list.child(self.render_row(row, &chrome, &mono, ui, cx));
        }
        if shown.is_empty() {
            list = list.child(
                div()
                    .pt(px(PAD_PX * ui))
                    .text_color(chrome.text_muted)
                    .child("No setting matches."),
            );
        }

        let focus = self.focus.clone();
        let this = cx.entity();
        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(chrome.overlay_bg)
            .text_color(chrome.text)
            .font_family(t.family.clone())
            .font_weight(t.regular.weight)
            .text_size(px(t.size(KEY_FONT_PX) * ui))
            .track_focus(&self.focus)
            .on_key_down(
                cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)),
            )
            .child(menu)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(ROW_GAP_PX * ui))
                            .p(px(PAD_PX * ui))
                            .child(div().flex_1().child(search_box))
                            .child(
                                div()
                                    .cursor_pointer()
                                    .text_color(chrome.focus_ring)
                                    .child("Open settings.json")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                            this.open_file(cx)
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id("settings-rows")
                            .flex_1()
                            .overflow_y_scroll()
                            .px(px(PAD_PX * ui))
                            .pb(px(PAD_PX * ui))
                            .child(list),
                    ),
            )
            .child(
                // Registered every paint, as own_window.rs does: the input
                // system asks again each frame.
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus,
                            gpui::ElementInputHandler::new(bounds, this.clone()),
                            cx,
                        );
                    },
                )
                .absolute()
                .size_0(),
            )
    }
}

impl SettingsWindow {
    fn render_row(
        &self,
        row: &Row,
        chrome: &Chrome,
        mono: &str,
        ui: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = &chrome.typography;
        let key = row.key;
        let label = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6. * ui))
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_none()
                            .size(px(SET_DOT_PX * ui))
                            .rounded_full()
                            .when(row.set, |d| d.bg(chrome.focus_ring)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(chrome.text_bright)
                            .child(settings_rows::title(key)),
                    )
                    // The key as it is spelled in settings.json, in the
                    // terminal's monospace face and a size down, so the
                    // title reads first and the key is there to copy.
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(mono.to_string())
                            .text_size(px(t.size(KEY_MONO_FONT_PX) * ui))
                            .text_color(chrome.text_muted)
                            .child(key),
                    ),
            )
            .child(
                div()
                    .pl(px((SET_DOT_PX + 6.) * ui))
                    .text_size(px(t.size(SUMMARY_FONT_PX) * ui))
                    .text_color(chrome.text_muted)
                    .truncate()
                    .child(settings_rows::summary(key)),
            );

        let control = self.render_control(row, chrome, ui, cx);
        let reset = div()
            .w(px(44. * ui))
            .flex()
            .justify_end()
            .when(row.set, |d| {
                d.child(
                    div()
                        .cursor_pointer()
                        .text_size(px(t.size(SUMMARY_FONT_PX) * ui))
                        .text_color(chrome.text_muted)
                        .child("Reset")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                this.click_reset(key, cx)
                            }),
                        ),
                )
            });

        let mut out = div()
            .flex()
            .flex_col()
            .py(px(ROW_PAD_Y_PX * ui))
            .border_b_1()
            .border_color(chrome.bar_border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(ROW_GAP_PX * ui))
                    .child(label)
                    .child(control)
                    .child(reset),
            );
        if let (Some(Kind::Choice(words)), true) = (Some(row.kind), self.choosing == Some(key)) {
            out = out.child(self.render_choices(row, words, chrome, ui, cx));
        }
        if let Some((_, why)) = self.error.as_ref().filter(|(k, _)| *k == key) {
            out = out.child(
                div()
                    .pt(px(4. * ui))
                    .text_size(px(t.size(SUMMARY_FONT_PX) * ui))
                    .text_color(chrome.warn)
                    .child(why.clone()),
            );
        }
        out.into_any_element()
    }

    fn render_control(
        &self,
        row: &Row,
        chrome: &Chrome,
        ui: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let key = row.key;
        let boxed = |d: gpui::Div| {
            d.w(px(CONTROL_W_PX * ui))
                .px(px(8. * ui))
                .py(px(4. * ui))
                .rounded_md()
                .border_1()
                .bg(chrome.control_bg)
        };
        match row.kind {
            Kind::Bool => {
                let on = row.value.as_bool().unwrap_or(false);
                div()
                    .w(px(CONTROL_W_PX * ui))
                    .child(
                        div()
                            .size(px(CHECK_PX * ui))
                            .rounded_sm()
                            .border_1()
                            .border_color(if on {
                                chrome.sel_bg
                            } else {
                                chrome.control_border
                            })
                            .bg(if on { chrome.sel_bg } else { chrome.control_bg })
                            .text_color(chrome.sel_fg)
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .when(on, |d| d.child("\u{2713}"))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                    this.click_check(key, on, cx)
                                }),
                            ),
                    )
                    .into_any_element()
            }
            Kind::Choice(_) => boxed(div())
                .border_color(chrome.control_border)
                .flex()
                .flex_row()
                .justify_between()
                .cursor_pointer()
                .child(settings_rows::display(&row.value))
                .child(div().text_color(chrome.text_muted).child("\u{25be}"))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| this.click_choice(key, cx)),
                )
                .into_any_element(),
            Kind::Number | Kind::OptionalNumber | Kind::Text | Kind::Color => {
                let editing = self.editing.as_ref().filter(|(k, _)| *k == key);
                let text = match editing {
                    Some((_, f)) => f.text.clone(),
                    None => settings_rows::display(&row.value),
                };
                let swatch = (row.kind == Kind::Color)
                    .then(|| swatch_color(text.trim()))
                    .flatten();
                let shown = match editing {
                    Some((_, f)) => {
                        f.inline_composing(chrome.sel_bg, chrome.sel_fg, self.composing.as_deref())
                    }
                    None if text.is_empty() => div().text_color(chrome.text_faint).child(
                        if row.kind == Kind::OptionalNumber {
                            "unset"
                        } else {
                            "empty"
                        },
                    ),
                    None => div().truncate().child(text),
                };
                let value = row.value.clone();
                boxed(div())
                    .border_color(if editing.is_some() {
                        chrome.focus_ring
                    } else {
                        chrome.control_border
                    })
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6. * ui))
                    .cursor_text()
                    .children(swatch.map(|c| {
                        div()
                            .size(px(SWATCH_PX * ui))
                            .rounded_sm()
                            .border_1()
                            .border_color(chrome.control_border)
                            .bg(c)
                    }))
                    .child(div().flex_1().min_w_0().child(shown))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.click_field(key, &value, cx)
                        }),
                    )
                    .into_any_element()
            }
            // Their controls are the next PR: the value, read only, and the
            // file one click away.
            Kind::Font
            | Kind::TerminalTheme
            | Kind::UiTheme
            | Kind::Folder
            | Kind::TextList
            | Kind::NumberList
            | Kind::SearchEngines => {
                let text = settings_rows::display(&row.value);
                div()
                    .w(px(CONTROL_W_PX * ui))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_color(chrome.text_mid)
                            .child(if text.is_empty() {
                                "empty".to_string()
                            } else {
                                text
                            }),
                    )
                    .child(
                        div()
                            .cursor_pointer()
                            .text_size(px(chrome.typography.size(SUMMARY_FONT_PX) * ui))
                            .text_color(chrome.focus_ring)
                            .child("Edit in settings.json")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, _, cx| this.open_file(cx)),
                            ),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_choices(
        &self,
        row: &Row,
        words: &'static [&'static str],
        chrome: &Chrome,
        ui: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let key = row.key;
        let current = settings_rows::display(&row.value);
        words
            .iter()
            .fold(
                div()
                    .mt(px(6. * ui))
                    .ml_auto()
                    .mr(px((44. + ROW_GAP_PX) * ui))
                    .w(px(CONTROL_W_PX * ui))
                    .p(px(4. * ui))
                    .rounded_md()
                    .border_1()
                    .border_color(chrome.overlay_border)
                    .bg(chrome.bar_bg)
                    .flex()
                    .flex_col(),
                |list, &word| {
                    let here = word == current;
                    list.child(
                        div()
                            .px(px(8. * ui))
                            .py(px(4. * ui))
                            .rounded_sm()
                            .cursor_pointer()
                            .when(here, |d| {
                                d.bg(chrome.row_selected).text_color(chrome.text_bright)
                            })
                            .child(word)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                    this.pick_choice(key, word, cx)
                                }),
                            ),
                    )
                },
            )
            .into_any_element()
    }
}

impl EntityInputHandler for SettingsWindow {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        self.composing.clone()
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self
            .composing
            .as_deref()
            .map(|m| m.encode_utf16().count())
            .unwrap_or(0);
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.composing
            .as_deref()
            .map(|m| 0..m.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.composing = None;
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composing = None;
        if text.is_empty() {
            return;
        }
        self.active_field().insert_text(text);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composing = (!new_text.is_empty()).then(|| new_text.to_string());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(Bounds::new(element_bounds.origin, size(px(0.), px(0.))))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
