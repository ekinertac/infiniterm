//! What a workspace tab carries while it is dragged to a new place in the tab
//! row (#242): the value gpui hands to the tab it is dropped on, and the small
//! ghost that follows the pointer.
//!
//! Called by `overlays.rs` (`render_title_bar`: `on_drag`, `drag_over`,
//! `on_drop` on each tab). The reorder itself is `Model::move_workspace`,
//! which takes the dropped-on tab's index. A drag that starts on a tab still
//! selects it (the tab's own mouse-down), so dragging the tab you are not on
//! also switches to it.

use gpui::{div, px, Context, Hsla, IntoElement, ParentElement, Render, Styled, Window};

/// The workspace being dragged.
#[derive(Clone)]
pub struct DraggedTab {
    pub id: String,
    pub name: String,
    pub font_px: f32,
    pub bg: Hsla,
    pub fg: Hsla,
}

/// The tab's label, under the pointer.
pub struct TabGhost(pub DraggedTab);

impl Render for TabGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let d = &self.0;
        div()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(d.bg)
            .text_color(d.fg)
            .text_size(px(d.font_px))
            .child(d.name.clone())
    }
}
