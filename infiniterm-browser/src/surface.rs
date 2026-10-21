//! One windowless browser: created on `about:blank` so the moat is in
//! place before the first navigation, painting BGRA frames into a shared
//! slot the ui copies to a texture when it changed, taking the mouse and
//! keys in page pixels. From spikes/canvas/src/browser.rs, plus what the
//! spike left out: a view size that follows the card, a device scale the
//! card can raise when drawn above 100%, popups turned into urls for the
//! ui to open as cards, and the title and address for the label and the
//! save file.
// CEF's flag enums wrap a `u32` on macOS and an `i32` on Windows, and the
// casts to `u32` below are what make one source line right on both. On the
// Mac they are the same type, which clippy calls unnecessary.
#![allow(clippy::unnecessary_cast)]
use crate::moat;
use cef::*;

/// CEF's native cursor handle, which is `HCURSOR` on Windows and an opaque
/// pointer elsewhere. Nothing here reads it — the cursor is chosen from
/// `type_` — but a trait method's signature has to match the platform's.
#[cfg(windows)]
type CursorHandle = *mut sys::HICON__;
#[cfg(not(windows))]
type CursorHandle = *mut u8;
use std::{cell::RefCell, rc::Rc, sync::Arc};

/// `[browser]` lines on stderr under INFINITERM_KEYLOG: the size handshake with
/// CEF (what the card asked for, what CEF asked back, what it painted), which
/// is how #479's "sometimes it works, sometimes not" was found. Read once.
/// How many of CEF's recent size and scale asks are kept to explain a frame.
const ASKED_KEPT: usize = 8;

pub fn keylog() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("INFINITERM_KEYLOG").is_some())
}

/// A scale below this makes CEF's layout math degenerate; a card zoomed
/// far out still asks for a real, paintable surface.
const MIN_DEVICE_SCALE: f32 = 0.5;
/// CEF's windowless renderer paints at this rate regardless of the app's
/// own frame pacing.
const TARGET_FPS: i32 = 60;
/// Chromium's zoom levels are powers of this base: level = log(factor) /
/// log(ZOOM_LEVEL_BASE).
const ZOOM_LEVEL_BASE: f64 = 1.2;
/// Windows virtual-key codes for space and enter, needed twice: once in
/// the RAWKEYDOWN/KEYUP table below, once to decide whether a CHAR event
/// follows it (CEF's text fields want both for these two keys).
const VK_SPACE: i32 = 0x20;
const VK_ENTER: i32 = 0x0D;

/// One painted frame: BGRA, `width * height * 4` bytes.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    /// The size CEF laid the page out at (CSS pixels) when it painted this
    /// frame: the last `view_rect` it asked for, NOT what the card wants
    /// now. The pixels are that size times the device scale, so the ui can
    /// draw an old frame at its natural size while a resize is still being
    /// repainted instead of stretching it to the new card (#479).
    pub css_width: i32,
    pub css_height: i32,
}

/// A right-click, in page pixels (the same space `mouse_down` takes), and
/// enough of what it landed on for a slim menu: the ui decides what to show
/// and how, CEF's own model and native menu are never used.
#[derive(Clone, Debug)]
pub struct ContextMenuRequest {
    pub x: i32,
    pub y: i32,
    pub link_url: Option<String>,
    pub editable: bool,
    pub has_selection: bool,
    /// The page's history, so the menu offers Back and Forward only when
    /// there is somewhere to go.
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// The page's own address, for "open in the system browser".
    pub page_url: String,
}

#[derive(Default)]
pub struct Shared {
    /// The page's DevTools target id, asked for after each load (see
    /// `TargetIdBuilder`). None until the first load ends.
    pub target_id: Option<String>,
    pub frame: Option<Arc<Frame>>,
    /// A frame arrived since the ui last took one.
    pub dirty: bool,
    pub paints: u32,
    /// Windows the page or the extension tried to open, as urls.
    pub popups: Vec<String>,
    pub title: Option<String>,
    pub url: Option<String>,
    pub loading: bool,
    /// The size the page lays out at, in CSS pixels.
    pub width: i32,
    pub height: i32,
    /// The size CEF last ASKED for in `view_rect` (0 before the first ask):
    /// what the next painted frame is laid out at. It trails `width` and
    /// `height` by one round trip after a resize, which is the whole point.
    pub reported_width: i32,
    pub reported_height: i32,
    /// The device scale CEF last ASKED for in `screen_info` (0 before the
    /// first ask), the scale of the frames it paints from then on.
    pub reported_scale: f32,
    /// The last few sizes (CSS pixels) and device scales CEF asked for, newest
    /// last: a frame is laid out at one pair of them, whichever its own pixels
    /// prove (`frame_fit::frame_layout_size`), not at the newest ask (#479).
    pub asked: Vec<(i32, i32)>,
    pub asked_scales: Vec<f32>,
    /// The (width, height, scale) last DELIVERED to CEF with `was_resized`: a
    /// size is only "sent" once the browser host existed to take it. Compared
    /// against, instead of what the card last wanted, so a size asked for before
    /// the host was there is not forgotten (#479).
    pub sent: (i32, i32, f32),
    pub scale: f32,
    /// The last find's result: how many matches and which one is current.
    /// Chromium reports this several times per search as it walks the page,
    /// so the bar shows whatever arrived last.
    pub find: (i32, i32),
    /// The page's last requested cursor, as a CSS cursor keyword: a pointer
    /// over a link, text over an input. Empty means CEF hasn't said yet, the
    /// same as "default" to whoever reads it.
    pub cursor: &'static str,
    /// A right-click since the ui last took one.
    pub context_menu: Option<ContextMenuRequest>,
}

