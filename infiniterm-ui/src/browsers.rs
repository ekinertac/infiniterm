//! Browser cards and their surfaces: one `BrowserBody` per browser card,
//! holding one CEF surface per tab, opened on the card's url at the card's
//! size and kept in step each frame (new pixels, the page's own zoom from
//! `card.zoom`, a url typed into `browser.navigate`, and `card.tabs`
//! against the body's own list), with what the page did written back: the
//! address it moved to onto the card, popups opened as TABS on the same
//! card, a right-click turned into the slim menu overlay, the page's own
//! focus mirrored onto `card.locked`. A tab command is plain `Card`
//! mutation in `tabs_cmd.rs` with no `Effect` behind it, so the diff here
//! is the whole of how one becomes a surface. CEF itself is started once
//! in `main.rs`; `cef_running` says whether it is.
use crate::browser_body::BrowserBody;
use crate::tab_strip::StripStyle;
use crate::{now_ms, AppView};
use infiniterm_core::grid::{Point, Size};
use infiniterm_core::ift::url_plan;
use infiniterm_core::model::{BrowserAction, FindRequest};
use infiniterm_core::saved_layout::CardKind;
use infiniterm_core::viewport::screen_pos_of;

/// A right-click's slim menu, positioned once at open time in content-area
/// screen pixels. Not re-derived from the card each frame: the menu is up
/// for a couple of clicks at most, and tracking a pan or zoom mid-menu
/// would cost a frame of extra state for a case that does not come up.
pub struct CardContextMenu {
    pub card_id: String,
    pub x: f64,
    pub y: f64,
    pub link_url: Option<String>,
    pub editable: bool,
    pub has_selection: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub page_url: String,
}

/// What a slim menu item does. Copy-able so a render closure can capture it
/// by value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuAction {
    Back,
    Forward,
    Reload,
    Cut,
    Copy,
    Paste,
    CopyLinkAddress,
    OpenLinkInNewCard,
    OpenLinkInNewTab,
    OpenInSystemBrowser,
    OpenLinkInSystemBrowser,
}

/// What the body has to do to hold the tabs the card says it has.
#[derive(Debug, PartialEq, Eq)]
enum TabDiff {
    /// No tab opened or closed. The urls may still differ: a tab that
    /// navigated in place is the navigate branch's business, not a
    /// structural change.
    Keep,
    Added(String),
    RemovedAt(usize),
    /// Neither one-step case fits, so nothing can say WHICH tabs moved:
    /// build the list again from the card's. The expensive one, and the
    /// only one that always converges.
    Rebuild,
}

/// A `tabs_cmd.rs` command changes the list by exactly one, so one step is
/// all a frame normally has to catch up on. A frame can carry more than one
/// though: two popups from the same page drain into two `browser_tab_open`
/// calls before the next reconcile ever runs. Leaving that case alone
/// stranded the body at the wrong tab count for the rest of the run, with
/// `set_active` refusing an out-of-range index, so the card's label said one
/// address while the body painted another.
fn tab_diff(old: &[String], new: &[String]) -> TabDiff {
    if let Some(added) = diff_added(old, new) {
        return TabDiff::Added(added);
    }
    if let Some(at) = diff_removed_index(old, new) {
        return TabDiff::RemovedAt(at);
    }
    if old.len() == new.len() {
        return TabDiff::Keep;
    }
    TabDiff::Rebuild
}

/// The one url `new` has that `old` does not, assuming at most one tab was
/// appended since the last frame (every `tabs_cmd.rs` command that grows the
/// list appends). `None` when the lengths do not differ by exactly +1.
fn diff_added(old: &[String], new: &[String]) -> Option<String> {
    if new.len() != old.len() + 1 {
        return None;
    }
    new.last().cloned()
}

