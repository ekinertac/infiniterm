//! Spike (2026-09-14): CEF off-screen frames inside a gpui window.
//!
//! What it has to show, in order: a page renders through the software
//! `on_paint` path into a gpui `img`; the img can be drawn at any zoom
//! (`=` / `-` / `0`); the mouse is forwarded back to CEF divided by that
//! zoom, so a link under the cursor is the link that gets clicked; and the
//! Claude in Chrome extension loads the same way it did in `cef-extension`,
//! reusing that spike's profile (the claude.ai session and deviceId live
//! there) so Claude Code can drive this window too.
//!
//! Process shape: CEF runs on the main thread with `external_message_pump`;
//! a gpui foreground task calls `do_message_loop_work` every few ms, and
//! `on_paint` therefore lands on that same thread, which is why the frame
//! can sit in an `Rc<RefCell>` shared with the view. The helper binary
//! (`src/bin/helper.rs`) is every non-browser process.
//!
//! Not the browser crate. Measurements and traps go to NOTES.md.
use cef::{args::Args, *};
// gpui is imported by name: both crates glob-export `App`, `Window`, `Point`
// and `MouseEvent`, and the CEF ones are the ones the wrap_* macros expect.
use gpui::{
    div, img, prelude::*, px, rgb, size, AnyElement, AsyncApp, Bounds, Context, FocusHandle,
    KeyDownEvent, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Render, RenderImage, ScrollDelta, ScrollWheelEvent, TitlebarOptions, WindowBounds,
    WindowOptions,
};
use image::{Frame, RgbaImage};
use smallvec::SmallVec;
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

const URL: &str = "https://en.wikipedia.org/wiki/Terminal_emulator";
/// Browser size in logical px; the img is this times the zoom.
const VIEW_W: i32 = 1024;
const VIEW_H: i32 = 768;

/// Where the cef-extension spike left the unpacked extension and the
/// signed-in profile. Absolute so the bundled app finds them from anywhere.
fn spike_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../cef-extension")
}

#[derive(Default)]
struct Shared {
    frame: Option<Arc<RenderImage>>,
    dirty: bool,
    paints: u32,
    /// ms spent copying the last frame, the cost this spike exists to measure
    last_copy_ms: f32,
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
            eprintln!("[spike] view_rect asked");
            if let Some(rect) = rect {
                rect.width = VIEW_W;
                rect.height = VIEW_H;
            }
        }

        fn screen_info(
            &self,
            _browser: Option<&mut Browser>,
            info: Option<&mut ScreenInfo>,
        ) -> ::std::os::raw::c_int {
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
            // Popups (select dropdowns) come as a second element; ignored here.
            if type_ != PaintElementType::VIEW || buffer.is_null() || width <= 0 || height <= 0 {
                return;
            }
            let started = Instant::now();
            let len = (width * height * 4) as usize;
            if self.handler.shared.borrow().paints == 0 {
                eprintln!("[spike] first paint {width}x{height}");
            }
            // CEF hands BGRA and gpui stores its images as BGRA (it swaps
            // decoded PNGs into that order), so this is a straight copy.
            let bytes = unsafe { std::slice::from_raw_parts(buffer, len) }.to_vec();
            let Some(img) = RgbaImage::from_raw(width as u32, height as u32, bytes) else {
                return;
            };
            let render = RenderImage::new(SmallVec::from_elem(Frame::new(img), 1));
            let mut shared = self.handler.shared.borrow_mut();
            shared.frame = Some(Arc::new(render));
            shared.dirty = true;
            shared.paints += 1;
            shared.last_copy_ms = started.elapsed().as_secs_f32() * 1000.0;
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
    struct AppBuilder;

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefStringUtf16>,
            command_line: Option<&mut CommandLine>,
        ) {
            let Some(cmd) = command_line else { return };
            // Chrome bootstrap would otherwise open its own window on start.
            cmd.append_switch(Some(&"no-startup-window".into()));
            cmd.append_switch(Some(&"noerrdialogs".into()));
            cmd.append_switch(Some(&"use-mock-keychain".into()));
            let ext = spike_dir().join("extension");
            cmd.append_switch_with_value(
                Some(&"load-extension".into()),
                Some(&CefString::from(ext.to_string_lossy().as_ref())),
            );
        }
    }
}

