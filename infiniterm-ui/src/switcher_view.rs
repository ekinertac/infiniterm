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
/// Narrower than the palette: a row is a number, a label, what is open in
/// the card, its kind and a workspace.
const SWITCHER_WIDTH_PX: f32 = 560.;
/// The agent dot beside a row, the tab dots' size.
const SWITCHER_DOT_PX: f32 = 7.;
/// How much of the agent's colour fills a row that is not the selection:
/// enough to tell the states apart at a glance, faint enough that the
/// selection's fill still reads as the strongest row.
const SWITCHER_TINT_ALPHA: f32 = 0.14;
/// The program icon, a little over the body text so a glyph reads.
const SWITCHER_ICON_PX: f32 = 15.;

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
        // The program's icon (#746), the tab's and the header's: drawn when
        // the theme turns either of those on, chosen the same way.
        let theme = &self.model.ui_theme;
        let icons_on = theme.tab.icons == Some(true) || theme.card.header_icons == Some(true);
        let icon_px = SWITCHER_ICON_PX * ui;
        let icons_dir =
            infiniterm_core::config_files::ui_theme_icons_dir(&self.model.ui_theme_name);
        let shell = infiniterm_core::tab_icon::shell_name(
            &self.model.config.terminal.shell,
            std::env::var("SHELL").ok().as_deref(),
        );
        let icon_of = |card: &infiniterm_core::model::Card| -> Option<gpui::AnyElement> {
            if !icons_on {
                return None;
            }
            let has_file = |f: &str| {
                icons_dir.as_ref().is_some_and(|d| {
                    self.icon_files.borrow_mut().has_file(
                        d,
                        f,
                        crate::now_ms(),
                        |p| p.metadata().and_then(|m| m.modified()).ok(),
                        |p| p.is_file(),
                    )
                })
            };
            let icon = infiniterm_core::tab_icon::choose_subject(
                infiniterm_core::tab_icon::subject(
                    card.kind,
                    card.proc.as_deref(),
                    card.agent,
                    card.agent_kind.as_deref(),
                    card.agent_session.as_deref(),
                    self.model.program_owns_state(&card.id),
                )
                .unwrap_or(infiniterm_core::tab_icon::Subject::Program(&shell)),
                theme.tab.icon_style.unwrap_or_default(),
                &theme.tab.icon_map,
                &has_file,
            );
            // The icon font is only a FALLBACK (`icon_font.rs`), so the
            // glyph goes through a run whose font names it.
            match icon {
                infiniterm_core::tab_icon::Icon::Glyph(glyph) => {
                    let run = gpui::TextRun {
                        len: glyph.len(),
                        font: crate::terminal_body::term_font(&chrome.typography.family),
                        color: chrome.text,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    Some(
                        div()
                            .flex_none()
                            .text_size(px(icon_px))
                            .child(gpui::StyledText::new(glyph).with_runs(vec![run]))
                            .into_any_element(),
                    )
                }
                infiniterm_core::tab_icon::Icon::File(file) => icons_dir.as_ref().map(|dir| {
                    gpui::img(dir.join(&file))
                        .flex_none()
                        .size(px(icon_px))
                        .object_fit(gpui::ObjectFit::Contain)
                        .into_any_element()
                }),
            }
        };
        let mut list = div().flex().flex_col();
        for (i, id) in s.list.iter().enumerate().skip(start).take(SWITCHER_ROWS) {
            let Some(card) = self.model.card(id) else {
                continue;
            };
            let text = self.model.switcher_row(card);
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
            let state = match card.agent {
                AgentState::Working => Some(chrome.agent_working),
                AgentState::Waiting => Some(chrome.agent_waiting),
                AgentState::Failed => Some(chrome.agent_failed),
                AgentState::Done => Some(chrome.agent_done),
                AgentState::None => None,
            };
            let selected = i == s.index;
            let mut row = div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .when(selected, |d| d.bg(chrome.row_selected))
                // The agent's state carries the row (#746), not only a dot:
                // the selection's own fill wins on the chosen row, the dot
                // still says the state there.
                .when_some(state.filter(|_| !selected), |d, c| {
                    d.bg(c.opacity(SWITCHER_TINT_ALPHA))
                })
                .child(
                    div()
                        .flex_none()
                        .w(px(SWITCHER_DOT_PX * ui))
                        .h(px(SWITCHER_DOT_PX * ui))
                        .rounded_full()
                        .when_some(state, |d, c| d.bg(c)),
                );
            if let Some(icon) = icon_of(card) {
                row = row.child(icon);
            }
            row = row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .gap_2()
                    .overflow_hidden()
                    .child(div().flex_none().max_w_full().truncate().child(text.label))
                    .when_some(text.detail, |d, detail| {
                        d.child(div().truncate().text_color(chrome.text_faint).child(detail))
                    }),
            );
            if let Some(kind) = text.kind {
                row = row.child(div().flex_none().text_color(chrome.text_faint).child(kind));
            }
            if let Some(ws) = elsewhere {
                row = row.child(div().flex_none().text_color(chrome.text_faint).child(ws));
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