#[derive(Clone)]
struct Handler {
    shared: Rc<RefCell<Shared>>,
}

wrap_render_handler! {
    struct RenderHandlerBuilder {
        handler: Handler,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(rect) = rect {
                let mut s = self.handler.shared.borrow_mut();
                rect.width = s.width.max(1);
                rect.height = s.height.max(1);
                // What the frame CEF paints next is laid out at (see `Frame`).
                s.reported_width = rect.width;
                s.reported_height = rect.height;
                let ask = (rect.width, rect.height);
                if s.asked.last() != Some(&ask) {
                    s.asked.push(ask);
                    if s.asked.len() > ASKED_KEPT {
                        s.asked.remove(0);
                    }
                }
                if keylog() {
                    eprintln!("[browser] view_rect asked -> {}x{}", rect.width, rect.height);
                }
            }
        }

        fn screen_info(&self, _browser: Option<&mut Browser>, info: Option<&mut ScreenInfo>) -> ::std::os::raw::c_int {
            match info {
                Some(info) => {
                    let mut s = self.handler.shared.borrow_mut();
                    info.device_scale_factor = s.scale;
                    s.reported_scale = s.scale;
                    let scale = s.scale;
                    if s.asked_scales.last() != Some(&scale) {
                        s.asked_scales.push(scale);
                        if s.asked_scales.len() > ASKED_KEPT {
                            s.asked_scales.remove(0);
                        }
                    }
                    if keylog() {
                        eprintln!("[browser] screen_info asked -> scale {}", s.scale);
                    }
                    1
                }
                None => 0,
            }
        }

        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            if type_ != PaintElementType::VIEW || buffer.is_null() || width <= 0 || height <= 0 {
                return;
            }
            let len = (width * height * 4) as usize;
            let bgra = unsafe { std::slice::from_raw_parts(buffer, len) }.to_vec();
            let mut shared = self.handler.shared.borrow_mut();
            // The layout size the pixels prove, not the newest size CEF asked
            // for (see `frame_fit::frame_layout_size`: the race of #479).
            // Before CEF has asked for anything, the requested size and scale
            // are the best answer there is.
            let fallback_scale = if shared.reported_scale > 0. { shared.reported_scale } else { shared.scale };
            let (css_width, css_height) = if shared.asked.is_empty() {
                (shared.width, shared.height)
            } else {
                crate::frame_fit::frame_layout_size(
                    width as u32,
                    height as u32,
                    &shared.asked,
                    &shared.asked_scales,
                    fallback_scale,
                )
            };
            if keylog() {
                eprintln!(
                    "[browser] on_paint {width}x{height}px tagged css {css_width}x{css_height} (requested {}x{} @{}, reported scale {})",
                    shared.width, shared.height, shared.scale, shared.reported_scale
                );
            }
            shared.frame = Some(Arc::new(Frame {
                width: width as u32,
                height: height as u32,
                bgra,
                css_width,
                css_height,
            }));
            shared.dirty = true;
            shared.paints += 1;
        }
    }
}

wrap_life_span_handler! {
    struct LifeSpanBuilder {
        handler: Handler,
    }

    impl LifeSpanHandler {
        // A popup (window.open, a target=_blank link, the extension's
        // chrome.windows.create) becomes a card, never a native window:
        // the url is queued for the ui and the popup refused.
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefStringUtf16>,
            _target_frame_name: Option<&CefStringUtf16>,
            _target_disposition: WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            if let Some(url) = target_url {
                let url = url.to_string();
                if !url.is_empty() {
                    self.handler.shared.borrow_mut().popups.push(url);
                }
            }
            1
        }
    }
}