struct BrowserView {
    shared: Rc<RefCell<Shared>>,
    browser: cef::Browser,
    zoom: f32,
    focus: FocusHandle,
    started: Instant,
    frames_drawn: u32,
}

impl BrowserView {
    fn host(&self) -> Option<BrowserHost> {
        self.browser.host()
    }

    fn mouse_event(&self, position: gpui::Point<Pixels>, modifiers: &Modifiers) -> MouseEvent {
        // Window px -> page px: the img is at the window origin, scaled by zoom.
        let mut flags = 0u32;
        if modifiers.shift {
            flags |= sys::cef_event_flags_t::EVENTFLAG_SHIFT_DOWN.0 as u32;
        }
        if modifiers.control {
            flags |= sys::cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0 as u32;
        }
        if modifiers.platform {
            flags |= sys::cef_event_flags_t::EVENTFLAG_COMMAND_DOWN.0 as u32;
        }
        MouseEvent {
            x: (f32::from(position.x) / self.zoom) as i32,
            y: (f32::from(position.y) / self.zoom) as i32,
            modifiers: flags,
        }
    }
}

impl Render for BrowserView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frames_drawn += 1;
        let zoom = self.zoom;
        let (frame, paints, copy_ms) = {
            let s = self.shared.borrow();
            (s.frame.clone(), s.paints, s.last_copy_ms)
        };
        let elapsed = self.started.elapsed().as_secs_f32().max(0.001);
        let status = format!(
            "zoom {zoom:.2}  paints {paints} ({:.0}/s)  draws {} ({:.0}/s)  copy {copy_ms:.1}ms  [= - 0 zoom]",
            paints as f32 / elapsed,
            self.frames_drawn,
            self.frames_drawn as f32 / elapsed,
        );

        let page: AnyElement = match frame {
            Some(frame) => img(frame)
                .w(px(VIEW_W as f32 * zoom))
                .h(px(VIEW_H as f32 * zoom))
                .into_any_element(),
            None => div()
                .w(px(VIEW_W as f32 * zoom))
                .h(px(VIEW_H as f32 * zoom))
                .bg(rgb(0x313244))
                .into_any_element(),
        };

        div()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                eprintln!("[spike] key {:?}", e.keystroke);
                match e.keystroke.key.as_str() {
                    "=" | "+" => this.zoom *= 1.25,
                    "-" => this.zoom /= 1.25,
                    "0" => this.zoom = 1.0,
                    _ => return,
                }
                cx.notify();
            }))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, _| {
                        if let Some(host) = this.host() {
                            host.send_mouse_move_event(Some(&this.mouse_event(e.position, &e.modifiers)), 0);
                        }
                    }))
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, window, _| {
                        window.focus(&this.focus);
                        eprintln!("[spike] mouse down at {:?} zoom {}", e.position, this.zoom);
                        if let Some(host) = this.host() {
                            host.set_focus(1);
                            host.send_mouse_click_event(
                                Some(&this.mouse_event(e.position, &e.modifiers)),
                                MouseButtonType::LEFT,
                                0,
                                e.click_count as i32,
                            );
                        }
                    }))
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, e: &MouseUpEvent, _, _| {
                        if let Some(host) = this.host() {
                            host.send_mouse_click_event(
                                Some(&this.mouse_event(e.position, &e.modifiers)),
                                MouseButtonType::LEFT,
                                1,
                                e.click_count as i32,
                            );
                        }
                    }))
                    .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, _| {
                        let (dx, dy) = match e.delta {
                            ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
                            ScrollDelta::Lines(l) => (l.x * 20.0, l.y * 20.0),
                        };
                        if let Some(host) = this.host() {
                            host.send_mouse_wheel_event(
                                Some(&this.mouse_event(e.position, &e.modifiers)),
                                (dx / this.zoom) as i32,
                                (dy / this.zoom) as i32,
                            );
                        }
                    }))
                    .child(page),
            )
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .px_2()
                    .py_1()
                    .bg(rgb(0x11111b))
                    .text_color(rgb(0xcdd6f4))
                    .text_size(px(12.0))
                    .child(status),
            )
    }
}

