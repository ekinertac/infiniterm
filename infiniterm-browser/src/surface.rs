//! One windowless browser: created on `about:blank` so the moat is in
//! place before the first navigation, painting BGRA frames into a shared
//! slot the ui copies to a texture when it changed, taking the mouse and
//! keys in page pixels. From spikes/canvas/src/browser.rs, plus what the
//! spike left out: a view size that follows the card, a device scale the
//! card can raise when drawn above 100%, popups turned into urls for the
//! ui to open as cards, and the title and address for the label and the
//! save file.
use crate::moat;
use cef::*;
use std::{cell::RefCell, rc::Rc, sync::Arc};

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
}

#[derive(Default)]
pub struct Shared {
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
    pub scale: f32,
    /// The last find's result: how many matches and which one is current.
    /// Chromium reports this several times per search as it walks the page,
    /// so the bar shows whatever arrived last.
    pub find: (i32, i32),
    /// The page's last requested cursor, as a CSS cursor keyword: a pointer
    /// over a link, text over an input. Empty means CEF hasn't said yet, the
    /// same as "default" to whoever reads it.
    pub cursor: &'static str,
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
                let s = self.handler.shared.borrow();
                rect.width = s.width.max(1);
                rect.height = s.height.max(1);
            }
        }

        fn screen_info(&self, _browser: Option<&mut Browser>, info: Option<&mut ScreenInfo>) -> ::std::os::raw::c_int {
            match info {
                Some(info) => {
                    info.device_scale_factor = self.handler.shared.borrow().scale;
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
            shared.frame = Some(Arc::new(Frame { width: width as u32, height: height as u32, bgra }));
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
            _cursor: *mut u8,
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
        fn on_loading_state_change(&self, _browser: Option<&mut Browser>, is_loading: ::std::os::raw::c_int, _can_go_back: ::std::os::raw::c_int, _can_go_forward: ::std::os::raw::c_int) {
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

wrap_client! {
    struct ClientBuilder {
        render_handler: RenderHandler,
        life_span_handler: LifeSpanHandler,
        display_handler: DisplayHandler,
        load_handler: LoadHandler,
        find_handler: FindHandler,
    }

    impl Client {
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
    }
}

pub struct Surface {
    browser: cef::Browser,
    pub shared: Rc<RefCell<Shared>>,
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
            FindBuilder::new(handler),
        );
        let browser = browser_host_create_browser_sync(
            Some(&window_info),
            Some(&mut client),
            Some(&"about:blank".into()),
            Some(&settings),
            None,
            None,
        )?;
        if let Some(host) = browser.host() {
            moat::apply(&host);
        }
        if let Some(frame) = browser.main_frame() {
            frame.load_url(Some(&url.into()));
        }
        Some(Surface { browser, shared })
    }

    fn host(&self) -> Option<BrowserHost> {
        self.browser.host()
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
        let changed = {
            let mut s = self.shared.borrow_mut();
            let changed = s.width != width.max(1) || s.height != height.max(1) || s.scale != scale;
            s.width = width.max(1);
            s.height = height.max(1);
            s.scale = scale;
            changed
        };
        if changed {
            if let Some(host) = self.host() {
                host.notify_screen_info_changed();
                host.was_resized();
            }
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
            flags |= sys::cef_event_flags_t::EVENTFLAG_SHIFT_DOWN.0;
        }
        if mods.control {
            flags |= sys::cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0;
        }
        if mods.alt {
            flags |= sys::cef_event_flags_t::EVENTFLAG_ALT_DOWN.0;
        }
        flags
    }

    fn mouse(x: f32, y: f32, mods: Mods) -> MouseEvent {
        MouseEvent {
            x: x as i32,
            y: y as i32,
            modifiers: Self::flags(mods),
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
            host.send_mouse_click_event(
                Some(&Self::mouse(x, y, mods)),
                b,
                up as i32,
                clicks.max(1) as i32,
            );
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

/// macOS virtual key codes (Carbon `kVK_*`) for the same named keys, the
/// codeset Cocoa's `interpretKeyEvents:` resolves editing commands from.
/// Only meaningful alongside `virtual_key`, which is why an unnamed key (0)
/// never reaches here: `key()` skips this table for the CHAR-only path.
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

    #[test]
    fn backspace_carries_the_real_macos_keycode_not_the_a_key() {
        // Cocoa resolves deleteBackward: from the native code; left at the
        // default (0, kVK_ANSI_A) the key silently did nothing in a field.
        assert_eq!(native_key_code("backspace"), 0x33);
        assert_eq!(native_key_code("left"), 0x7B);
        assert_eq!(native_key_code("home"), 0x73);
        assert_eq!(native_key_code("a"), 0);
    }

    #[test]
    fn cursor_name_covers_a_link_and_an_input_and_falls_back_for_the_rest() {
        assert_eq!(cursor_name(CursorType::HAND), "pointer");
        assert_eq!(cursor_name(CursorType::IBEAM), "text");
        assert_eq!(cursor_name(CursorType::POINTER), "default");
        // gpui has no spinner cursor: WAIT falls back rather than faking one.
        assert_eq!(cursor_name(CursorType::WAIT), "default");
    }
}