wrap_display_handler! {
    struct DisplayBuilder {
        handler: Handler,
    }

    impl DisplayHandler {
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefStringUtf16>) {
            let t = title.map(|t| t.to_string()).filter(|t| !t.is_empty());
            self.handler.shared.borrow_mut().title = t;
        }

        fn on_address_change(&self, _browser: Option<&mut Browser>, frame: Option<&mut cef::Frame>, url: Option<&CefStringUtf16>) {
            if frame.is_some_and(|f| f.is_main() == 1) {
                let u = url.map(|u| u.to_string()).filter(|u| !u.is_empty());
                self.handler.shared.borrow_mut().url = u;
            }
        }

        // Windowless CEF has no native cursor tracking of its own: without
        // this the OS cursor never changes over a link or a text field.
        fn on_cursor_change(
            &self,
            _browser: Option<&mut Browser>,
            _cursor: CursorHandle,
            type_: CursorType,
            _custom_cursor_info: Option<&CursorInfo>,
        ) -> ::std::os::raw::c_int {
            self.handler.shared.borrow_mut().cursor = cursor_name(type_);
            1
        }
    }
}

/// CEF's cursor type as a CSS cursor keyword, the vocabulary the ui maps to
/// a gpui `CursorStyle`. A cursor gpui has no equivalent for (the OS-drawn
/// spinner behind `WAIT`/`PROGRESS`, `HELP`) falls back to the default
/// arrow rather than faking one.
fn cursor_name(t: CursorType) -> &'static str {
    if t == CursorType::HAND {
        "pointer"
    } else if t == CursorType::IBEAM {
        "text"
    } else if t == CursorType::CROSS {
        "crosshair"
    } else if t == CursorType::GRAB {
        "grab"
    } else if t == CursorType::GRABBING {
        "grabbing"
    } else if t == CursorType::EASTRESIZE {
        "e-resize"
    } else if t == CursorType::WESTRESIZE {
        "w-resize"
    } else if t == CursorType::NORTHRESIZE {
        "n-resize"
    } else if t == CursorType::SOUTHRESIZE {
        "s-resize"
    } else if t == CursorType::NORTHSOUTHRESIZE {
        "ns-resize"
    } else if t == CursorType::EASTWESTRESIZE {
        "ew-resize"
    } else if t == CursorType::NORTHEASTSOUTHWESTRESIZE {
        "nesw-resize"
    } else if t == CursorType::NORTHWESTSOUTHEASTRESIZE {
        "nwse-resize"
    } else if t == CursorType::COLUMNRESIZE {
        "col-resize"
    } else if t == CursorType::ROWRESIZE {
        "row-resize"
    } else if t == CursorType::NOTALLOWED || t == CursorType::NODROP {
        "not-allowed"
    } else if t == CursorType::COPY {
        "copy"
    } else if t == CursorType::ALIAS {
        "alias"
    } else if t == CursorType::CONTEXTMENU {
        "context-menu"
    } else if t == CursorType::VERTICALTEXT {
        "vertical-text"
    } else if t == CursorType::NONE {
        "none"
    } else {
        "default"
    }
}

wrap_load_handler! {
    struct LoadBuilder {
        handler: Handler,
    }

    impl LoadHandler {
        fn on_loading_state_change(&self, browser: Option<&mut Browser>, is_loading: ::std::os::raw::c_int, _can_go_back: ::std::os::raw::c_int, _can_go_forward: ::std::os::raw::c_int) {
            // Ask again once a load ends: a page that moved to another site
            // can come back with another main frame, and the DevTools
            // command needs the id the page has now.
            if is_loading == 0 {
                if let Some(host) = browser.and_then(|b| b.host()) {
                    host.execute_dev_tools_method(TARGET_QUERY_ID, Some(&"Page.getFrameTree".into()), None);
                }
            }
            self.handler.shared.borrow_mut().loading = is_loading == 1;
        }
    }
}

wrap_find_handler! {
    struct FindBuilder {
        handler: Handler,
    }

    impl FindHandler {
        fn on_find_result(
            &self,
            _browser: Option<&mut Browser>,
            _identifier: ::std::os::raw::c_int,
            count: ::std::os::raw::c_int,
            _selection_rect: Option<&cef::Rect>,
            active_match_ordinal: ::std::os::raw::c_int,
            _final_update: ::std::os::raw::c_int,
        ) {
            if std::env::var_os("INFINITERM_KEYLOG").is_some() {
                eprintln!("[find] count={count} active={active_match_ordinal}");
            }
            self.handler.shared.borrow_mut().find = (count, active_match_ordinal);
        }
    }
}