/// CEF requires the NSApplication to implement `CefAppProtocol`
/// (`isHandlingSendEvent` / `setHandlingSendEvent:`); Chromium's message
/// pump calls the getter and the app aborts with "unrecognized selector"
/// the first time it does, about ten seconds in when the extension opens
/// its window. gpui owns the NSApplication subclass, so the two methods
/// are added to its class at runtime. The flag is not yet set around
/// `sendEvent:` the way cefsimple's subclass does it; noted in NOTES.md.
mod cef_app_protocol {
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
            class_addMethod(
                class as *const Class as *mut Class,
                sel!(isHandlingSendEvent),
                std::mem::transmute(getter),
                c"c@:".as_ptr(),
            );
            class_addMethod(
                class as *const Class as *mut Class,
                sel!(setHandlingSendEvent:),
                std::mem::transmute(setter),
                c"v@:c".as_ptr(),
            );
        }
    }
}

fn main() {
    let _loader = {
        let loader = library_loader::LibraryLoader::new(&std::env::current_exe().unwrap(), false);
        assert!(loader.load(), "CEF framework not found beside the executable; run from the bundle");
        loader
    };
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

    let args = Args::new();
    let mut app = AppBuilder::new();
    // The bundle's helper handles subprocesses; if this binary is ever
    // launched as one, execute_process runs it and returns its exit code.
    let ret = execute_process(Some(args.as_main_args()), Some(&mut app), std::ptr::null_mut());
    if ret >= 0 {
        std::process::exit(ret);
    }

    let profile = spike_dir().join("profile");
    let settings = Settings {
        windowless_rendering_enabled: 1,
        external_message_pump: 1,
        cache_path: profile.to_string_lossy().as_ref().into(),
        ..Default::default()
    };

    gpui::Application::new().run(move |cx: &mut gpui::App| {
        cef_app_protocol::install();
        assert_eq!(
            initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), std::ptr::null_mut()),
            1,
            "cef initialize failed"
        );

        let shared = Rc::new(RefCell::new(Shared::default()));
        let bounds = Bounds::centered(None, size(px(1100.0), px(820.0)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("cef-frame".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    // The browser is created here and not before the window
                    // because CEF asks for the device scale factor at
                    // creation, and only the window knows it.
                    let handler = Handler { shared: shared.clone(), scale: window.scale_factor() };
                    let window_info = WindowInfo {
                        windowless_rendering_enabled: 1,
                        ..Default::default()
                    };
                    let browser_settings = BrowserSettings {
                        windowless_frame_rate: 60,
                        ..Default::default()
                    };
                    let browser = browser_host_create_browser_sync(
                        Some(&window_info),
                        Some(&mut ClientBuilder::new(RenderHandlerBuilder::new(handler))),
                        Some(&URL.into()),
                        Some(&browser_settings),
                        None,
                        None,
                    )
                    .expect("browser");
                    cx.new(|cx| BrowserView {
                        shared: shared.clone(),
                        browser,
                        zoom: 1.0,
                        focus: cx.focus_handle(),
                        started: Instant::now(),
                        frames_drawn: 0,
                    })
                },
            )
            .expect("window");
        cx.activate(true);

        // The message pump. CEF's on_schedule_message_pump_work would say
        // when; a fixed cadence is enough to measure the frame path.
        let shared = shared.clone();
        let mut ticks = 0u32;
        cx.spawn(async move |cx: &mut AsyncApp| loop {
            do_message_loop_work();
            ticks += 1;
            if ticks % 500 == 0 {
                eprintln!("[spike] pump tick {ticks}, paints {}", shared.borrow().paints);
            }
            let dirty = std::mem::take(&mut shared.borrow_mut().dirty);
            if dirty {
                if let Err(e) = window.update(cx, |_, window, cx| {
                    cx.notify();
                    window.refresh();
                }) {
                    eprintln!("[spike] window update failed: {e}");
                }
            }
            cx.background_executor().timer(Duration::from_millis(4)).await;
        })
        .detach();
    });
    shutdown();
}
