//! Browser cards and their surfaces: one `BrowserBody` per browser card,
//! opened on the card's url at the card's size, kept in step each frame
//! (new pixels, the page's own zoom from `card.zoom`, a url typed into
//! `browser.navigate`), and what the page did written back: the address
//! it moved to onto the card, popups opened as cards beside it, a
//! right-click turned into the slim menu overlay. CEF itself is started
//! once in `main.rs`; `cef_running` says whether it is.
use crate::browser_body::BrowserBody;
use crate::overlays::OVERLAY_BODY_FONT_PX;
use crate::{now_ms, AppView};
use gpui::prelude::*;
use gpui::{div, px, MouseButton, MouseDownEvent};
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
        let inactive_dim = self.model.config.ui.inactive_dim;
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
        let mut menu: Option<(String, Point, infiniterm_browser::ContextMenuRequest)> = None;
        for card in cards {
            let world = Size {
                w: card.rect.w,
                h: card.rect.h,
            };
            let url = card.url.clone().unwrap_or_else(|| "about:blank".into());
            if self.browser_for(&card.id).is_none() {
                let body = BrowserBody::new(&card.id, &url, world, scale, cef);
                self.bodies.insert(card.id.clone(), Box::new(body));
            }
            let Some(body) = self.browser_for(&card.id) else {
                continue;
            };
            body.card_bg = card_bg;
            body.text = text;
            body.font_family = family.clone();
            body.inactive_dim = inactive_dim;
            body.zoom = card.zoom;
            // `browser.navigate` changed the card's url: the page follows.
            if body.url != url && url != "about:blank" {
                body.navigate(&url);
            }
            if let Some(new_url) = body.sync() {
                moved.push((card.id.clone(), new_url));
            }
            if let Some(title) = body.take_title() {
                titles.push((body.url.clone(), title));
            }
            for popup in std::mem::take(&mut body.popups) {
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
            });
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
            let plan = url_plan(&url, "");
            self.model.open_in_card(plan, Some(&id));
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
            _ => {}
        }
        let Some(surface) = self
            .browser_for(&menu.card_id)
            .and_then(|b| b.surface.as_ref())
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
            ContextMenuAction::CopyLinkAddress | ContextMenuAction::OpenLinkInNewCard => {
                unreachable!("handled above, before the surface borrow")
            }
        }
    }

    /// The slim menu itself: a floating sheet at the click point over a
    /// transparent full-canvas backdrop, so a click anywhere else dismisses
    /// it the way the omnibox's backdrop does. Empty when nothing is open.
    pub fn render_context_menu(&self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let Some(menu) = &self.context_menu else {
            return div();
        };
        let chrome = &self.chrome;
        let ui = self.model.ui_scale as f32;
        let mut items: Vec<(&'static str, ContextMenuAction)> = vec![
            ("Back", ContextMenuAction::Back),
            ("Forward", ContextMenuAction::Forward),
            ("Reload", ContextMenuAction::Reload),
        ];
        if menu.editable {
            items.push(("Cut", ContextMenuAction::Cut));
        }
        if menu.editable || menu.has_selection {
            items.push(("Copy", ContextMenuAction::Copy));
        }
        if menu.editable {
            items.push(("Paste", ContextMenuAction::Paste));
        }
        if menu.link_url.is_some() {
            items.push(("Copy link address", ContextMenuAction::CopyLinkAddress));
            items.push((
                "Open link in new card",
                ContextMenuAction::OpenLinkInNewCard,
            ));
        }
        let mut sheet = div().flex().flex_col().py_1();
        for (label, action) in items {
            sheet = sheet.child(
                div()
                    .id(gpui::SharedString::from(format!("ctxmenu-{label}")))
                    .px_3()
                    .py_1()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.context_menu_choose(action, cx);
                            cx.notify();
                        }),
                    )
                    .child(label),
            );
        }
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.context_menu = None;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(menu.x as f32))
                    .top(px(menu.y as f32))
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
                    // The sheet swallows the click that would otherwise
                    // reach the backdrop and close it.
                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                        cx.stop_propagation()
                    })
                    .child(sheet),
            )
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
        let Some(surface) = &body.surface else { return };
        match action {
            BrowserAction::Back => surface.back(),
            BrowserAction::Forward => surface.forward(),
            BrowserAction::Reload => surface.reload(),
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
        let Some(surface) = &body.surface else { return };
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
    pub fn apply_hover_cursor(&mut self, window: &mut gpui::Window) {
        let style = self
            .hover_body
            .clone()
            .and_then(|id| self.browser_for(&id))
            .map(|b| b.cursor_style())
            .unwrap_or(gpui::CursorStyle::Arrow);
        window.set_window_cursor_style(style);
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
            .and_then(|b| b.surface.as_ref())
            .map(|s| s.find_result());
        if let Some((matches, active)) = result {
            let before = (self.model.find.matches, self.model.find.active);
            self.model.find_result(&card_id, matches, active);
            self.redraw |= (self.model.find.matches, self.model.find.active) != before;
        }
    }
}