wrap_context_menu_handler! {
    struct ContextMenuBuilder {
        handler: Handler,
    }

    impl ContextMenuHandler {
        // A slim menu is ours to draw, never Chromium's: `params` is enough
        // to decide what a card's overlay should offer (a link, an editable
        // field, a selection), and `cancel` on the callback is how CEF is
        // told no command was chosen through ITS model, since one never
        // will be. Chromium's own default menu (OSR cannot show it: there
        // is no native window to anchor one to) is never asked for.
        fn run_context_menu(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            params: Option<&mut ContextMenuParams>,
            _model: Option<&mut MenuModel>,
            callback: Option<&mut RunContextMenuCallback>,
        ) -> ::std::os::raw::c_int {
            if let Some(params) = params {
                let link = CefStringUtf16::from(&params.link_url()).to_string();
                // i32 on Windows, u32 on macOS; `has_flag` takes one width.
                let flags = params.type_flags().as_ref().0 as u32;
                self.handler.shared.borrow_mut().context_menu = Some(ContextMenuRequest {
                    x: params.xcoord(),
                    y: params.ycoord(),
                    link_url: (!link.is_empty()).then_some(link),
                    editable: params.is_editable() != 0,
                    has_selection: has_flag(
                        flags,
                        sys::cef_context_menu_type_flags_t::CM_TYPEFLAG_SELECTION.0 as u32,
                    ),
                    can_go_back: browser.as_ref().is_some_and(|b| b.can_go_back() == 1),
                    can_go_forward: browser.as_ref().is_some_and(|b| b.can_go_forward() == 1),
                    page_url: CefStringUtf16::from(&params.page_url()).to_string(),
                });
            }
            if let Some(callback) = callback {
                callback.cancel();
            }
            1
        }
    }
}

/// Whether `bit` is set in a CEF bitmask (`ContextMenuTypeFlags` and its
/// kin wrap a raw `u32` with no `Eq` against a single flag).
fn has_flag(flags: u32, bit: u32) -> bool {
    flags & bit != 0
}

wrap_request_handler! {
    struct RequestBuilder {
        handler: Handler,
    }

    impl RequestHandler {
        // A middle-click or Cmd+click on a link reaches the browser process
        // here, not through `on_before_popup`: with no answer Chromium
        // follows the link in the tab it came from. The address is queued
        // for the ui as a popup is, and the navigation refused.
        fn on_open_urlfrom_tab(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            target_url: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            if target_disposition == sys::cef_window_open_disposition_t::CEF_WOD_CURRENT_TAB.into() {
                return 0;
            }
            if let Some(url) = target_url {
                let url = url.to_string();
                if !url.is_empty() {
                    self.handler.shared.borrow_mut().popups.push(url);
                }
            }
            1
        }
    }
}

/// The message id of our one DevTools question, `Page.getFrameTree`: its
/// answer carries the main frame's id, which is the page's target id on the
/// debugging port. CEF's own frame identifier is a different value.
const TARGET_QUERY_ID: i32 = 7001;

wrap_dev_tools_message_observer! {
    struct TargetIdBuilder {
        handler: Handler,
    }

    impl DevToolsMessageObserver {
        fn on_dev_tools_method_result(
            &self,
            _browser: Option<&mut Browser>,
            message_id: ::std::os::raw::c_int,
            success: ::std::os::raw::c_int,
            result: Option<&[u8]>,
        ) {
            if message_id != TARGET_QUERY_ID || success != 1 {
                return;
            }
            let Some(text) = result.and_then(|r| std::str::from_utf8(r).ok()) else {
                return;
            };
            if let Some(id) = infiniterm_core::devtools::target_id_from_frame_tree(text) {
                self.handler.shared.borrow_mut().target_id = Some(id);
            }
        }
    }
}

wrap_client! {
    struct ClientBuilder {
        render_handler: RenderHandler,
        life_span_handler: LifeSpanHandler,
        display_handler: DisplayHandler,
        load_handler: LoadHandler,
        find_handler: FindHandler,
        context_menu_handler: ContextMenuHandler,
        request_handler: RequestHandler,
    }

    impl Client {
        fn request_handler(&self) -> Option<RequestHandler> {
            Some(self.request_handler.clone())
        }
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(self.render_handler.clone())
        }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(self.life_span_handler.clone())
        }
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(self.display_handler.clone())
        }
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(self.load_handler.clone())
        }
        fn find_handler(&self) -> Option<FindHandler> {
            Some(self.find_handler.clone())
        }
        fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
            Some(self.context_menu_handler.clone())
        }
    }
}

pub struct Surface {
    browser: cef::Browser,
    pub shared: Rc<RefCell<Shared>>,
    /// Keeps the target id observer registered for as long as the surface.
    _target_id_observer: Option<Registration>,
}

