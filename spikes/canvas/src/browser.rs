//! A browser card's insides: one CEF windowless browser painting into a
//! shared frame, and the input encoders that send the mouse and keys back.
//!
//! From the cef-frame spike: software `on_paint` into a `RenderImage` (CEF
//! hands BGRA and gpui stores BGRA, so it is a copy), the moat applied
//! before the first navigation, mouse positions handed over in page
//! pixels. `cef_app_protocol` patches gpui's NSApplication with the two
//! methods CEF's message pump calls. The CEF process setup (execute_process,
//! initialize, the pump) stays in main.rs because it is one per process.
use crate::chrome_moat;
use cef::*;
use gpui::RenderImage;
use image::{Frame, RgbaImage};
use smallvec::SmallVec;
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

pub const VIEW_W: i32 = 1024;
pub const VIEW_H: i32 = 768;

/// Where the cef-extension spike keeps the unpacked extension and the
/// signed-in profile; the manifest dir is what `env!` resolves against.
pub fn spike_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../cef-extension")
}

#[derive(Default)]
pub struct Shared {
    pub frame: Option<Arc<RenderImage>>,
    pub dirty: bool,
    pub paints: u32,
}

#[derive(Clone)]
struct Handler {
    shared: Rc<RefCell<Shared>>,
    scale: f32,
}

wrap_render_handler! {
    struct RenderHandlerBuilder {
        handler: Handler,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(rect) = rect {
                rect.width = VIEW_W;
                rect.height = VIEW_H;
            }
        }

        fn screen_info(&self, _browser: Option<&mut Browser>, info: Option<&mut ScreenInfo>) -> ::std::os::raw::c_int {
            match info {
                Some(info) => {
                    info.device_scale_factor = self.handler.scale;
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
            let bytes = unsafe { std::slice::from_raw_parts(buffer, len) }.to_vec();
            let Some(img) = RgbaImage::from_raw(width as u32, height as u32, bytes) else { return };
            let render = RenderImage::new(SmallVec::from_elem(Frame::new(img), 1));
            let mut shared = self.handler.shared.borrow_mut();
            shared.frame = Some(Arc::new(render));
            shared.dirty = true;
            shared.paints += 1;
        }
    }
}

wrap_client! {
    struct ClientBuilder {
        render_handler: RenderHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(self.render_handler.clone())
        }
    }
}

wrap_app! {
    pub struct AppBuilder;

    impl App {
        fn on_before_command_line_processing(&self, _process_type: Option<&CefStringUtf16>, command_line: Option<&mut CommandLine>) {
            let Some(cmd) = command_line else { return };
            cmd.append_switch(Some(&"no-startup-window".into()));
            cmd.append_switch(Some(&"noerrdialogs".into()));
            cmd.append_switch(Some(&"use-mock-keychain".into()));
            let ext = spike_dir().join("extension");
            cmd.append_switch_with_value(Some(&"load-extension".into()), Some(&CefString::from(ext.to_string_lossy().as_ref())));
        }
    }
}

pub struct BrowserCard {
    pub browser: cef::Browser,
    pub shared: Rc<RefCell<Shared>>,
}

impl BrowserCard {
    /// Creates a windowless browser on `about:blank`, applies the moat, then
    /// loads `url`, so the overrides are in place for the first navigation.
    pub fn open(url: &str, scale: f32) -> BrowserCard {
        let shared = Rc::new(RefCell::new(Shared::default()));
        let handler = Handler { shared: shared.clone(), scale };
        let window_info = WindowInfo { windowless_rendering_enabled: 1, ..Default::default() };
        let settings = BrowserSettings { windowless_frame_rate: 60, ..Default::default() };
        let browser = browser_host_create_browser_sync(
            Some(&window_info),
            Some(&mut ClientBuilder::new(RenderHandlerBuilder::new(handler))),
            Some(&"about:blank".into()),
            Some(&settings),
            None,
            None,
        )
        .expect("browser");
        if let Some(host) = browser.host() {
            chrome_moat::apply(&host);
        }
        if let Some(frame) = browser.main_frame() {
            frame.load_url(Some(&url.into()));
        }
        BrowserCard { browser, shared }
    }

    fn host(&self) -> Option<BrowserHost> {
        self.browser.host()
    }

    fn flags(m: &gpui::Modifiers) -> u32 {
        let mut flags = 0u32;
        if m.shift {
            flags |= sys::cef_event_flags_t::EVENTFLAG_SHIFT_DOWN.0 as u32;
        }
        if m.control {
            flags |= sys::cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0 as u32;
        }
        if m.alt {
            flags |= sys::cef_event_flags_t::EVENTFLAG_ALT_DOWN.0 as u32;
        }
        flags
    }

