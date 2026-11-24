//! The settings window (#745): every settings.json key in a window of its
//! own, opened by Cmd+, when `ui.settings` is on (the default) and by
//! "App: open settings window" either way.
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
//! Controls, by kind: checkbox, choice list, text fields for numbers, text
//! and colours (first PR); a searchable list for installed fonts, terminal
//! themes and ui themes, the system's folder panel for `startingDir`
//! (`cx.prompt_for_paths`), and one row per item with add and remove for
//! `terminal.env`, `editor.rulers` and `browser.engines` (second PR). A list
//! row edits what the file holds (`Row::list`), so a search engine still
//! being typed stays on screen although the merge does not use it yet; the
//! row says why. No AppKit-only part, so it draws the same on every system.
//! The window's frame is kept in `settings-window.json` (`window_state.rs`).
//!
//! UI Theme page (third PR): the ui theme file `ui.theme` names
//! (`settings_schema::THEME_SCHEMA`, `settings_rows::theme_rows`), written
//! through `theme_apply` so `ui_theme::parse` refuses what would not load.
//! The built-in theme (`default`) is not edited: "Make a copy" writes it
//! under a new name and points `ui.theme` at the copy. Theme rows are in
//! the search too, under "UI Theme".
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

use infiniterm_core::config_files::{self, config_read, config_write, ConfigFile};
use infiniterm_core::settings_rows::{self, Row, Write};
use infiniterm_core::settings_schema::{self, Kind, Section};
use infiniterm_core::ui_theme;
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
const CONTROL_W_PX: f32 = 200.;
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
/// A picker's list: its width (font names are long), its height before it
/// scrolls, and how many matches it draws at most (a Mac has hundreds of
/// fonts; past this the filter is the way in).
const PICKER_W_PX: f32 = 320.;
const PICKER_H_PX: f32 = 260.;
const PICKER_MAX_ROWS: usize = 300;
/// The "Make a copy" name field's target key: not a setting, so it can
/// never collide with a row.
const COPY_KEY: &str = "\u{0}copy";
/// Settings files are small; a write that fails says so in the row.
const SETTINGS_EMPTY: &str = "{\n}\n";

/// What an open text field edits: a key's value, one item of a list key,
/// or one part (`keyword`, `name`, `searchUrl`) of a search engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Target {
    key: &'static str,
    item: Option<usize>,
    part: Option<&'static str>,
}

impl Target {
    fn key(key: &'static str) -> Target {
        Target {
            key,
            item: None,
            part: None,
        }
    }
}

/// A searchable list under a row: installed fonts, terminal themes, ui
/// themes. Typing filters it (`settings_rows::pick_matches`).
struct Picker {
    key: &'static str,
    filter: Field,
    items: Vec<String>,
}