impl Surface {
    /// A browser laid out at `width` x `height` CSS pixels, painted at
    /// `scale`, on `about:blank` with the moat applied, then `url`.
    pub fn open(url: &str, width: i32, height: i32, scale: f32) -> Option<Surface> {
        let shared = Rc::new(RefCell::new(Shared {
            width: width.max(1),
            height: height.max(1),
            scale: scale.max(MIN_DEVICE_SCALE),
            ..Default::default()
        }));
        let handler = Handler {
            shared: shared.clone(),
        };
        let handler_for_observer = handler.clone();
        let window_info = WindowInfo {
            windowless_rendering_enabled: 1,
            ..Default::default()
        };
        let settings = BrowserSettings {
            windowless_frame_rate: TARGET_FPS,
            ..Default::default()
        };
        let mut client = ClientBuilder::new(
            RenderHandlerBuilder::new(handler.clone()),
            LifeSpanBuilder::new(handler.clone()),
            DisplayBuilder::new(handler.clone()),
            LoadBuilder::new(handler.clone()),
            FindBuilder::new(handler.clone()),
            ContextMenuBuilder::new(handler.clone()),
            RequestBuilder::new(handler),
        );
        let browser = browser_host_create_browser_sync(
            Some(&window_info),
            Some(&mut client),
            Some(&"about:blank".into()),
            Some(&settings),
            None,
            None,
        )?;
        let mut target_id_observer = None;
        if let Some(host) = browser.host() {
            moat::apply(&host);
            let mut observer = TargetIdBuilder::new(handler_for_observer);
            target_id_observer = host.add_dev_tools_message_observer(Some(&mut observer));
        }
        if let Some(frame) = browser.main_frame() {
            frame.load_url(Some(&url.into()));
        }
        Some(Surface {
            browser,
            shared,
            _target_id_observer: target_id_observer,
        })
    }

    fn host(&self) -> Option<BrowserHost> {
        self.browser.host()
    }

    /// The page's id on the debugging port, once a load has ended and CEF
    /// has answered; the DevTools command builds its address from it.
    pub fn target_id(&self) -> Option<String> {
        self.shared.borrow().target_id.clone()
    }

    pub fn navigate(&self, url: &str) {
        if let Some(frame) = self.browser.main_frame() {
            frame.load_url(Some(&url.into()));
        }
    }

    pub fn back(&self) {
        self.browser.go_back();
    }

    pub fn forward(&self) {
        self.browser.go_forward();
    }

    /// Whether the page has history behind it: a tab a link opened has none,
    /// and Back there means "close this tab".
    pub fn can_go_back(&self) -> bool {
        self.browser.can_go_back() == 1
    }

    pub fn reload(&self) {
        self.browser.reload();
    }

    /// Find in page. `next` false starts a new search (Chromium highlights
    /// everything and selects the first hit); true steps to the next or
    /// previous hit of the search already running.
    pub fn find(&self, text: &str, forward: bool, next: bool) {
        if let Some(host) = self.host() {
            host.find(Some(&text.into()), forward as i32, 0, next as i32);
        }
    }

    /// Ends the search and drops every highlight. Always called when the
    /// bar closes: highlights that outlive the bar are litter nobody can
    /// clear.
    pub fn stop_find(&self) {
        if let Some(host) = self.host() {
            host.stop_finding(1);
        }
        self.shared.borrow_mut().find = (0, 0);
    }

    /// (matches, which one is current). Zero matches means no hits, which
    /// is the only way the bar can say so.
    pub fn find_result(&self) -> (i32, i32) {
        self.shared.borrow().find
    }

    /// The layout size or the device scale changed: tell CEF, which asks
    /// `view_rect` and `screen_info` again and repaints.
    pub fn resize(&self, width: i32, height: i32, scale: f32) {
        let (w, h) = (width.max(1), height.max(1));
        let changed = {
            let mut s = self.shared.borrow_mut();
            s.width = w;
            s.height = h;
            s.scale = scale;
            // Against what CEF was last TOLD, not what the card last wanted:
            // a size set while the browser host did not exist yet was never
            // delivered, and comparing with the wanted size would call it
            // unchanged from then on (#479).
            s.sent != (w, h, scale)
        };
        if changed {
            if keylog() {
                eprintln!("[browser] resize -> css {w}x{h} @{scale}");
            }
            if let Some(host) = self.host() {
                host.notify_screen_info_changed();
                host.was_resized();
                self.shared.borrow_mut().sent = (w, h, scale);
            }
        }
    }

    /// Ask CEF to paint again at the size the card wants now, when the newest
    /// frame is of an older one and nothing more is coming (`frame_fit::needs_kick`,
    /// #479): the size and scale are told again and the whole view invalidated.
    pub fn kick(&self) {
        if let Some(host) = self.host() {
            if keylog() {
                let s = self.shared.borrow();
                eprintln!(
                    "[browser] kick: repaint at css {}x{} @{}",
                    s.width, s.height, s.scale
                );
            }
            host.notify_screen_info_changed();
            host.was_resized();
            host.invalidate(PaintElementType::VIEW);
        }
    }

