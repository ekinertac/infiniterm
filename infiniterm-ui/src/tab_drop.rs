//! Which workspace tab a dragged card is over (#667). A card drag is the
//! canvas's own gesture (`Gesture` in main.rs, mouse events in input.rs), not
//! gpui's drag system that a tab reorder uses (`tab_drag.rs`), so the two never
//! mix: this file only answers "is the pointer on a tab, and which".
//!
//! The title bar records each tab's window bounds every frame
//! (`overlays.rs`, `render_title_bar`); `input.rs` asks `tab_under` on each
//! mouse move of a card drag and on the drop. The tab of the workspace the
//! card is already in is never a target: dropping there is a drop on the
//! canvas, as the issue says.
//!
//! Related: `Model::drop_on_workspace` (what the drop does).

use gpui::{Bounds, Pixels, Point};

/// Each workspace's tab with its window bounds, shared between the title bar
/// that fills it every frame and the mouse handlers that read it.
pub type TabBounds = std::rc::Rc<std::cell::RefCell<Vec<(String, Bounds<Pixels>)>>>;

/// The workspace whose tab contains `at`, unless it is `active` (the one the
/// drag started in).
pub fn tab_under(
    tabs: &[(String, Bounds<Pixels>)],
    at: Point<Pixels>,
    active: Option<&str>,
) -> Option<String> {
    tabs.iter()
        .find(|(id, bounds)| bounds.contains(&at) && Some(id.as_str()) != active)
        .map(|(id, _)| id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px, size};

    fn tab(id: &str, x: f32) -> (String, Bounds<Pixels>) {
        (
            id.to_string(),
            Bounds::new(point(px(x), px(0.)), size(px(80.), px(24.))),
        )
    }

    #[test]
    fn the_pointer_is_over_the_tab_that_contains_it() {
        let tabs = vec![tab("a", 0.), tab("b", 90.), tab("c", 180.)];
        assert_eq!(
            tab_under(&tabs, point(px(100.), px(10.)), None).as_deref(),
            Some("b")
        );
        assert_eq!(
            tab_under(&tabs, point(px(5.), px(23.)), None).as_deref(),
            Some("a")
        );
    }

    #[test]
    fn a_gap_or_the_canvas_is_no_tab() {
        let tabs = vec![tab("a", 0.), tab("b", 90.)];
        // Between the two tabs, and below the bar.
        assert_eq!(tab_under(&tabs, point(px(85.), px(10.)), None), None);
        assert_eq!(tab_under(&tabs, point(px(10.), px(300.)), None), None);
        assert_eq!(tab_under(&[], point(px(10.), px(10.)), None), None);
    }

    #[test]
    fn the_workspace_the_card_is_in_is_never_a_target() {
        let tabs = vec![tab("a", 0.), tab("b", 90.)];
        assert_eq!(tab_under(&tabs, point(px(10.), px(10.)), Some("a")), None);
        assert_eq!(
            tab_under(&tabs, point(px(100.), px(10.)), Some("a")).as_deref(),
            Some("b")
        );
    }
}