/// Which position in `old` is missing from `new`, assuming at most one was
/// removed and every url that survived kept its order. `close_tab` needs the
/// exact index: the closed tab is not always the last one.
fn diff_removed_index(old: &[String], new: &[String]) -> Option<usize> {
    if old.len() != new.len() + 1 {
        return None;
    }
    Some(
        old.iter()
            .zip(new.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(new.len()),
    )
}

impl AppView {
    pub fn browser_for(&mut self, id: &str) -> Option<&mut BrowserBody> {
        self.bodies
            .get_mut(id)
            .and_then(|b| b.as_any_mut().downcast_mut::<BrowserBody>())
    }

    pub fn reconcile_browsers(&mut self) {
        let cef = self.cef_running;
        let scale = self.scale_factor;
        let card_bg = self.chrome.card_bg;
        let text = self.chrome.text;
        let family = crate::terminals::family_of(&self.model.config.terminal.font_family);
        // The same `StripStyle` the editor's strip takes, so the two look
        // and size alike: `terminal.fontSize` is the strip's font too, not
        // a strip-only constant.
        let style = StripStyle {
            bg: self.chrome.bar_bg,
            border: self.chrome.bar_border,
            active_bg: self.chrome.row_selected,
            text_bright: self.chrome.text_bright,
            text_muted: self.chrome.text_muted,
            font_family: family.clone(),
            font_px: self.model.config.terminal.font_size,
        };
        let inactive_dim = self.model.config.ui.inactive_dim;
        let ui_scale = self.model.ui_scale as f32;
        let cards: Vec<_> = self
            .model
            .cards
            .iter()
            .filter(|c| c.kind == CardKind::Browser)
            .cloned()
            .collect();
        let mut opens: Vec<(String, String)> = vec![];
        let mut moved: Vec<(String, String)> = vec![];
        let mut titles: Vec<(String, String)> = vec![];
        let mut retabs: Vec<(String, Vec<String>)> = vec![];
        let mut locks: Vec<(String, bool)> = vec![];
        let mut menu: Option<(String, Point, infiniterm_browser::ContextMenuRequest)> = None;
        for card in cards {
            let world = Size {
                w: card.rect.w,
                h: card.rect.h,
            };
            let url = card.url.clone().unwrap_or_else(|| "about:blank".into());
            if self.browser_for(&card.id).is_none() {
                // A restored card arrives with every tab it was saved with,
                // so the body is built holding all of them. `tab_diff` would
                // rebuild it into the same shape a frame later anyway; doing
                // it here is a CEF browser opened and closed for nothing on
                // every restored card, saved.
                let first = card.tabs.first().unwrap_or(&url);
                let mut body = BrowserBody::new(&card.id, first, world, scale, cef, style.clone());
                for later in card.tabs.iter().skip(1) {
                    body.open_tab(later);
                }
                self.bodies.insert(card.id.clone(), Box::new(body));
            }
            let Some(body) = self.browser_for(&card.id) else {
                continue;
            };
            body.card_bg = card_bg;
            body.text = text;
            body.style = style.clone();
            body.font_family = family.clone();
            body.inactive_dim = inactive_dim;
            body.card_number = card.number;
            body.protected = card.protected;
            body.ui_scale = ui_scale;
            body.zoom = card.zoom;

            // Tabs: the card's list of urls against the body's list of
            // surfaces, `tab_diff` saying what that means. A single-tab card
            // keeps `tabs` empty, so its one url stands in and nothing about
            // it ever changes.
            let desired: Vec<String> = if card.tabs.is_empty() {
                vec![url.clone()]
            } else {
                card.tabs.clone()
            };
            let actual: Vec<String> = body.tabs.iter().map(|t| t.url.clone()).collect();
            match tab_diff(&actual, &desired) {
                TabDiff::Keep => {}
                TabDiff::Added(added) => body.open_tab(&added),
                TabDiff::RemovedAt(at) => body.close_tab(at),
                TabDiff::Rebuild => body.rebuild_tabs(&desired),
            }
            // The active index arrives from the card the way `card.zoom`
            // does; it has to be applied after the open or close, or it
            // would name a tab the body does not have yet.
            body.set_active(if card.tabs.is_empty() {
                0
            } else {
                card.active_tab
            });

            // `browser.navigate` or the omnibox changed the card's url: the
            // active tab follows, exactly as a single-tab card always has.
            if body.active_url() != url && url != "about:blank" {
                body.navigate(&url);
            }
            if let Some(new_url) = body.sync() {
                moved.push((card.id.clone(), new_url));
            }
            if let Some(title) = body.take_title() {
                titles.push((body.active_url().to_string(), title));
            }
            // Where the tabs actually are now, written back onto the card:
            // a BACKGROUND tab that redirected itself moves no card url
            // through `moved`, and without this the card would send it back
            // to the old address the moment you switched to it. Only ever an
            // address-for-address swap: a body of a different length is one
            // that has not caught up with a structural change yet, and
            // writing that back would drop a tab the card still holds.
            let after: Vec<String> = body.tabs.iter().map(|t| t.url.clone()).collect();
            if !card.tabs.is_empty() && after.len() == card.tabs.len() && after != card.tabs {
                retabs.push((card.id.clone(), after));
            }

            // Lock, mirrored one way only, body to card: `page_focused` IS
            // the lock. Not saved (see `Card.locked`), so no dirty_layout.
            let locked = body.page_focused;
            body.locked = locked;
            if card.locked != locked {
                locks.push((card.id.clone(), locked));
            }

            for popup in std::mem::take(&mut body.popups) {
                // The point of tabs: a popup, a target=_blank link or a
                // Cmd+click opens a tab on the SAME card, not a new card.
                opens.push((card.id.clone(), popup));
            }
            if let Some(request) = body.context_menu.take() {
                menu = Some((
                    card.id.clone(),
                    Point {
                        x: card.rect.x,
                        y: card.rect.y,
                    },
                    request,
                ));
            }
        }
        if let Some((card_id, origin, request)) = menu {
            let world = Point {
                x: origin.x + request.x as f64,
                y: origin.y + request.y as f64,
            };
            let screen = screen_pos_of(world, self.model.viewport);
            self.context_menu = Some(CardContextMenu {
                card_id,
                x: screen.x,
                y: screen.y,
                link_url: request.link_url,
                editable: request.editable,
                has_selection: request.has_selection,
                can_go_back: request.can_go_back,
                can_go_forward: request.can_go_forward,
                page_url: request.page_url,
            });
        }
        for (id, urls) in retabs {
            if let Some(c) = self.model.card_mut(&id) {
                c.tabs = urls;
                // `card.url` stays what it has always been, the active
                // tab's page, so the label, the omnibox's prefill and the
                // save file need know nothing about tabs.
                c.url = c.tabs.get(c.active_tab).cloned();
            }
            self.model.dirty_layout = true;
            self.redraw = true;
        }
        for (id, locked) in locks {
            if let Some(c) = self.model.card_mut(&id) {
                c.locked = locked;
            }
            // The status bar was built from the old lock state before this
            // ran, and a locked page that is done painting asks for no
            // further frames.
            self.redraw = true;
        }
        for (id, url) in moved {
            // Every navigation passes through here, which is the only place
            // the app sees one: history is recorded here rather than in the
            // model, which never learns where a page went by itself.
            self.model.record_visit(&url, None, now_ms());
            if let Some(c) = self.model.card_mut(&id) {
                c.url = Some(url);
                self.model.dirty_layout = true;
                // The label and the status bar were drawn from the old
                // address before this ran, and a page that has finished
                // loading asks for no further frames: without this the card
                // says where it used to be until something else moves.
                self.redraw = true;
            }
        }
        for (url, title) in titles {
            self.model.history.set_title(&url, &title);
        }
        for (id, url) in opens {
            self.model.browser_tab_open(&id, Some(&url));
        }
    }

    /// A slim menu item was chosen: closes the menu and runs it. Copying the
    /// link and opening it in a new card need no surface, everything else
    /// does, which is why those two return early rather than sharing the
    /// match below with a borrow of `self.bodies` already open.
    pub fn context_menu_choose(&mut self, action: ContextMenuAction, cx: &mut gpui::App) {
        let Some(menu) = self.context_menu.take() else {
            return;
        };
        match action {
            ContextMenuAction::CopyLinkAddress => {
                if let Some(url) = menu.link_url {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(url));
                }
                return;
            }
            ContextMenuAction::OpenLinkInNewCard => {
                if let Some(url) = menu.link_url {
                    let plan = url_plan(&url, "");
                    self.model.open_in_card(plan, Some(&menu.card_id));
                }
                return;
            }
            ContextMenuAction::OpenLinkInNewTab => {
                if let Some(url) = menu.link_url {
                    self.model.browser_tab_open(&menu.card_id, Some(&url));
                }
                return;
            }
            ContextMenuAction::OpenInSystemBrowser | ContextMenuAction::OpenLinkInSystemBrowser => {
                let url = if action == ContextMenuAction::OpenLinkInSystemBrowser {
                    menu.link_url
                } else {
                    Some(menu.page_url).filter(|u| !u.is_empty())
                };
                if let Some(url) = url {
                    if let Err(e) = infiniterm_core::links_fs::open_url(&url) {
                        self.model.notify(format!("could not open {url}"));
                        eprintln!("[infiniterm] {e}");
                    }
                }
                return;
            }
            _ => {}
        }
        let Some(surface) = self
            .browser_for(&menu.card_id)
            .and_then(|b| b.active_surface())
        else {
            return;
        };
        match action {
            ContextMenuAction::Back => surface.back(),
            ContextMenuAction::Forward => surface.forward(),
            ContextMenuAction::Reload => surface.reload(),
            ContextMenuAction::Cut => {
                surface.edit_chord("x", false);
            }
            ContextMenuAction::Copy => {
                surface.edit_chord("c", false);
            }
            ContextMenuAction::Paste => {
                surface.edit_chord("v", false);
            }
            ContextMenuAction::CopyLinkAddress
            | ContextMenuAction::OpenLinkInNewCard
            | ContextMenuAction::OpenLinkInNewTab
            | ContextMenuAction::OpenInSystemBrowser
            | ContextMenuAction::OpenLinkInSystemBrowser => {
                unreachable!("handled above, before the surface borrow")
            }
        }
    }

    /// The page's right-click menu, native like the others (#281): Chromium
    /// said what is under the pointer (`context_menu`), this builds the rows
    /// from it and opens them from a task, as every native menu must
    /// (`native_menu.rs`). The choice goes through `context_menu_choose`; a
    /// dismissed menu just forgets the request.
    pub fn show_page_menu(&mut self, cx: &mut gpui::Context<Self>) {
        use crate::native_menu::{pop_up, Row};
        if self.page_menu_open {
            return;
        }
        let Some(menu) = &self.context_menu else {
            return;
        };
        let mut actions: Vec<ContextMenuAction> = vec![];
        let mut item = |title: &str, action: ContextMenuAction, chord: &str| -> Row {
            actions.push(action);
            Row::Item {
                tag: actions.len() as i64 - 1,
                title: title.into(),
                enabled: true,
                chord: (!chord.is_empty()).then(|| chord.to_string()),
            }
        };
        // What Chrome shows: Back, Forward and Reload belong to the page, so
        // not on a link, a selection or a field, and Back and Forward only
        // when there is somewhere to go.
        let mut rows: Vec<Row> = vec![];
        let on_page = menu.link_url.is_none() && !menu.editable && !menu.has_selection;
        if on_page {
            if menu.can_go_back {
                rows.push(item("Back", ContextMenuAction::Back, ""));
            }
            if menu.can_go_forward {
                rows.push(item("Forward", ContextMenuAction::Forward, ""));
            }
            rows.push(item("Reload", ContextMenuAction::Reload, ""));
        }
        if menu.editable || menu.has_selection {
            if !rows.is_empty() {
                rows.push(Row::Separator);
            }
            if menu.editable {
                rows.push(item("Cut", ContextMenuAction::Cut, "cmd+x"));
            }
            rows.push(item("Copy", ContextMenuAction::Copy, "cmd+c"));
            if menu.editable {
                rows.push(item("Paste", ContextMenuAction::Paste, "cmd+v"));
            }
        }
        if menu.link_url.is_some() {
            if !rows.is_empty() {
                rows.push(Row::Separator);
            }
            rows.push(item(
                "Open Link in New Tab",
                ContextMenuAction::OpenLinkInNewTab,
                "",
            ));
            rows.push(item(
                "Open Link in New Card",
                ContextMenuAction::OpenLinkInNewCard,
                "",
            ));
            rows.push(item(
                "Copy Link Address",
                ContextMenuAction::CopyLinkAddress,
                "",
            ));
        }
        // The footer of every browser menu.
        if !rows.is_empty() {
            rows.push(Row::Separator);
        }
        if menu.link_url.is_some() {
            rows.push(item(
                "Open Link in System Browser",
                ContextMenuAction::OpenLinkInSystemBrowser,
                "",
            ));
        } else {
            rows.push(item(
                "Open Page in System Browser",
                ContextMenuAction::OpenInSystemBrowser,
                "",
            ));
        }
        self.page_menu_open = true;
        cx.spawn(async move |view, cx| {
            let chosen = pop_up(&rows);
            let _ = view.update(cx, |this, cx| {
                this.page_menu_open = false;
                match chosen.and_then(|tag| actions.get(tag as usize).copied()) {
                    Some(action) => this.context_menu_choose(action, cx),
                    None => this.context_menu = None,
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl AppView {
    /// Back and forward. The page owns its history, so this is the whole
    /// implementation: the model only says which card and which direction.
    pub fn browser_effect(&mut self, card_id: &str, action: BrowserAction) {
        let Some(body) = self
            .bodies
            .get_mut(card_id)
            .and_then(|b| b.as_any_mut().downcast_mut::<BrowserBody>())
        else {
            return;
        };
        let Some(surface) = body.active_surface() else {
            return;
        };
        match action {
            BrowserAction::Back => surface.back(),
            BrowserAction::Forward => surface.forward(),
            BrowserAction::Reload => surface.reload(),
        }
    }

    /// The mouse's back and forward buttons, over a browser page. Back on a
    /// tab with no history of its own (one a link opened) closes it, as in
    /// Chrome, when the card has another tab to land on.
    pub fn mouse_navigate(&mut self, e: &gpui::MouseDownEvent, forward: bool) {
        if self.model.modal_open() {
            return;
        }
        let p = self.to_content(e.position);
        let crate::input::Hit::CardBody { id, .. } = self.hit(p) else {
            return;
        };
        let Some(card) = self.model.card(&id) else {
            return;
        };
        if card.kind != CardKind::Browser {
            return;
        }
        let several_tabs = card.tabs.len() > 1;
        let Some(surface) = self.browser_for(&id).and_then(|b| b.active_surface()) else {
            return;
        };
        if forward {
            surface.forward();
        } else if surface.can_go_back() {
            surface.back();
        } else if several_tabs {
            self.model.browser_tab_close(&id);
            self.perform_effects();
        }
    }

    /// Find in page. A None request ends the search and clears the
    /// highlights, which is what closing the bar must always do.
    pub fn find_effect(&mut self, card_id: &str, request: Option<FindRequest>) {
        let Some(body) = self
            .bodies
            .get_mut(card_id)
            .and_then(|b| b.as_any_mut().downcast_mut::<BrowserBody>())
        else {
            return;
        };
        let Some(surface) = body.active_surface() else {
            return;
        };
        match request {
            Some(r) => surface.find(&r.text, r.forward, r.next),
            None => surface.stop_find(),
        }
    }

    /// The OS cursor follows whichever browser card the pointer is over
    /// (a pointer over a link, an I-beam over an input), the way it would
    /// in a real browser tab. Anything else hovered, or nothing, keeps the
    /// arrow: nothing else in the app sets a cursor style, so this is
    /// decided fresh every frame rather than left to whatever gpui reset
    /// to on its own.
    ///
    /// gpui gives a WINDOW-level cursor precedence over every element's own
    /// (`cursor_pointer` on a row), so asking for the arrow every frame, as
    /// this once did, made every pointer in the palette, the dialogs and the
    /// tabs a no-op (#256). It now asks only when the pointer is over a
    /// browser page and no overlay is up; everywhere else the elements
    /// decide, and gpui's default is the arrow.
    pub fn apply_hover_cursor(&mut self, window: &mut gpui::Window) {
        if self.model.modal_open() {
            return;
        }
        let hover = self.hover_body.clone();
        let style = hover
            .as_ref()
            .and_then(|id| self.browser_for(id))
            .map(|b| b.cursor_style())
            .or_else(|| {
                // An editor's tree divider: the resize cursor (#258).
                hover
                    .as_ref()
                    .and_then(|id| self.editor_tabs_for(id))
                    .and_then(|t| t.divider_cursor())
            });
        if let Some(style) = style {
            window.set_window_cursor_style(style);
        }
    }

    /// Chromium reports a find's progress several times per search; the
    /// model takes whatever arrived by this frame.
    pub fn drain_find(&mut self) {
        let Some(card_id) = self.model.find.card_id.clone() else {
            return;
        };
        let result = self
            .bodies
            .get_mut(&card_id)
            .and_then(|b| b.as_any_mut().downcast_mut::<BrowserBody>())
            .and_then(|b| b.active_surface())
            .map(|s| s.find_result());
        if let Some((matches, active)) = result {
            let before = (self.model.find.matches, self.model.find.active);
            self.model.find_result(&card_id, matches, active);
            self.redraw |= (self.model.find.matches, self.model.find.active) != before;
        }
    }
}

#[cfg(test)]
mod tab_diff_tests {
    use super::*;

    #[test]
    fn added_is_the_new_last_entry() {
        let old = vec!["a".to_string(), "b".to_string()];
        let new = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(diff_added(&old, &new), Some("c".to_string()));
        assert_eq!(diff_removed_index(&old, &new), None);
    }

    #[test]
    fn removed_from_the_middle_is_found_by_its_index() {
        let old = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let new = vec!["a".to_string(), "c".to_string()];
        assert_eq!(diff_removed_index(&old, &new), Some(1));
        assert_eq!(diff_added(&old, &new), None);
    }

    #[test]
    fn removed_from_the_end_is_found_too() {
        let old = vec!["a".to_string(), "b".to_string()];
        let new = vec!["a".to_string()];
        assert_eq!(diff_removed_index(&old, &new), Some(1));
    }

    #[test]
    fn one_step_changes_are_the_one_step_cases() {
        let two = vec!["a".to_string(), "b".to_string()];
        let three = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(tab_diff(&two, &three), TabDiff::Added("c".to_string()));
        assert_eq!(tab_diff(&three, &two), TabDiff::RemovedAt(2));
        assert_eq!(tab_diff(&two, &two), TabDiff::Keep);
    }

    #[test]
    fn a_url_that_changed_in_place_is_not_a_structural_change() {
        // The omnibox navigating the active tab: same tabs, one new
        // address, which `body.navigate` handles a few lines further on.
        let old = vec!["a".to_string(), "b".to_string()];
        let new = vec!["a".to_string(), "z".to_string()];
        assert_eq!(tab_diff(&old, &new), TabDiff::Keep);
    }

    #[test]
    fn more_than_one_step_in_a_frame_rebuilds() {
        // Two popups from one page land in the same frame, so `card.tabs`
        // grows by two before the next reconcile: neither one-step case
        // fits, and doing nothing would strand the body a tab behind for
        // the rest of the run.
        let old = vec!["a".to_string()];
        let new = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(tab_diff(&old, &new), TabDiff::Rebuild);
        assert_eq!(tab_diff(&new, &old), TabDiff::Rebuild);
    }

    #[test]
    fn an_unchanged_or_multiply_changed_list_reports_neither() {
        let old = vec!["a".to_string()];
        let new = vec!["a".to_string()];
        assert_eq!(diff_added(&old, &new), None);
        assert_eq!(diff_removed_index(&old, &new), None);
        let two_more = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(diff_added(&old, &two_more), None);
    }
}