    /// A frame is waiting since the last take.
    pub fn has_new_frame(&self) -> bool {
        self.shared.borrow().dirty
    }

    /// Takes the newest frame if one arrived since the last take.
    pub fn take_frame(&self) -> Option<Arc<Frame>> {
        let mut s = self.shared.borrow_mut();
        if !s.dirty {
            return None;
        }
        s.dirty = false;
        s.frame.clone()
    }

    pub fn take_popups(&self) -> Vec<String> {
        std::mem::take(&mut self.shared.borrow_mut().popups)
    }

    /// A right-click since the last call, if one landed.
    pub fn take_context_menu(&self) -> Option<ContextMenuRequest> {
        self.shared.borrow_mut().context_menu.take()
    }

    pub fn title(&self) -> Option<String> {
        self.shared.borrow().title.clone()
    }

    pub fn url(&self) -> Option<String> {
        self.shared.borrow().url.clone()
    }

    /// The page's last requested cursor, as a CSS keyword ("pointer",
    /// "text", ...), or "default" before CEF has said anything.
    pub fn cursor(&self) -> &'static str {
        let c = self.shared.borrow().cursor;
        if c.is_empty() {
            "default"
        } else {
            c
        }
    }

    pub fn close(&self) {
        if let Some(host) = self.host() {
            host.close_browser(1);
        }
    }

    fn flags(mods: Mods) -> u32 {
        let mut flags = 0u32;
        if mods.shift {
            flags |= sys::cef_event_flags_t::EVENTFLAG_SHIFT_DOWN.0 as u32;
        }
        if mods.control {
            flags |= sys::cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0 as u32;
        }
        if mods.alt {
            flags |= sys::cef_event_flags_t::EVENTFLAG_ALT_DOWN.0 as u32;
        }
        flags
    }

    fn mouse(x: f32, y: f32, mods: Mods) -> MouseEvent {
        // Cmd is for the mouse only: a key sent with it would reach the page
        // as a shortcut the app already took.
        let command = if mods.command {
            sys::cef_event_flags_t::EVENTFLAG_COMMAND_DOWN.0 as u32
        } else {
            0
        };
        MouseEvent {
            x: x as i32,
            y: y as i32,
            modifiers: Self::flags(mods) | command,
        }
    }

    /// `x`, `y` in page pixels: the caller has already undone the zoom. The
    /// second argument to `send_mouse_move_event` is CEF's `mouseLeave`, not
    /// a button flag: a move within the card is never a leave.
    pub fn mouse_move(&self, x: f32, y: f32, mods: Mods) {
        if let Some(host) = self.host() {
            host.send_mouse_move_event(Some(&Self::mouse(x, y, mods)), 0);
        }
    }

    /// The pointer left the card for another card or empty canvas: tells
    /// CEF the way a real mouseout would, so `:hover` and tooltips clear
    /// instead of sticking to whatever was last under the cursor.
    pub fn mouse_leave(&self) {
        if let Some(host) = self.host() {
            host.send_mouse_move_event(Some(&Self::mouse(0., 0., Mods::default())), 1);
        }
    }

    pub fn mouse_button(
        &self,
        x: f32,
        y: f32,
        mods: Mods,
        button: Button,
        up: bool,
        clicks: usize,
    ) {
        if let Some(host) = self.host() {
            if !up {
                host.set_focus(1);
            }
            let b = match button {
                Button::Left => MouseButtonType::LEFT,
                Button::Middle => MouseButtonType::MIDDLE,
                Button::Right => MouseButtonType::RIGHT,
            };
            let mut event = Self::mouse(x, y, mods);
            // Chromium reads which button is down from the flags as well as
            // from the event's button: without this a middle press on a link
            // was a plain click.
            if !up && button == Button::Middle {
                event.modifiers |= sys::cef_event_flags_t::EVENTFLAG_MIDDLE_MOUSE_BUTTON.0 as u32;
            }
            if std::env::var_os("INFINITERM_KEYLOG").is_some() {
                eprintln!(
                    "[mouse] {button:?} up={up} flags={:#x} at {},{}",
                    event.modifiers, event.x, event.y
                );
            }
            host.send_mouse_click_event(Some(&event), b, up as i32, clicks.max(1) as i32);
        }
    }

    pub fn wheel(&self, x: f32, y: f32, mods: Mods, dx: f32, dy: f32) {
        if let Some(host) = self.host() {
            host.send_mouse_wheel_event(Some(&Self::mouse(x, y, mods)), dx as i32, dy as i32);
        }
    }

    pub fn focus(&self, on: bool) {
        if let Some(host) = self.host() {
            host.set_focus(on as i32);
        }
    }

    /// Cmd chords a page owns: `shift` splits Cmd+Z (undo) from
    /// Cmd+Shift+Z (redo). Returns whether it took the key.
    pub fn edit_chord(&self, key: &str, shift: bool) -> bool {
        let Some(frame) = self.browser.main_frame() else {
            return false;
        };
        match key {
            "v" => frame.paste(),
            "c" => frame.copy(),
            "x" => frame.cut(),
            "a" => frame.select_all(),
            "z" if shift => frame.redo(),
            "z" => frame.undo(),
            _ => return false,
        }
        true
    }

    /// A key by gpui's name and character: named keys go as RAWKEYDOWN /
    /// KEYUP with Windows virtual key codes, printable ones as CHAR
    /// events, which is what a page's handlers and text fields expect.
    /// `native_key_code` rides along on the named-key path: macOS resolves
    /// editing commands (delete, cursor movement, select-word) through
    /// Cocoa's key-binding tables, keyed on the real macOS keycode rather
    /// than the Windows one, and a synthetic event that leaves it at 0 (the
    /// `A` key) resolves to nothing, which is why Backspace and the arrows
    /// silently did not work in a text field.
    pub fn key(&self, name: &str, text: Option<&str>, mods: Mods) {
        let Some(host) = self.host() else { return };
        let modifiers = Self::flags(mods);
        let vk = virtual_key(name);
        if vk != 0 {
            let native_key_code = native_key_code(name);
            for type_ in [KeyEventType::RAWKEYDOWN, KeyEventType::KEYUP] {
                host.send_key_event(Some(&KeyEvent {
                    type_,
                    modifiers,
                    windows_key_code: vk,
                    native_key_code,
                    ..Default::default()
                }));
            }
            if vk == VK_SPACE || vk == VK_ENTER {
                let ch = if vk == VK_SPACE { b' ' } else { b'\r' } as u16;
                host.send_key_event(Some(&KeyEvent {
                    type_: KeyEventType::CHAR,
                    modifiers,
                    windows_key_code: vk,
                    native_key_code,
                    character: ch,
                    unmodified_character: ch,
                    ..Default::default()
                }));
            }
            return;
        }
        let Some(text) = text else { return };
        for ch in text.encode_utf16() {
            host.send_key_event(Some(&KeyEvent {
                type_: KeyEventType::CHAR,
                modifiers,
                windows_key_code: ch as i32,
                character: ch,
                unmodified_character: ch,
                ..Default::default()
            }));
        }
    }

    /// The page's own zoom as a factor (1 is 100%).
    pub fn set_zoom(&self, factor: f64) {
        if let Some(host) = self.host() {
            host.set_zoom_level(factor.ln() / ZOOM_LEVEL_BASE.ln());
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    /// Cmd: with it Chromium opens a clicked link in a new tab.
    pub command: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

/// Windows virtual key codes for the keys a page handles by name.
pub fn virtual_key(name: &str) -> i32 {
    match name {
        "enter" => VK_ENTER,
        "backspace" => 0x08,
        "tab" => 0x09,
        "escape" => 0x1B,
        "space" => VK_SPACE,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "end" => 0x23,
        "home" => 0x24,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "delete" => 0x2E,
        "f1" => 0x70,
        "f2" => 0x71,
        "f3" => 0x72,
        "f4" => 0x73,
        "f5" => 0x74,
        "f6" => 0x75,
        "f7" => 0x76,
        "f8" => 0x77,
        "f9" => 0x78,
        "f10" => 0x79,
        "f11" => 0x7A,
        "f12" => 0x7B,
        _ => 0,
    }
}

/// The PHYSICAL key, in whatever codeset this platform's CEF reads it in.
///
/// Two different jobs behind one name, which is why there are two tables.
/// On macOS it is the Carbon `kVK_*` code and Cocoa resolves the editing
/// commands from it, so Backspace and the arrows do nothing in a text field
/// without it. On Windows it is the scan code, and Chromium turns it into
/// the `code` a page's key handler reads: left at the macOS value, a
/// Backspace arrived at the page as `code: "Comma"`, because 0x33 is the
/// comma key in scan code set 1 (measured in a card, 2026-09-25). The value
/// is wrong in a quiet way on both platforms — the key still works, a page
/// that reads `event.code` just hears a different key — so it is worth a
/// test each rather than trusting the tables to look right.
///
/// Only meaningful alongside `virtual_key`, which is why an unnamed key (0)
/// never reaches here: `key()` skips this table for the CHAR-only path.
#[cfg(windows)]
fn native_key_code(name: &str) -> i32 {
    match name {
        "enter" => 0x1C,
        "backspace" => 0x0E,
        "tab" => 0x0F,
        "escape" => 0x01,
        "space" => 0x39,
        // The grey navigation block and the arrows are the extended keys:
        // the same scan codes as the numeric keypad, told apart by the E0
        // prefix. Chromium wants that prefix as bit 8 of the scan code
        // (`0xE0xx` in its own tables), not as the lParam's extended bit,
        // which a synthetic event has no room for.
        "pageup" => 0xE049,
        "pagedown" => 0xE051,
        "end" => 0xE04F,
        "home" => 0xE047,
        "left" => 0xE04B,
        "up" => 0xE048,
        "right" => 0xE04D,
        "down" => 0xE050,
        "delete" => 0xE053,
        "f1" => 0x3B,
        "f2" => 0x3C,
        "f3" => 0x3D,
        "f4" => 0x3E,
        "f5" => 0x3F,
        "f6" => 0x40,
        "f7" => 0x41,
        "f8" => 0x42,
        "f9" => 0x43,
        "f10" => 0x44,
        "f11" => 0x57,
        "f12" => 0x58,
        _ => 0,
    }
}

#[cfg(not(windows))]
fn native_key_code(name: &str) -> i32 {
    match name {
        "enter" => 0x24,
        "backspace" => 0x33,
        "tab" => 0x30,
        "escape" => 0x35,
        "space" => 0x31,
        "pageup" => 0x74,
        "pagedown" => 0x79,
        "end" => 0x77,
        "home" => 0x73,
        "left" => 0x7B,
        "up" => 0x7E,
        "right" => 0x7C,
        "down" => 0x7D,
        "delete" => 0x75,
        "f1" => 0x7A,
        "f2" => 0x78,
        "f3" => 0x63,
        "f4" => 0x76,
        "f5" => 0x60,
        "f6" => 0x61,
        "f7" => 0x62,
        "f8" => 0x64,
        "f9" => 0x65,
        "f10" => 0x6D,
        "f11" => 0x67,
        "f12" => 0x6F,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys_have_codes_and_letters_do_not() {
        assert_eq!(virtual_key("enter"), 0x0D);
        assert_eq!(virtual_key("f5"), 0x74);
        assert_eq!(virtual_key("a"), 0);
    }

    #[cfg(not(windows))]
    #[test]
    fn backspace_carries_the_real_macos_keycode_not_the_a_key() {
        // Cocoa resolves deleteBackward: from the native code; left at the
        // default (0, kVK_ANSI_A) the key silently did nothing in a field.
        assert_eq!(native_key_code("backspace"), 0x33);
        assert_eq!(native_key_code("left"), 0x7B);
        assert_eq!(native_key_code("home"), 0x73);
        assert_eq!(native_key_code("a"), 0);
    }

    // The Windows table is scan codes, and the macOS one read as scan codes
    // is what sent `code: "Comma"` to a page for Backspace. The three that
    // would collide if the wrong table were compiled in are the check.
    #[cfg(windows)]
    #[test]
    fn backspace_carries_the_scan_code_a_page_reads_as_backspace() {
        assert_eq!(native_key_code("backspace"), 0x0E);
        assert_eq!(native_key_code("escape"), 0x01);
        assert_eq!(native_key_code("a"), 0);
    }

    // An arrow shares its scan code with the keypad key printed beside it;
    // without the E0 prefix a page hears Numpad4 for Left.
    #[cfg(windows)]
    #[test]
    fn the_navigation_keys_carry_their_extended_prefix() {
        for name in [
            "left", "up", "right", "down", "home", "end", "pageup", "pagedown", "delete",
        ] {
            let code = native_key_code(name);
            assert_eq!(code & 0xFF00, 0xE000, "{name} is missing its E0 prefix");
        }
        // And the ones that are not extended do not carry it.
        for name in ["enter", "backspace", "tab", "escape", "space", "f1", "f12"] {
            assert_eq!(
                native_key_code(name) & 0xFF00,
                0,
                "{name} must not be extended"
            );
        }
    }

    #[test]
    fn cursor_name_covers_a_link_and_an_input_and_falls_back_for_the_rest() {
        assert_eq!(cursor_name(CursorType::HAND), "pointer");
        assert_eq!(cursor_name(CursorType::IBEAM), "text");
        assert_eq!(cursor_name(CursorType::POINTER), "default");
        // gpui has no spinner cursor: WAIT falls back rather than faking one.
        assert_eq!(cursor_name(CursorType::WAIT), "default");
    }

    #[test]
    fn has_flag_reads_one_bit_out_of_a_combined_mask() {
        let selection = sys::cef_context_menu_type_flags_t::CM_TYPEFLAG_SELECTION.0 as u32;
        let link = sys::cef_context_menu_type_flags_t::CM_TYPEFLAG_LINK.0 as u32;
        assert!(has_flag(selection | link, selection));
        assert!(!has_flag(link, selection));
    }
}
