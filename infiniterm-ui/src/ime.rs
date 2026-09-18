//! macOS's text input system, on the one view there is. This is what turns
//! the emoji panel, a dead key (Option+E, then E, for `é`) and an input
//! method (Pinyin, Japanese) into text in the focused card or field.
//!
//! gpui gives our `key_down` every key FIRST; only a key nobody took falls
//! through to macOS's input context, which is where composition happens and
//! where this handler is called back. So a body or a field that takes a key
//! says so (`BodyAction::Ignored` and `Edit::Ignored` are the "did not"),
//! and typed text can never arrive twice: once through `key_down` and once
//! through `replace_text_in_range`.
//!
//! Marked text (the `\u{b4}` before the `e`) is held and not drawn. Holding
//! it is what matters: while `marked_text_range` is `Some`, macOS routes
//! the next key to the input context before us, which is how the `e`
//! becomes `é` instead of a plain `e`. Drawing it is a later nicety.
//!
//! Registered from the canvas paint in overlays.rs, on the app's one focus
//! handle. Related: body.rs (`insert_text`, `caret_bounds`), field.rs
//! (`insert_text`), and gpui's own examples/input.rs, which this follows.
use crate::AppView;
use gpui::{Bounds, Context, EntityInputHandler, Pixels, UTF16Selection, Window};
use std::ops::Range;

impl AppView {
    /// Where committed text goes: the open field if there is one, else the
    /// focused card's body. The same order `key_down` uses, so the emoji
    /// panel lands where typing would have.
    fn insert_text(&mut self, text: &str, cx: &mut gpui::App) {
        if text.is_empty() {
            return;
        }
        if self.model.palette_open() {
            self.query_field.insert_text(text);
            self.model.palette.query = self.query_field.text.clone();
            self.model.palette.index = 0;
        } else if self.model.omni.open {
            self.omni_field.insert_text(text);
            let q = self.omni_field.text.clone();
            self.model.omni_type(&q);
        } else if self.model.find.open {
            self.find_field.insert_text(text);
            let q = self.find_field.text.clone();
            self.model.find_type(&q);
        } else if self.model.prompt.is_open() {
            self.prompt_field.insert_text(text);
            self.model.prompt.value = self.prompt_field.text.clone();
        } else if self.model.shortcuts_open {
            self.shortcuts_field.insert_text(text);
        } else if let Some(id) = self.model.selection.focused_id.clone() {
            if let Some(body) = self.bodies.get_mut(&id) {
                body.insert_text(text);
            }
            self.flush_writes();
        }
        let _ = cx;
        self.perform_effects();
        self.redraw = true;
    }
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

impl EntityInputHandler for AppView {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        // The only text macOS can ask us about is what it is composing.
        self.composing.clone()
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        // A terminal has no selection macOS can edit; the caret is at the
        // end of whatever is being composed.
        let end = self.composing.as_deref().map(utf16_len).unwrap_or(0);
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
        self.composing.as_deref().map(|m| 0..utf16_len(m))
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
        // The commit. Whatever was being composed is replaced by this.
        self.composing = None;
        self.insert_text(text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Composition in progress. Held, not drawn; see the header.
        self.composing = if new_text.is_empty() {
            None
        } else {
            Some(new_text.to_string())
        };
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // The candidate window sits under the caret when a body knows
        // where its caret is; the canvas's corner otherwise.
        let caret = self
            .model
            .selection
            .focused_id
            .as_ref()
            .and_then(|id| self.bodies.get(id))
            .and_then(|b| b.caret_bounds());
        Some(caret.unwrap_or(Bounds::new(
            element_bounds.origin,
            gpui::size(Pixels::ZERO, Pixels::ZERO),
        )))
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
