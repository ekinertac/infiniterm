//! The card switcher's panel (Ctrl+Tab): the rows `Model::switcher` froze
//! when it opened, the selection on one of them, drawn like the palette so
//! it reads as the same family. Nothing here decides anything: the order
//! and the earning rule are `infiniterm_core::switcher`, the stepping and
//! the commit are `model/focus_cmd.rs`, and the commit on Ctrl's release is
//! the root element's `on_modifiers_changed` in overlays.rs.
//!
//! The canvas does not move while you step. Panning across twenty cards on
//! every Tab would be motion for nothing; the commit focuses and fits the
//! card the way a palette card row does.
use crate::AppView;
use gpui::{div, prelude::*, px, IntoElement};
use infiniterm_core::agent_state::AgentState;

/// Rows shown at once; the selection is kept inside the window.
const SWITCHER_ROWS: usize = 12;
/// Narrower than the palette: a row is a number, a label and a workspace.
const SWITCHER_WIDTH_PX: f32 = 520.;
/// The agent dot beside a row, the tab dots' size.
const SWITCHER_DOT_PX: f32 = 7.;

impl AppView {
    pub fn render_switcher(&self) -> impl IntoElement {
        let ui = self.model.ui_scale as f32;
        let chrome = &self.chrome;
        let Some(s) = &self.model.switcher else {
            return div();
        };
        let active_ws = self.model.active_workspace.clone();
        let start = s
            .index
            .saturating_sub(SWITCHER_ROWS - 1)
            .min(s.list.len().saturating_sub(SWITCHER_ROWS));
        let mut list = div().flex().flex_col();
        for (i, id) in s.list.iter().enumerate().skip(start).take(SWITCHER_ROWS) {
            let Some(card) = self.model.card(id) else {
                continue;
            };
            let label = self.model.numbered_label(card);
            // The workspace only when it is not the one on screen: the same
            // rule the palette's card rows follow.
            let elsewhere = (active_ws.as_deref() != Some(card.workspace_id.as_str()))
                .then(|| {
                    self.model
                        .workspaces
                        .iter()
                        .find(|w| w.id == card.workspace_id)
                        .map(|w| w.name.clone())
                })
                .flatten();
            let dot = match card.agent {
                AgentState::Working => Some(chrome.agent_working),
                AgentState::Waiting => Some(chrome.agent_waiting),
                AgentState::Failed => Some(chrome.agent_failed),
                AgentState::Done => Some(chrome.agent_done),
                AgentState::None => None,
            };
            let mut row = div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .when(i == s.index, |d| d.bg(chrome.row_selected))
                .child(
                    div()
                        .w(px(SWITCHER_DOT_PX * ui))
                        .h(px(SWITCHER_DOT_PX * ui))
                        .rounded_full()
                        .when_some(dot, |d, c| d.bg(c)),
                )
                .child(div().flex_1().truncate().child(label));
            if let Some(ws) = elsewhere {
                row = row.child(div().text_color(chrome.text_faint).child(ws));
            }
            list = list.child(row);
        }
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .justify_center()
            .pt(px(crate::overlays::OVERLAY_TOP_PAD_PX * ui))
            .child(
                div()
                    .w(px(SWITCHER_WIDTH_PX * ui))
                    .h(px(0.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .py_1()
                            .bg(chrome.overlay_bg)
                            .border_1()
                            .border_color(chrome.overlay_border)
                            .rounded_md()
                            .shadow_lg()
                            .font_family(chrome.typography.family.clone())
                            .font_weight(chrome.typography.regular.weight)
                            .text_size(px(chrome
                                .typography
                                .size(crate::overlays::OVERLAY_BODY_FONT_PX)
                                * ui))
                            .text_color(chrome.text)
                            .child(list),
                    ),
            )
    }
}