pub struct SettingsWindow {
    app: Entity<AppView>,
    focus: FocusHandle,
    /// settings.json as last read or written.
    text: String,
    section: Section,
    search: Field,
    /// The text field that is open, and what it edits.
    editing: Option<(Target, Field)>,
    /// The row whose choice list is open.
    choosing: Option<&'static str>,
    /// The row whose searchable list is open.
    picking: Option<Picker>,
    /// The frame last saved, so a resize writes it once.
    saved_frame: Option<crate::window_state::WindowState>,
    /// The last refusal, shown under its row until the next edit.
    error: Option<(&'static str, String)>,
    /// Text an input method is composing (`ime.rs` does the same for the
    /// main window).
    composing: Option<String>,
    /// The UI Theme page is showing instead of a section.
    theme_page: bool,
    /// The ui theme `ui.theme` names, and its file's text (empty when the
    /// file is missing: the built-in look).
    theme_name: String,
    theme_text: String,
}

impl SettingsWindow {
    /// Opens the window and returns its handle, which `AppView` keeps so
    /// Cmd+, raises it instead of opening a second one.
    pub fn open(app: Entity<AppView>, cx: &mut App) -> Option<WindowHandle<SettingsWindow>> {
        // Where it was last time, else centred.
        let saved = crate::window_state::WindowState::load_settings();
        let bounds = saved.as_ref().map(|s| s.bounds()).unwrap_or_else(|| {
            WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(WINDOW_W_PX), px(WINDOW_H_PX)),
                cx,
            ))
        });
        let options = WindowOptions {
            window_bounds: Some(bounds),
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
            window.on_window_should_close(cx, move |window, cx| {
                save_frame(window);
                closing.update(cx, |a, _| a.settings_window = None);
                true
            });
            let view = cx.new(|_| {
                let mut w = SettingsWindow {
                    app,
                    focus: focus.clone(),
                    text: String::new(),
                    section: Section::General,
                    search: Field::default(),
                    editing: None,
                    choosing: None,
                    picking: None,
                    saved_frame: saved,
                    error: None,
                    composing: None,
                    theme_page: false,
                    theme_name: String::new(),
                    theme_text: String::new(),
                };
                w.reload();
                w
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
        self.theme_name = settings_rows::rows(&self.text)
            .into_iter()
            .find(|r| r.key == "ui.theme")
            .and_then(|r| r.value.as_str().map(String::from))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| ui_theme::DEFAULT_NAME.to_string());
        self.theme_text = config_files::ui_theme_read(&self.theme_name).unwrap_or_default();
    }

    /// Every row: the settings, then the ui theme's.
    fn all_rows(&self) -> Vec<Row> {
        let mut rows = settings_rows::rows(&self.text);
        rows.extend(settings_rows::theme_rows(&self.theme_text));
        rows
    }

    /// True while the ui theme in force is the built-in one, which the page
    /// does not edit: "Make a copy" makes one of your own (#745).
    fn theme_is_builtin(&self) -> bool {
        self.theme_name == ui_theme::DEFAULT_NAME
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        save_frame(window);
        self.app.update(cx, |a, _| a.settings_window = None);
        window.remove_window();
    }

    /// The list a list row edits, from the text as it is now.
    fn list_of(&self, key: &str) -> Vec<Value> {
        self.row_of(key).map(|r| r.list()).unwrap_or_default()
    }

    fn row_of(&self, key: &str) -> Option<Row> {
        self.all_rows().into_iter().find(|r| r.key == key)
    }

    /// What a key's value looks like in the file, for the edits that change
    /// one part of it (an icon map entry, a gradient stop).
    fn raw_of(&self, key: &str) -> Value {
        self.row_of(key)
            .map(|r| r.raw.unwrap_or(r.value))
            .unwrap_or(Value::Null)
    }

    /// Sets `key` to `value`: removed instead when it is the default.
    fn set(&mut self, key: &'static str, value: Value) {
        let write = if settings_schema::theme_kind_of(key).is_some() {
            settings_rows::theme_write_for(key, value)
        } else {
            settings_rows::write_for(key, value)
        };
        self.write(key, write);
    }

    /// Writes one edit and keeps the new text, or keeps the reason it was
    /// refused for the row to show. A ui theme key goes to the theme's file.
    fn write(&mut self, key: &'static str, write: Write) {
        if settings_schema::theme_kind_of(key).is_some() {
            self.write_theme(key, write);
            return;
        }
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

    fn write_theme(&mut self, key: &'static str, write: Write) {
        if self.theme_is_builtin() {
            self.error = Some((
                key,
                "this is the built-in look: make a copy at the top of UI Theme to change it".into(),
            ));
            return;
        }
        match settings_rows::theme_apply(&self.theme_text, key, &write) {
            Ok(Some(next)) => match config_files::ui_theme_write(&self.theme_name, &next) {
                Ok(()) => {
                    self.theme_text = next;
                    self.error = None;
                }
                Err(e) => self.error = Some((key, format!("could not write the ui theme: {e}"))),
            },
            Ok(None) => self.error = None,
            Err(e) => self.error = Some((key, e)),
        }
    }

    /// "Make a copy": the built-in look (its file if there is one, which
    /// may hold the look settings moved there in #511, else the reference
    /// text) under `name`, and `ui.theme` pointed at it.
    fn make_copy(&mut self, name: &str) {
        let name = name.trim();
        let taken = config_files::list_ui_themes();
        let why = if !ui_theme::valid_name(name) || name == ui_theme::DEFAULT_NAME {
            Some("a plain file name other than default")
        } else if taken.iter().any(|t| t == name) {
            Some("a ui theme with that name exists; pick another")
        } else {
            None
        };
        if let Some(why) = why {
            self.error = Some((COPY_KEY, format!("the copy's name should be {why}")));
            return;
        }
        let text = if self.theme_text.trim().is_empty() {
            infiniterm_core::settings_doc::default_ui_theme_text()
        } else {
            self.theme_text.clone()
        };
        if let Err(e) = config_files::ui_theme_write(name, &text) {
            self.error = Some((COPY_KEY, format!("could not write the copy: {e}")));
            return;
        }
        self.write("ui.theme", Write::Set(Value::from(name)));
        self.theme_name = name.to_string();
        self.theme_text = text;
    }

    /// The open field's text goes in, or stays open with the reason.
    fn commit(&mut self) {
        let Some((target, field)) = self.editing.take() else {
            return;
        };
        let key = target.key;
        if key == COPY_KEY {
            self.make_copy(&field.text);
            return;
        }
        let kind = settings_schema::kind_of(key)
            .or_else(|| settings_schema::theme_kind_of(key))
            .unwrap_or(Kind::Text);
        let Some(i) = target.item else {
            match settings_rows::parse_field(kind, &field.text) {
                Ok(value) => self.set(key, value),
                Err(why) => {
                    self.error = Some((key, format!("{key} should be {why}")));
                    self.editing = Some((target, field));
                }
            }
            return;
        };
        // One item of a list. An emptied item goes, and an "Add" left
        // empty adds nothing.
        let list = self.list_of(key);
        let text = field.text.trim();
        let next = match (kind, target.part) {
            // A gradient is two stops edited one at a time; both emptied is
            // "off" (`[]`). The theme takes two stops or none, so a stop
            // typed while the other is empty fills both (a flat colour) and
            // the other cell then changes its end; one alone was refused
            // and the typed stop lost (seen on screen).
            (Kind::ColorPair, _) => {
                let mut pair: Vec<Value> = list.clone();
                pair.resize(2, Value::from(""));
                pair[i.min(1)] = Value::from(text);
                let empty = |v: &Value| v.as_str().unwrap_or("").is_empty();
                match (empty(&pair[0]), empty(&pair[1])) {
                    (true, true) => Value::Array(vec![]),
                    (false, true) => Value::Array(vec![pair[0].clone(), pair[0].clone()]),
                    (true, false) => Value::Array(vec![pair[1].clone(), pair[1].clone()]),
                    (false, false) => Value::Array(pair),
                }
            }
            (Kind::IconMap, _) if text.is_empty() => {
                if i >= settings_rows::map_entries(&self.raw_of(key)).len() {
                    return;
                }
                settings_rows::map_remove(&self.raw_of(key), i)
            }
            (Kind::IconMap, _) => match settings_rows::map_set(&self.raw_of(key), i, text) {
                Ok(map) => map,
                Err(why) => {
                    self.error = Some((key, format!("an entry should be {why}")));
                    self.editing = Some((target, field));
                    return;
                }
            },
            (Kind::SearchEngines, Some(part)) => {
                if i >= list.len() && text.is_empty() {
                    return;
                }
                settings_rows::engine_set(&list, i, part, text)
            }
            _ if text.is_empty() => {
                if i >= list.len() {
                    return;
                }
                settings_rows::list_remove(&list, i)
            }
            (Kind::NumberList, _) => match settings_rows::parse_field(Kind::Number, text) {
                Ok(n) => settings_rows::list_set(&list, i, n),
                Err(why) => {
                    self.error = Some((key, format!("{key} items should be {why}")));
                    self.editing = Some((target, field));
                    return;
                }
            },
            _ => settings_rows::list_set(&list, i, Value::from(field.text.clone())),
        };
        self.set(key, next);
    }

    fn open_theme_page(&mut self, cx: &mut Context<Self>) {
        self.settle();
        self.theme_page = true;
        self.search = Field::default();
        cx.notify();
    }

    fn toggle_name(&mut self, key: &'static str, name: &str, cx: &mut Context<Self>) {
        self.settle();
        let next = settings_rows::name_toggle(&self.list_of(key), name);
        self.set(key, next);
        cx.notify();
    }

    /// A picture for `ui.backgroundImage` from the system's file panel,
    /// added at the end of the list.
    fn add_picture(&mut self, key: &'static str, cx: &mut Context<Self>) {
        self.settle();
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Add".into()),
        });
        cx.spawn(async move |view, cx| {
            let path = picked
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
                .and_then(|paths| paths.into_iter().next());
            let Some(path) = path else { return };
            let _ = view.update(cx, |this, cx| {
                let list = this.list_of(key);
                let path = Value::from(path.to_string_lossy().into_owned());
                let next = settings_rows::list_set(&list, list.len(), path);
                this.set(key, next);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// A click anywhere settles what was open first: a field commits (a
    /// Mac field applies on blur) and a list closes.
    fn settle(&mut self) {
        self.commit();
        self.choosing = None;
        self.picking = None;
    }

    fn open_picker(&mut self, key: &'static str, kind: Kind, cx: &mut Context<Self>) {
        let was = self.picking.as_ref().map(|p| p.key);
        self.settle();
        if was == Some(key) {
            cx.notify();
            return;
        }
        let mut items: Vec<String> = match kind {
            Kind::Font => crate::fonts::installed_families().to_vec(),
            Kind::TerminalTheme => {
                let dir = self.app.read(cx).themes_dir.clone();
                infiniterm_core::themes_files::list_themes(&dir)
            }
            _ => {
                let mut names = infiniterm_core::config_files::list_ui_themes();
                let default = infiniterm_core::ui_theme::DEFAULT_NAME.to_string();
                if !names.contains(&default) {
                    names.insert(0, default);
                }
                names
            }
        };
        if kind == Kind::Font {
            items.sort_by_key(|f| f.to_lowercase());
            items.dedup();
        }
        self.picking = Some(Picker {
            key,
            filter: Field::default(),
            items,
        });
        cx.notify();
    }

    fn pick(&mut self, key: &'static str, item: String, cx: &mut Context<Self>) {
        self.picking = None;
        self.set(key, Value::from(item));
        cx.notify();
    }

    /// The system's folder panel; the answer arrives on its own task.
    fn choose_folder(&mut self, key: &'static str, cx: &mut Context<Self>) {
        self.settle();
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |view, cx| {
            let path = picked
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
                .and_then(|paths| paths.into_iter().next());
            let Some(path) = path else { return };
            let _ = view.update(cx, |this, cx| {
                let path = path.to_string_lossy().into_owned();
                this.set(key, Value::from(path));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn click_item(&mut self, target: Target, text: String, cx: &mut Context<Self>) {
        if self.editing.as_ref().is_some_and(|(t, _)| *t == target) {
            return;
        }
        self.settle();
        if self.editing.is_none() {
            self.editing = Some((target, Field::open(&text, true)));
        }
        cx.notify();
    }

    fn remove_item(&mut self, key: &'static str, i: usize, cx: &mut Context<Self>) {
        self.settle();
        let next = if settings_schema::theme_kind_of(key) == Some(Kind::IconMap) {
            settings_rows::map_remove(&self.raw_of(key), i)
        } else {
            settings_rows::list_remove(&self.list_of(key), i)
        };
        self.set(key, next);
        cx.notify();
    }

    fn click_check(&mut self, key: &'static str, on: bool, cx: &mut Context<Self>) {
        self.settle();
        self.set(key, Value::Bool(!on));
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
        self.set(key, Value::from(word));
        cx.notify();
    }

    fn click_field(&mut self, key: &'static str, value: &Value, cx: &mut Context<Self>) {
        self.click_item(Target::key(key), settings_rows::display(value), cx);
    }

    fn click_reset(&mut self, key: &'static str, cx: &mut Context<Self>) {
        self.settle();
        if self.editing.as_ref().is_some_and(|(t, _)| t.key == key) {
            self.editing = None;
        }
        self.write(key, Write::Remove);
        cx.notify();
    }

    fn click_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.settle();
        self.section = section;
        self.theme_page = false;
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
                if self.choosing.take().is_none()
                    && self.editing.take().is_none()
                    && self.picking.take().is_none()
                {
                    self.search = Field::default();
                }
                self.error = None;
            }
            "enter" if self.editing.is_some() => self.commit(),
            // Enter in a picker takes the best match.
            "enter" if self.picking.is_some() => {
                let best = self.picking.as_ref().and_then(|p| {
                    settings_rows::pick_matches(&p.items, &p.filter.text)
                        .first()
                        .map(|s| (p.key, s.to_string()))
                });
                match best {
                    Some((key, item)) => self.pick(key, item, cx),
                    None => self.picking = None,
                }
            }
            _ => {
                let paste = (m.platform && k.key == "v")
                    .then(|| cx.read_from_clipboard().and_then(|c| c.text()))
                    .flatten();
                let field = self.active_field();
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

    /// Where typing goes: the open field, else an open picker's filter,
    /// else the search.
    fn active_field(&mut self) -> &mut Field {
        if let Some((_, f)) = &mut self.editing {
            return f;
        }
        if let Some(p) = &mut self.picking {
            return &mut p.filter;
        }
        &mut self.search
    }

    /// The frame, written once per change: a resize repaints, so this runs
    /// from `render` and costs a compare when nothing moved.
    fn note_frame(&mut self, window: &Window) {
        let now = frame_of(window);
        if self.saved_frame.as_ref() != Some(&now) {
            now.save_settings();
            self.saved_frame = Some(now);
        }
    }
}

/// The window's frame into `settings-window.json`, beside the main window's.
fn save_frame(window: &Window) {
    frame_of(window).save_settings();
}

/// Where the window is, in the terms `open_window` takes back: the frame's
/// origin and the CONTENT size. This window keeps a standard title bar, and
/// `window_bounds` reports the outer frame, so saving that grew the window
/// by the bar's height on every reopen (seen on screen: 672 became 704).
fn frame_of(window: &Window) -> crate::window_state::WindowState {
    let mut s = crate::window_state::WindowState::of(window.window_bounds());
    let content = window.viewport_size();
    s.w = f32::from(content.width);
    s.h = f32::from(content.height);
    s
}

fn read_settings() -> String {
    config_read(ConfigFile::Settings).unwrap_or_else(|| SETTINGS_EMPTY.to_string())
}

/// The colour a `#rgb`, `#rrggbb` or `#rrggbbaa` text stands for.
fn swatch_color(text: &str) -> Option<Hsla> {
    crate::chrome::hex_alpha(text).or_else(|| crate::chrome::hex(text))
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.note_frame(window);
        let (chrome, ui, mono) = {
            let a = self.app.read(cx);
            (
                a.chrome.clone(),
                a.model.ui_scale as f32,
                crate::terminals::family_of(&a.model.config.terminal.font_family),
            )
        };
        let t = &chrome.typography;
        let rows = self.all_rows();
        let query = self.search.text.trim().to_string();
        let searching = !query.is_empty();
        // The search field has the keys when no field or picker does.
        let searching_keys = self.editing.is_none() && self.picking.is_none();
        let theme_page = self.theme_page && !searching;

        let menu_item = |title: &'static str, current: bool| {
            div()
                .px(px(8. * ui))
                .py(px(5. * ui))
                .rounded_md()
                .cursor_pointer()
                .when(current, |d| {
                    d.bg(chrome.row_selected).text_color(chrome.text_bright)
                })
                .when(!current, |d| d.text_color(chrome.text_mid))
                .child(title)
        };
        let theme_item = menu_item("UI Theme", theme_page).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _, cx| this.open_theme_page(cx)),
        );
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
                let current = !searching && !theme_page && section == self.section;
                menu.child(menu_item(section.title(), current).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                        this.click_section(section, cx)
                    }),
                ))
            },
        );
        // The ui theme's own page, apart from the settings sections (#745).
        let menu = menu
            .child(div().my(px(6. * ui)).h(px(1.)).bg(chrome.bar_border))
            .child(theme_item);

        let search_box = div()
            .flex()
            .items_center()
            .px(px(10. * ui))
            .py(px(6. * ui))
            .rounded_md()
            .border_1()
            .border_color(if searching_keys {
                chrome.focus_ring
            } else {
                chrome.control_border
            })
            .bg(chrome.control_bg)
            .text_color(chrome.text)
            .child(if self.search.text.is_empty() && searching_keys {
                Field::empty_with_placeholder("Search settings").text_color(chrome.text_faint)
            } else if self.search.text.is_empty() {
                div().text_color(chrome.text_faint).child("Search settings")
            } else if !searching_keys {
                // Another field has the keys: no caret here.
                div().child(self.search.text.clone())
            } else {
                self.search.inline_composing(
                    chrome.sel_bg,
                    chrome.sel_fg,
                    searching_keys.then_some(()).and(self.composing.as_deref()),
                )
            });

        let shown: Vec<&Row> = rows
            .iter()
            .filter(|r| {
                if searching {
                    settings_rows::matches(r, &query)
                } else if theme_page {
                    r.theme.is_some()
                } else {
                    r.theme.is_none() && r.section == self.section
                }
            })
            .collect();
        let mut list = div().flex().flex_col();
        if theme_page {
            list = list.child(self.render_theme_header(&chrome, &mono, ui, cx));
        }
        // A heading wherever the group changes: the section while searching,
        // the theme's groups on its page and in a search.
        let mut last_heading: Option<String> = None;
        for row in &shown {
            let heading = match row.theme {
                Some(group) if searching => Some(format!("UI Theme \u{b7} {}", group.title())),
                Some(group) => Some(group.title().to_string()),
                None if searching => Some(row.section.title().to_string()),
                None => None,
            };
            if heading.is_some() && heading != last_heading {
                last_heading = heading.clone();
                list = list.child(
                    div()
                        .pt(px(PAD_PX * ui))
                        .pb(px(4. * ui))
                        .text_size(px(t.size(HEADER_FONT_PX) * ui))
                        .font_weight(t.bold.weight)
                        .text_color(chrome.text_muted)
                        .children(heading),
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
    /// The top of the UI Theme page: which file it edits, and while the
    /// built-in look is in force, a name and "Make a copy" (#745).
    fn render_theme_header(
        &self,
        chrome: &Chrome,
        mono: &str,
        ui: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = &chrome.typography;
        let file = format!("ui-themes/{}.json", self.theme_name);
        let mut out = div()
            .flex()
            .flex_col()
            .gap(px(6. * ui))
            .pb(px(ROW_PAD_Y_PX * ui))
            .border_b_1()
            .border_color(chrome.bar_border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(6. * ui))
                    .text_color(chrome.text_mid)
                    .child("Editing")
                    .child(
                        div()
                            .font_family(mono.to_string())
                            .text_size(px(t.size(KEY_MONO_FONT_PX) * ui))
                            .text_color(chrome.text_bright)
                            .child(file),
                    )
                    .child("(set by ui.theme). Unset keys use the built-in look."),
            );
        if self.theme_is_builtin() {
            let target = Target::key(COPY_KEY);
            let suggested = settings_rows::copy_name(&config_files::list_ui_themes());
            out = out
                .child(
                    div()
                        .text_color(chrome.text_muted)
                        .child("This is the built-in look. Make a copy to change it: the copy becomes your ui.theme."),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(ROW_GAP_PX * ui))
                        .child(
                            div()
                                .w(px(CONTROL_W_PX * ui))
                                .flex()
                                .child(self.cell(target, suggested.clone(), "name", chrome, ui, cx)),
                        )
                        .child(
                            div()
                                .px(px(10. * ui))
                                .py(px(4. * ui))
                                .rounded_md()
                                .bg(chrome.sel_bg)
                                .text_color(chrome.sel_fg)
                                .cursor_pointer()
                                .child("Make a copy")
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                        // The open field's name, else the
                                        // suggested one shown in it.
                                        let name = match this.editing.take() {
                                            Some((t, f)) if t.key == COPY_KEY => f.text,
                                            other => {
                                                this.editing = other;
                                                this.settle();
                                                suggested.clone()
                                            }
                                        };
                                        this.make_copy(&name);
                                        cx.notify();
                                    }),
                                ),
                        ),
                );
            if let Some((_, why)) = self.error.as_ref().filter(|(k, _)| *k == COPY_KEY) {
                out = out.child(
                    div()
                        .text_size(px(t.size(SUMMARY_FONT_PX) * ui))
                        .text_color(chrome.warn)
                        .child(why.clone()),
                );
            }
        }
        out.into_any_element()
    }

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
                    // A long key goes to the next line rather than being
                    // cut (Tarik, 2026-11-21: the text was cut off).
                    .flex_wrap()
                    .min_w_0()
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
                            .flex_none()
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
                    // Wrapped, never cut: the first sentence of the doc.
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
        if let Some(picker) = self.render_picker(row, chrome, ui, cx) {
            out = out.child(picker);
        }
        if let Some(items) = self.render_items(row, chrome, ui, cx) {
            out = out.child(items);
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
                .child(match settings_rows::display(&row.value) {
                    // A theme key the file does not set.
                    s if s.is_empty() => div().text_color(chrome.text_faint).child("built-in"),
                    s => div().child(s),
                })
                .child(div().text_color(chrome.text_muted).child("\u{25be}"))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| this.click_choice(key, cx)),
                )
                .into_any_element(),
            // Two stops, each its own cell; both emptied is "off".
            Kind::ColorPair => {
                let pair = row.list();
                let stop = |i: usize| {
                    pair.get(i)
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string()
                };
                div()
                    .w(px(CONTROL_W_PX * ui))
                    .flex()
                    .flex_row()
                    .gap(px(6. * ui))
                    .children((0..2).map(|i| {
                        let text = stop(i);
                        let target = Target {
                            key,
                            item: Some(i),
                            part: None,
                        };
                        let swatch = swatch_color(text.trim());
                        self.cell(
                            target,
                            text,
                            if i == 0 { "from" } else { "to" },
                            chrome,
                            ui,
                            cx,
                        )
                        .children(swatch.map(|c| {
                            div()
                                .absolute()
                                .right(px(4. * ui))
                                .top(px(5. * ui))
                                .size(px(SWATCH_PX * ui * 0.8))
                                .rounded_sm()
                                .bg(c)
                        }))
                        .relative()
                    }))
                    .into_any_element()
            }
            // The items are drawn under the row (`render_items`).
            Kind::PathList | Kind::IconMap | Kind::NameList(_) => {
                let count = if row.kind == Kind::IconMap {
                    settings_rows::map_entries(row.raw.as_ref().unwrap_or(&row.value)).len()
                } else {
                    row.list().len()
                };
                let action: Option<(&'static str, bool)> = match row.kind {
                    Kind::PathList => Some(("Add picture\u{2026}", true)),
                    Kind::IconMap => Some(("Add", false)),
                    _ => None,
                };
                div()
                    .w(px(CONTROL_W_PX * ui))
                    .flex()
                    .flex_row()
                    .justify_between()
                    .child(div().text_color(chrome.text_muted).child(match count {
                        0 if row.set => "none".to_string(),
                        0 => "built-in".to_string(),
                        1 => "1 item".to_string(),
                        n => format!("{n} items"),
                    }))
                    .children(action.map(|(label, picture)| {
                        div()
                            .cursor_pointer()
                            .text_color(chrome.focus_ring)
                            .child(label)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                    if picture {
                                        this.add_picture(key, cx)
                                    } else {
                                        let target = Target {
                                            key,
                                            item: Some(count),
                                            part: None,
                                        };
                                        this.click_item(target, String::new(), cx)
                                    }
                                }),
                            )
                    }))
                    .into_any_element()
            }
            Kind::Number | Kind::OptionalNumber | Kind::Text | Kind::Color | Kind::ColorWord => {
                let editing = self
                    .editing
                    .as_ref()
                    .filter(|(t, _)| *t == Target::key(key));
                let text = match editing {
                    Some((_, f)) => f.text.clone(),
                    None => settings_rows::display(&row.value),
                };
                let swatch = matches!(row.kind, Kind::Color | Kind::ColorWord)
                    .then(|| swatch_color(text.trim()))
                    .flatten();
                let shown = match editing {
                    Some((_, f)) => {
                        f.inline_composing(chrome.sel_bg, chrome.sel_fg, self.composing.as_deref())
                    }
                    None if text.is_empty() => {
                        div()
                            .text_color(chrome.text_faint)
                            .child(if row.theme.is_some() {
                                "built-in"
                            } else if row.kind == Kind::OptionalNumber {
                                "unset"
                            } else {
                                "empty"
                            })
                    }
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
            // A searchable list of what is installed.
            Kind::Font | Kind::TerminalTheme | Kind::UiTheme => {
                let kind = row.kind;
                let text = settings_rows::display(&row.value);
                boxed(div())
                    .border_color(if self.picking.as_ref().is_some_and(|p| p.key == key) {
                        chrome.focus_ring
                    } else {
                        chrome.control_border
                    })
                    .flex()
                    .flex_row()
                    .justify_between()
                    .gap(px(6. * ui))
                    .cursor_pointer()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(if text.is_empty() {
                                "default".to_string()
                            } else {
                                text
                            }),
                    )
                    .child(div().text_color(chrome.text_muted).child("\u{25be}"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.open_picker(key, kind, cx)
                        }),
                    )
                    .into_any_element()
            }
            // The system's folder panel; empty means home.
            Kind::Folder => {
                let text = settings_rows::display(&row.value);
                boxed(div())
                    .border_color(chrome.control_border)
                    .flex()
                    .flex_row()
                    .justify_between()
                    .gap(px(6. * ui))
                    .cursor_pointer()
                    .child(if text.is_empty() {
                        div().text_color(chrome.text_faint).child("home folder")
                    } else {
                        div().flex_1().min_w_0().truncate().child(text)
                    })
                    .child(div().text_color(chrome.focus_ring).child("Choose\u{2026}"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.choose_folder(key, cx)
                        }),
                    )
                    .into_any_element()
            }
            // The items are drawn under the row (`render_items`); the
            // control is the way to add one.
            Kind::TextList | Kind::NumberList | Kind::SearchEngines => {
                let len = row.list().len();
                let target = Target {
                    key,
                    item: Some(len),
                    part: (row.kind == Kind::SearchEngines).then_some("keyword"),
                };
                div()
                    .w(px(CONTROL_W_PX * ui))
                    .flex()
                    .flex_row()
                    .justify_between()
                    .child(div().text_color(chrome.text_muted).child(match len {
                        0 => "none".to_string(),
                        1 => "1 item".to_string(),
                        n => format!("{n} items"),
                    }))
                    .child(
                        div()
                            .cursor_pointer()
                            .text_color(chrome.focus_ring)
                            .child("Add")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                    this.click_item(target, String::new(), cx)
                                }),
                            ),
                    )
                    .into_any_element()
            }
        }
    }

    /// One editable cell: the open field when it is the target, else its
    /// text (or a faint placeholder). A click opens it.
    fn cell(
        &self,
        target: Target,
        text: String,
        placeholder: &'static str,
        chrome: &Chrome,
        ui: f32,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let editing = self.editing.as_ref().filter(|(t, _)| *t == target);
        let shown = match editing {
            Some((_, f)) => {
                f.inline_composing(chrome.sel_bg, chrome.sel_fg, self.composing.as_deref())
            }
            None if text.is_empty() => div().text_color(chrome.text_faint).child(placeholder),
            None => div().truncate().child(text.clone()),
        };
        div()
            .flex_1()
            .min_w_0()
            .px(px(8. * ui))
            .py(px(4. * ui))
            .rounded_md()
            .border_1()
            .border_color(if editing.is_some() {
                chrome.focus_ring
            } else {
                chrome.control_border
            })
            .bg(chrome.control_bg)
            .cursor_text()
            .child(shown)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                    this.click_item(target, text.clone(), cx)
                }),
            )
    }

    /// A list key's items under its row, one line each with a remove
    /// button, plus the new item while "Add" has its field open. A search
    /// engine is three cells, and says why when it is not used yet.
    fn render_items(
        &self,
        row: &Row,
        chrome: &Chrome,
        ui: f32,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let key = row.key;
        if let Kind::NameList(names) = row.kind {
            return Some(self.render_names(row, names, chrome, ui, cx));
        }
        // An icon map's entries read as `program=icon`, one cell each.
        let list: Vec<Value> = match row.kind {
            Kind::TextList | Kind::NumberList | Kind::SearchEngines | Kind::PathList => row.list(),
            Kind::IconMap => settings_rows::map_entries(row.raw.as_ref().unwrap_or(&row.value))
                .into_iter()
                .map(|(k, v)| Value::from(format!("{k}={v}")))
                .collect(),
            _ => return None,
        };
        let adding = self
            .editing
            .as_ref()
            .and_then(|(t, _)| (t.key == key && t.item == Some(list.len())).then_some(()))
            .is_some();
        if list.is_empty() && !adding {
            return None;
        }
        let engines = row.kind == Kind::SearchEngines;
        let t = &chrome.typography;
        let mut out = div()
            .flex()
            .flex_col()
            .gap(px(4. * ui))
            .pt(px(6. * ui))
            .pl(px((SET_DOT_PX + 6.) * ui));
        let count = list.len() + usize::from(adding);
        for i in 0..count {
            let item = list.get(i).cloned().unwrap_or(Value::Null);
            let mut line = div().flex().flex_row().items_center().gap(px(6. * ui));
            if engines {
                for (part, placeholder) in settings_rows::ENGINE_FIELDS.iter().zip([
                    "keyword",
                    "name",
                    "https://example.com/?q=%s",
                ]) {
                    let text = item
                        .get(*part)
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let target = Target {
                        key,
                        item: Some(i),
                        part: Some(part),
                    };
                    line = line.child(self.cell(target, text, placeholder, chrome, ui, cx));
                }
            } else {
                let target = Target {
                    key,
                    item: Some(i),
                    part: None,
                };
                let placeholder = match row.kind {
                    Kind::NumberList => "column",
                    Kind::PathList => "a picture's path",
                    Kind::IconMap => "program=icon",
                    _ => "NAME=value",
                };
                line = line.child(self.cell(
                    target,
                    settings_rows::display(&item),
                    placeholder,
                    chrome,
                    ui,
                    cx,
                ));
            }
            if i < list.len() {
                line = line.child(
                    div()
                        .flex_none()
                        .w(px(20. * ui))
                        .cursor_pointer()
                        .text_color(chrome.text_muted)
                        .child("\u{00d7}")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                this.remove_item(key, i, cx)
                            }),
                        ),
                );
            }
            out = out.child(line);
            if engines && i < list.len() {
                if let Some(why) = settings_rows::engine_problem(&item) {
                    out = out.child(
                        div()
                            .text_size(px(t.size(SUMMARY_FONT_PX) * ui))
                            .text_color(chrome.warn)
                            .child(why),
                    );
                }
            }
        }
        Some(out.into_any_element())
    }

    /// A name list's checkboxes, one per name it may hold, in the fixed
    /// order; a tick adds the name at the end, as the file lists them.
    fn render_names(
        &self,
        row: &Row,
        names: &'static [&'static str],
        chrome: &Chrome,
        ui: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let key = row.key;
        let list = row.list();
        names
            .iter()
            .fold(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(ROW_GAP_PX * ui))
                    .pt(px(6. * ui))
                    .pl(px((SET_DOT_PX + 6.) * ui)),
                |out, &name| {
                    let on = list.iter().any(|v| v.as_str() == Some(name));
                    out.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6. * ui))
                            .cursor_pointer()
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
                                    .when(on, |d| d.child("\u{2713}")),
                            )
                            .child(name)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                    this.toggle_name(key, name, cx)
                                }),
                            ),
                    )
                },
            )
            .into_any_element()
    }

    /// The searchable list under a font or theme row: a filter field and
    /// the matches, best first, the current one marked.
    fn render_picker(
        &self,
        row: &Row,
        chrome: &Chrome,
        ui: f32,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let p = self.picking.as_ref().filter(|p| p.key == row.key)?;
        let key = row.key;
        let current = settings_rows::display(&row.value);
        let matches = settings_rows::pick_matches(&p.items, &p.filter.text);
        let filter = if p.filter.text.is_empty() {
            Field::empty_with_placeholder("Type to filter").text_color(chrome.text_faint)
        } else {
            p.filter
                .inline_composing(chrome.sel_bg, chrome.sel_fg, self.composing.as_deref())
        };
        let list = matches.iter().take(PICKER_MAX_ROWS).fold(
            div()
                .id(gpui::SharedString::from(format!("pick-{key}")))
                .max_h(px(PICKER_H_PX * ui))
                .overflow_y_scroll()
                .flex()
                .flex_col(),
            |list, &item| {
                let here = item == current;
                let owned = item.to_string();
                list.child(
                    div()
                        .px(px(8. * ui))
                        .py(px(3. * ui))
                        .rounded_sm()
                        .cursor_pointer()
                        .when(here, |d| {
                            d.bg(chrome.row_selected).text_color(chrome.text_bright)
                        })
                        .child(owned.clone())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                this.pick(key, owned.clone(), cx)
                            }),
                        ),
                )
            },
        );
        Some(
            div()
                .mt(px(6. * ui))
                .ml_auto()
                .mr(px((44. + ROW_GAP_PX) * ui))
                .w(px(PICKER_W_PX * ui))
                .p(px(4. * ui))
                .rounded_md()
                .border_1()
                .border_color(chrome.overlay_border)
                .bg(chrome.bar_bg)
                .flex()
                .flex_col()
                .gap(px(4. * ui))
                .child(
                    div()
                        .px(px(8. * ui))
                        .py(px(4. * ui))
                        .rounded_sm()
                        .border_1()
                        .border_color(chrome.focus_ring)
                        .bg(chrome.control_bg)
                        .child(filter),
                )
                .child(if matches.is_empty() {
                    div()
                        .px(px(8. * ui))
                        .text_color(chrome.text_muted)
                        .child("Nothing matches.")
                        .into_any_element()
                } else {
                    list.into_any_element()
                })
                .into_any_element(),
        )
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