    fn mouse(x: f32, y: f32, m: &gpui::Modifiers) -> MouseEvent {
        MouseEvent { x: x as i32, y: y as i32, modifiers: Self::flags(m) }
    }

    /// `x`, `y` in page pixels: the caller has already undone the zoom.
    pub fn mouse_move(&self, x: f32, y: f32, m: &gpui::Modifiers) {
        if let Some(host) = self.host() {
            host.send_mouse_move_event(Some(&Self::mouse(x, y, m)), 0);
        }
    }

    pub fn mouse_button(&self, x: f32, y: f32, m: &gpui::Modifiers, up: bool, clicks: usize) {
        if let Some(host) = self.host() {
            if !up {
                host.set_focus(1);
            }
            host.send_mouse_click_event(Some(&Self::mouse(x, y, m)), MouseButtonType::LEFT, up as i32, clicks as i32);
        }
    }

    pub fn wheel(&self, x: f32, y: f32, m: &gpui::Modifiers, dx: f32, dy: f32) {
        if let Some(host) = self.host() {
            host.send_mouse_wheel_event(Some(&Self::mouse(x, y, m)), dx as i32, dy as i32);
        }
    }

    pub fn focus(&self, on: bool) {
        if let Some(host) = self.host() {
            host.set_focus(on as i32);
        }
    }

    /// Cmd chords a page owns. Returns whether it took the key.
    pub fn edit_chord(&self, key: &str) -> bool {
        let Some(frame) = self.browser.main_frame() else { return false };
        match key {
            "v" => frame.paste(),
            "c" => frame.copy(),
            "x" => frame.cut(),
            "a" => frame.select_all(),
            "z" => frame.undo(),
            _ => return false,
        }
        true
    }

    /// Enough of a keyboard for a form: printable keys as CHAR events, the
    /// editing keys as RAWKEYDOWN/KEYUP with Windows virtual key codes.
    pub fn key(&self, k: &gpui::Keystroke) {
        let Some(host) = self.host() else { return };
        let vk = match k.key.as_str() {
            "enter" => 0x0D,
            "backspace" => 0x08,
            "tab" => 0x09,
            "escape" => 0x1B,
            "space" => 0x20,
            "left" => 0x25,
            "up" => 0x26,
            "right" => 0x27,
            "down" => 0x28,
            "delete" => 0x2E,
            _ => 0,
        };
        let modifiers = Self::flags(&k.modifiers);
        if vk != 0 {
            for type_ in [KeyEventType::RAWKEYDOWN, KeyEventType::KEYUP] {
                host.send_key_event(Some(&KeyEvent { type_, modifiers, windows_key_code: vk, ..Default::default() }));
            }
            if vk == 0x20 || vk == 0x0D {
                let ch = if vk == 0x20 { b' ' } else { b'\r' } as u16;
                host.send_key_event(Some(&KeyEvent {
                    type_: KeyEventType::CHAR,
                    modifiers,
                    windows_key_code: vk,
                    character: ch,
                    unmodified_character: ch,
                    ..Default::default()
                }));
            }
            return;
        }
        let Some(text) = k.key_char.as_deref() else { return };
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
}

/// CEF requires the NSApplication to implement `CefAppProtocol`; gpui owns
/// the subclass, so the two methods are added at runtime. See cef-frame's
/// NOTES.md for the abort this prevents.
pub mod cef_app_protocol {
    use objc::runtime::{class_addMethod, Class, Object, Sel, BOOL, NO, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use std::sync::atomic::{AtomicBool, Ordering};

    static HANDLING: AtomicBool = AtomicBool::new(false);

    extern "C" fn is_handling_send_event(_this: &Object, _sel: Sel) -> BOOL {
        if HANDLING.load(Ordering::Relaxed) { YES } else { NO }
    }

    extern "C" fn set_handling_send_event(_this: &Object, _sel: Sel, value: BOOL) {
        HANDLING.store(value == YES, Ordering::Relaxed);
    }

    pub fn install() {
        let app: *mut Object = unsafe { msg_send![class!(NSApplication), sharedApplication] };
        let class: &Class = unsafe { msg_send![app, class] };
        unsafe {
            let getter: extern "C" fn(&Object, Sel) -> BOOL = is_handling_send_event;
            let setter: extern "C" fn(&Object, Sel, BOOL) = set_handling_send_event;
            class_addMethod(class as *const Class as *mut Class, sel!(isHandlingSendEvent), std::mem::transmute(getter), c"c@:".as_ptr());
            class_addMethod(class as *const Class as *mut Class, sel!(setHandlingSendEvent:), std::mem::transmute(setter), c"v@:c".as_ptr());
        }
    }
}
