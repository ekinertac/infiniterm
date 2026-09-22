//! The text an input method is still composing, drawn at the caret of the
//! focused card. `ime.rs` holds it in `AppView::composing` so macOS keeps
//! composing (the `\u{b4}` before the `e`, a Pinyin syllable before its
//! candidates); this is what shows it, the way every Mac text view shows
//! marked text: in place, underlined, gone on the commit.
//!
//! Painted from `paint::frame` after the world and before the wash, so it
//! sits over the card's own text. A body says where its caret is through
//! `CardBody::caret_bounds` (the terminal's cursor cell, the editor's caret
//! cell); a body with no caret (a browser, a diff) draws nothing, and the
//! composition still commits through `insert_text` as before. The fields
//! draw their own marked text inline (`Field::inline_composing`).
//!
//! The font is the terminal's at the size the caret's cell implies, so the
//! marked text is the card's text size at the card's zoom without the body
//! having to export its metrics.
use crate::AppView;
use gpui::{fill, point, px, size, App, Bounds, Window};

/// The underline's thickness in screen pixels; macOS's marked-text
/// underline is one hairline plus a little at retina.
const UNDERLINE_PX: f32 = 1.5;

impl AppView {
    pub fn paint_composing(&self, window: &mut Window, cx: &mut App) {
        let Some(marked) = self.composing.as_deref().filter(|m| !m.is_empty()) else {
            return;
        };
        // A field that is open has the composition; the canvas does not.
        if self.model.overlay_open() {
            return;
        }
        let Some(caret) = self
            .model
            .selection
            .focused_id
            .as_ref()
            .and_then(|id| self.bodies.get(id))
            .and_then(|b| b.caret_bounds())
        else {
            return;
        };
        let metrics = self.metrics(window);
        // The cell's height is font size times line height at the zoom;
        // undo the line height for the size the glyphs were drawn at.
        let font_px = f32::from(caret.size.height) / metrics.line_height as f32;
        let line = crate::text::shape(
            window,
            marked,
            px(font_px),
            &metrics.font(),
            self.chrome.text_bright,
        );
        let under = Bounds::new(caret.origin, size(line.width, caret.size.height));
        window.paint_quad(fill(under, self.chrome.card_bg));
        let _ = line.paint(caret.origin, caret.size.height, window, cx);
        window.paint_quad(fill(
            Bounds::new(
                point(
                    caret.origin.x,
                    caret.origin.y + caret.size.height - px(UNDERLINE_PX),
                ),
                size(line.width, px(UNDERLINE_PX)),
            ),
            self.chrome.text_bright,
        ));
    }
}
