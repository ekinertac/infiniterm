// The macOS half of the webview spike: the gpui canvas, the wry child
// webviews and the AppKit calls that wry does not wrap (layer mask, alpha,
// bounds scale, snapshot, hit test, synthetic keys).
//
// Where it fits: main.rs starts it; cmd.rs parses the commands it carries
// out. Everything native runs on the main thread, inside `Spike::tick`,
// which a 30 ms timer calls with the window.
// Constraints: nothing here may be copied into the product unreviewed. It
// uses unsafe objc calls with no error handling beyond logging, on purpose,
// because it is a measurement tool.

use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use block2::RcBlock;
use gpui::{
    div, img, point, prelude::*, px, rgb, size, App, Application, AsyncApp, Bounds, Context,
    FocusHandle, KeyDownEvent, ObjectFit, RenderImage, Window, WindowBounds,
    WindowOptions,
};
use objc2::{
    class, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool},
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use image::Frame as ImageFrame;
use smallvec::SmallVec;
use wry::{
    dpi::{LogicalPosition, LogicalSize},
    NewWindowResponse, Rect, WebView, WebViewBuilder, WebViewBuilderExtDarwin, WebViewExtMacOS,
};

use crate::cmd::{self, Cmd};

// A snapshot arrives on a block called by WebKit; it is parked here and the
// next tick turns it into a gpui image.
static SNAPS: Mutex<Vec<(String, Vec<u8>)>> = Mutex::new(Vec::new());
// The local NSEvent monitor logs through this, from inside AppKit.
static LOG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

fn log(msg: &str) {
    eprintln!("{msg}");
    if let Some(p) = LOG_PATH.lock().unwrap().as_ref() {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "{msg}");
        }
    }
}

enum Kind {
    Web { url: String },
    Plain { color: u32 },
}

struct Card {
    id: String,
    world: [f32; 4],
    kind: Kind,
    dim: f32,
    snapshot: Option<Arc<RenderImage>>,
    wv: Option<WebView>,
}

struct Spike {
    cards: Vec<Card>, // paint order: later covers earlier
    zoom: f32,
    pan: (f32, f32),
    zoom_mode: String,
    cover: String,
    dir: PathBuf,
    cmd_len: usize,
    store: String,
    ua: Option<String>,
    focus: FocusHandle,
    dirty: bool,
    last_sig: String,
    anim: Option<(std::time::Instant, u64, Vec<f32>)>,
    shields: HashMap<String, Retained<AnyObject>>,
    ticks: u64,
    quit_after: Option<u64>,
}

fn store_id(seed: &str) -> [u8; 16] {
    // Not a hash for security: a stable 16 bytes from the seed, so the same
    // data dir always maps to the same WKWebsiteDataStore.
    let mut out = [0u8; 16];
    let mut h: u64 = 0xcbf29ce484222325;
    for b in seed.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    for (i, o) in out.iter_mut().enumerate() {
        h ^= i as u64;
        h = h.wrapping_mul(0x100000001b3);
        *o = (h >> 32) as u8;
    }
    out[6] = (out[6] & 0x0f) | 0x40;
    out[8] = (out[8] & 0x3f) | 0x80;
    out
}

impl Spike {
    fn screen_rect(&self, c: &Card) -> [f32; 4] {
        let z = self.zoom;
        [
            (c.world[0] - self.pan.0) * z,
            (c.world[1] - self.pan.1) * z,
            c.world[2] * z,
            c.world[3] * z,
        ]
    }

    fn card_mut(&mut self, id: &str) -> Option<&mut Card> {
        self.cards.iter_mut().find(|c| c.id == id)
    }

    fn create_webview(&self, window: &Window, id: &str, url: &str, rect: [f32; 4]) -> Option<WebView> {
        let ident = id.to_string();
        let mut b = WebViewBuilder::new()
            .with_bounds(Rect {
                position: LogicalPosition::new(rect[0] as f64, rect[1] as f64).into(),
                size: LogicalSize::new(rect[2] as f64, rect[3] as f64).into(),
            })
            .with_url(url)
            .with_devtools(true)
            .with_accept_first_mouse(true);
        if let Some(ua) = &self.ua {
            b = b.with_user_agent(ua.clone());
        }
        match self.store.as_str() {
            "default" => {}
            "incognito" => b = b.with_incognito(true),
            seed => b = b.with_data_store_identifier(store_id(seed)),
        }
        let i1 = ident.clone();
        b = b.with_new_window_req_handler(move |u, f| {
            log(&format!("[{i1}] new window requested: {u} size={:?}", f.size));
            NewWindowResponse::Deny
        });
        let i2 = ident.clone();
        b = b.with_navigation_handler(move |u| {
            log(&format!("[{i2}] navigation: {u}"));
            !u.starts_with("slack:")
        });
        let i3 = ident.clone();
        b = b.with_document_title_changed_handler(move |t| log(&format!("[{i3}] title: {t}")));
        let i4 = ident.clone();
        b = b.with_on_page_load_handler(move |e, u| {
            let kind = match e {
                wry::PageLoadEvent::Started => "started",
                wry::PageLoadEvent::Finished => "finished",
            };
            log(&format!("[{i4}] load {kind}: {u}"))
        });
        let i5 = ident.clone();
        b = b.with_download_started_handler(move |u, p| {
            log(&format!("[{i5}] download started: {u} -> {p:?}"));
            true
        });
        let i6 = ident;
        b = b.with_download_completed_handler(move |u, p, ok| {
            log(&format!("[{i6}] download done: {u} {p:?} ok={ok}"))
        });
        match b.build_as_child(window) {
            Ok(wv) => {
                if std::env::var("WV_MINFONT").is_ok() {
                    // WebKit keeps text from shrinking below about 9 px
                    // after page zoom (minimum logical font size); try to
                    // lower it through the private preferences setter.
                    unsafe {
                        let v = wv.webview();
                        let cfg: *mut AnyObject = msg_send![&*v, configuration];
                        let prefs: *mut AnyObject = msg_send![cfg, preferences];
                        let sel = objc2::sel!(_setMinimumLogicalFontSize:);
                        let ok: Bool = msg_send![prefs, respondsToSelector: sel];
                        log(&format!("[{id}] prefs responds to _setMinimumLogicalFontSize: {}", ok.as_bool()));
                        if ok.as_bool() {
                            let _: () = msg_send![prefs, _setMinimumLogicalFontSize: 1.0f64];
                        }
                        let _: () = msg_send![prefs, setMinimumFontSize: 0.0f64];
                    }
                }
                Some(wv)
            }
            Err(e) => {
                log(&format!("[{id}] build_as_child failed: {e}"));
                None
            }
        }
    }

    fn apply(&mut self, c: Cmd, window: &mut Window) -> bool {
        self.dirty = true;
        match c {
            Cmd::Web { id, rect, url } => {
                let sr = {
                    let tmp = Card { id: id.clone(), world: rect, kind: Kind::Plain { color: 0 }, dim: 1.0, snapshot: None, wv: None };
                    self.screen_rect(&tmp)
                };
                let wv = self.create_webview(window, &id, &url, sr);
                log(&format!("[{id}] webview created: {}", wv.is_some()));
                self.cards.retain(|c| c.id != id);
                self.cards.push(Card { id, world: rect, kind: Kind::Web { url }, dim: 1.0, snapshot: None, wv });
            }
            Cmd::Plain { id, rect, color } => {
                self.cards.retain(|c| c.id != id);
                self.cards.push(Card { id, world: rect, kind: Kind::Plain { color }, dim: 1.0, snapshot: None, wv: None });
            }
            Cmd::Move { id, x, y } => {
                if let Some(c) = self.card_mut(&id) {
                    c.world[0] = x;
                    c.world[1] = y;
                }
            }
            Cmd::Size { id, w, h } => {
                if let Some(c) = self.card_mut(&id) {
                    c.world[2] = w;
                    c.world[3] = h;
                }
            }
            Cmd::Zoom(z) => self.zoom = z,
            Cmd::Anim(ms) => self.anim = Some((std::time::Instant::now(), ms, Vec::new())),
            Cmd::Pan(x, y) => self.pan = (x, y),
            Cmd::ZoomMode(m) => self.zoom_mode = m,
            Cmd::Cover(m) => self.cover = m,
            Cmd::Dim { id, alpha } => {
                if let Some(c) = self.card_mut(&id) {
                    c.dim = alpha;
                }
            }
            Cmd::Front(id) => {
                if let Some(p) = self.cards.iter().position(|c| c.id == id) {
                    let c = self.cards.remove(p);
                    self.cards.push(c);
                }
            }
            Cmd::Close(id) => {
                if let Some(p) = self.cards.iter().position(|c| c.id == id) {
                    // Dropping the WebView removes the view from the window.
                    self.cards.remove(p);
                    log(&format!("[{id}] closed"));
                }
            }
            Cmd::Eval { id, js } => {
                if let Some(wv) = self.card_mut(&id).and_then(|c| c.wv.as_ref()) {
                    let idc = id.clone();
                    let r = wv.evaluate_script_with_callback(&js, move |s| {
                        log(&format!("[{idc}] eval => {s}"))
                    });
                    if let Err(e) = r {
                        log(&format!("[{id}] eval error: {e}"));
                    }
                } else {
                    log(&format!("[{id}] eval: no webview"));
                }
            }
            Cmd::Snap(id) => {
                if let Some(wv) = self.card_mut(&id).and_then(|c| c.wv.as_ref()) {
                    take_snapshot(wv, &id);
                }
            }
            Cmd::Hit(x, y) => hit_test(window, x, y),
            Cmd::Click(x, y) => send_click(window, x, y),
            Cmd::Key { id: _, chars, cmd } => {
                // To whoever is first responder; `focus` chooses.
                log(&format!("key to first responder {}", first_responder_class(window)));
                send_key(window, &chars, cmd);
            }
            Cmd::Focus(id) => {
                if id == "app" {
                    window.focus(&self.focus);
                    if let Some(wv) = self.cards.iter().find_map(|c| c.wv.as_ref()) {
                        let _ = wv.focus_parent();
                    }
                } else if let Some(wv) = self.card_mut(&id).and_then(|c| c.wv.as_ref()) {
                    let _ = wv.focus();
                }
                log(&format!("first responder: {}", first_responder_class(window)));
            }
            Cmd::Url { id, url } => {
                if let Some(wv) = self.card_mut(&id).and_then(|c| c.wv.as_ref()) {
                    let _ = wv.load_url(&url);
                }
            }
            Cmd::Devtools(id) => {
                if let Some(wv) = self.card_mut(&id).and_then(|c| c.wv.as_ref()) {
                    // Docked, the inspector reparents the webview and fills the
                    // whole window; detached, it is a window of its own.
                    wv.open_devtools();
                    unsafe {
                        let v = wv.webview();
                        let tool: *mut AnyObject = msg_send![&*v, _inspector];
                        let ok: Bool = msg_send![tool, respondsToSelector: objc2::sel!(detach)];
                        log(&format!("[{id}] inspector responds to detach: {}", ok.as_bool()));
                        if ok.as_bool() {
                            let _: () = msg_send![tool, detach];
                        }
                    }
                    log(&format!("[{id}] devtools open: {}", wv.is_devtools_open()));
                }
            }
            Cmd::Find { id, text } => {
                if let Some(wv) = self.card_mut(&id).and_then(|c| c.wv.as_ref()) {
                    let idc = id.clone();
                    let block = RcBlock::new(move |res: *mut AnyObject| unsafe {
                        let found: Bool = if res.is_null() { Bool::NO } else { msg_send![res, matchFound] };
                        log(&format!("[{idc}] find: matchFound={}", found.as_bool()));
                    });
                    let s = NSString::from_str(&text);
                    let v = wv.webview();
                    unsafe {
                        let cfg: *mut AnyObject = msg_send![objc2::class!(WKFindConfiguration), new];
                        let _: () = msg_send![cfg, setWraps: Bool::YES];
                        let _: () = msg_send![&*v, findString: &*s, withConfiguration: cfg, completionHandler: &*block];
                    }
                }
            }
            Cmd::Windows => list_windows(),
            Cmd::Info => self.info(window),
            Cmd::Quit => return false,
        }
        true
    }

    fn info(&self, window: &Window) {
        log(&format!(
            "info: zoom={} pan={:?} zoommode={} cover={} cards={} viewport={:?} scale={}",
            self.zoom,
            self.pan,
            self.zoom_mode,
            self.cover,
            self.cards.len(),
            window.viewport_size(),
            window.scale_factor()
        ));
        for c in &self.cards {
            if let Some(wv) = &c.wv {
                let v = wv.webview();
                unsafe {
                    let pz: f64 = msg_send![&*v, pageZoom];
                    let mag: f64 = msg_send![&*v, magnification];
                    let b: NSRect = msg_send![&*v, bounds];
                    let f: NSRect = msg_send![&*v, frame];
                    log(&format!("[{}] pageZoom={pz} magnification={mag} bounds={:?} frame={:?}", c.id, b.size, f.size));
                }
            }
        }
        if let Some(view) = native_view(window) {
            unsafe {
                let layer: *mut AnyObject = msg_send![&*view, layer];
                let lcls = class_name(layer);
                let subs: *mut AnyObject = msg_send![&*view, subviews];
                let n: usize = msg_send![subs, count];
                let sub_layers: *mut AnyObject = msg_send![layer, sublayers];
                let sl: usize = if sub_layers.is_null() { 0 } else { msg_send![sub_layers, count] };
                log(&format!("gpui view layer={lcls} subviews={n} sublayers={sl} flipped={}", {
                    let f: Bool = msg_send![&*view, isFlipped];
                    f.as_bool()
                }));
                for i in 0..n {
                    let s: *mut AnyObject = msg_send![subs, objectAtIndex: i];
                    let fr: NSRect = msg_send![s, frame];
                    log(&format!("  subview {i}: {} frame={:?}", class_name(s), fr));
                }
            }
        }
    }

    /// Native side of every frame: frame, zoom, cover, dim.
    /// Everything the native side depends on, as text: when it has not
    /// changed since the last frame nothing is sent to WebKit. Setting the
    /// frame and page zoom every tick cost ~15 points of CPU with 5 webviews.
    fn native_signature(&self, view_h: f32) -> String {
        let mut s = format!("{}|{:?}|{}|{}|{}", self.zoom, self.pan, self.zoom_mode, self.cover, view_h);
        for c in &self.cards {
            s += &format!("|{}:{:?}:{}:{}", c.id, c.world, c.dim, c.snapshot.is_some());
        }
        s
    }

    fn sync_native(&mut self, view_h: f32) {
        let sig = self.native_signature(view_h);
        if sig == self.last_sig {
            return;
        }
        self.last_sig = sig;
        let rects: Vec<[f32; 4]> = self.cards.iter().map(|c| self.screen_rect(c)).collect();
        let zoom = self.zoom;
        let mode = self.zoom_mode.clone();
        let cover = self.cover.clone();
        for i in 0..self.cards.len() {
            let r = rects[i];
            let c = &self.cards[i];
            let Some(wv) = c.wv.as_ref() else { continue };
            // Cards later in the vec are drawn over this one.
            let holes: Vec<[f32; 4]> = rects[i + 1..]
                .iter()
                .filter_map(|o| intersect(r, *o))
                .collect();
            let covered = !holes.is_empty();
            let hide = match cover.as_str() {
                "snap" => covered && c.snapshot.is_some(),
                "hide" => covered,
                _ => false,
            };
            let _ = wv.set_bounds(Rect {
                position: LogicalPosition::new(r[0] as f64, r[1] as f64).into(),
                size: LogicalSize::new(r[2] as f64, r[3] as f64).into(),
            });
            let view = wv.webview();
            unsafe {
                match mode.as_str() {
                    "bounds" => {
                        let _ = wv.zoom(1.0);
                        let _: () = msg_send![&*view, setBoundsSize: NSSize::new((c.world[2]) as f64, (c.world[3]) as f64)];
                    }
                    "mag" => {
                        let _ = wv.zoom(1.0);
                        let _: () = msg_send![&*view, setAllowsMagnification: Bool::YES];
                        let _: () = msg_send![&*view, setMagnification: zoom as f64];
                    }
                    _ => {
                        let _: () = msg_send![&*view, setMagnification: 1.0f64];
                        let _: () = msg_send![&*view, setBoundsSize: NSSize::new(r[2] as f64, r[3] as f64)];
                        let _ = wv.zoom(zoom as f64);
                    }
                }
                let _ = wv.set_visible(!hide);
                let a = c.dim as f64;
                let _: () = msg_send![&*view, setAlphaValue: a];
                // A translucent colour view above the webview is the other
                // way to dim it; `dim` below 1 puts one there. It is a plain
                // NSView, so it also takes clicks and passes them up.
                let dsup: *mut AnyObject = msg_send![&*view, superview];
                let dk = format!("dim-{}", c.id);
                let want_dim = c.dim < 1.0;
                if want_dim && !self.shields.contains_key(&dk) {
                    let dv: Retained<AnyObject> = msg_send![msg_send![class!(NSView), alloc], initWithFrame: NSRect::ZERO];
                    let _: () = msg_send![&*dv, setWantsLayer: Bool::YES];
                    let dl: *mut AnyObject = msg_send![&*dv, layer];
                    let col: *mut AnyObject = msg_send![class!(NSColor), colorWithSRGBRed: 0.106f64, green: 0.114f64, blue: 0.133f64, alpha: (1.0 - c.dim) as f64];
                    let cg: *mut AnyObject = msg_send![col, CGColor];
                    let _: () = msg_send![dl, setBackgroundColor: cg];
                    let _: () = msg_send![dsup, addSubview: &*dv, positioned: 1isize, relativeTo: &*view];
                    self.shields.insert(dk.clone(), dv);
                }
                if let Some(dv) = self.shields.get(&dk) {
                    let _: () = msg_send![&**dv, setHidden: Bool::new(!want_dim)];
                    let fr: NSRect = msg_send![&*view, frame];
                    let _: () = msg_send![&**dv, setFrame: fr];
                }
                apply_mask(&view, r, if cover == "hole" { &holes } else { &[] });
                // A layer mask does not change hit testing: a click in a hole
                // still reaches the webview. A plain NSView above it, which
                // passes every event up to the gpui view, takes those clicks.
                let want = cover == "hole" && !holes.is_empty();
                let sup: *mut AnyObject = msg_send![&*view, superview];
                let id = c.id.clone();
                if want && !self.shields.contains_key(&id) {
                    let sh: Retained<AnyObject> = msg_send![msg_send![class!(NSView), alloc], initWithFrame: NSRect::ZERO];
                    let _: () = msg_send![sup, addSubview: &*sh, positioned: 1isize, relativeTo: &*view];
                    self.shields.insert(id.clone(), sh);
                }
                if let Some(sh) = self.shields.get(&id) {
                    let _: () = msg_send![&**sh, setHidden: Bool::new(!want)];
                    if want {
                        let h0 = holes[0];
                        let y = view_h - h0[1] - h0[3];
                        let _: () = msg_send![&**sh, setFrame: NSRect::new(NSPoint::new(h0[0] as f64, y as f64), NSSize::new(h0[2] as f64, h0[3] as f64))];
                    }
                }
            }
        }
    }
}

fn intersect(a: [f32; 4], b: [f32; 4]) -> Option<[f32; 4]> {
    let x0 = a[0].max(b[0]);
    let y0 = a[1].max(b[1]);
    let x1 = (a[0] + a[2]).min(b[0] + b[2]);
    let y1 = (a[1] + a[3]).min(b[1] + b[3]);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPathCreateMutable() -> *mut std::ffi::c_void;
    fn CGPathAddRect(p: *mut std::ffi::c_void, t: *const std::ffi::c_void, r: NSRect);
    fn CGPathRelease(p: *mut std::ffi::c_void);
}

/// A CAShapeLayer mask over the webview's own layer: the whole view minus
/// the holes (even-odd), so a card drawn by gpui beneath shows through.
unsafe fn apply_mask(view: &AnyObject, r: [f32; 4], holes: &[[f32; 4]]) {
    let layer: *mut AnyObject = msg_send![view, layer];
    if layer.is_null() {
        return;
    }
    if holes.is_empty() {
        let _: () = msg_send![layer, setMask: std::ptr::null::<AnyObject>()];
        return;
    }
    let flipped: Bool = msg_send![view, isFlipped];
    let (w, h) = (r[2] as f64, r[3] as f64);
    let path = CGPathCreateMutable();
    CGPathAddRect(path, std::ptr::null(), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h)));
    for ho in holes {
        let x = (ho[0] - r[0]) as f64;
        let mut y = (ho[1] - r[1]) as f64;
        if !flipped.as_bool() {
            y = h - y - ho[3] as f64;
        }
        CGPathAddRect(path, std::ptr::null(), NSRect::new(NSPoint::new(x, y), NSSize::new(ho[2] as f64, ho[3] as f64)));
    }
    let mask: *mut AnyObject = msg_send![class!(CAShapeLayer), layer];
    let _: () = msg_send![mask, setFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h))];
    let rule = NSString::from_str("even-odd");
    let _: () = msg_send![mask, setFillRule: &*rule];
    let _: () = msg_send![mask, setPath: path];
    let _: () = msg_send![layer, setMask: mask];
    CGPathRelease(path);
}

unsafe fn class_name(o: *mut AnyObject) -> String {
    if o.is_null() {
        return "nil".into();
    }
    (*o).class().name().to_str().unwrap_or("?").to_string()
}

fn native_view(window: &Window) -> Option<Retained<AnyObject>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    // gpui's own `Window::window_handle` (an AnyWindowHandle) shadows the
    // trait method, so the trait is named.
    let h = HasWindowHandle::window_handle(window).ok()?;
    match h.as_raw() {
        RawWindowHandle::AppKit(a) => unsafe {
            Retained::retain(a.ns_view.as_ptr() as *mut AnyObject)
        },
        _ => None,
    }
}

fn first_responder_class(window: &Window) -> String {
    unsafe {
        let Some(v) = native_view(window) else { return "?".into() };
        let w: *mut AnyObject = msg_send![&*v, window];
        let fr: *mut AnyObject = msg_send![w, firstResponder];
        class_name(fr)
    }
}

fn hit_test(window: &Window, x: f32, y: f32) {
    unsafe {
        let Some(v) = native_view(window) else { return };
        let sup: *mut AnyObject = msg_send![&*v, superview];
        // The point is in the gpui view's own coordinates (top-left origin
        // because the view is flipped); convert to its superview's.
        // x and y arrive top-left origin, like every other coordinate here.
        let b: NSRect = msg_send![&*v, bounds];
        let p: NSPoint = msg_send![&*v, convertPoint: NSPoint::new(x as f64, b.size.height - y as f64), toView: sup];
        let hit: *mut AnyObject = msg_send![&*v, hitTest: p];
        log(&format!("hit ({x},{y}) => {}", class_name(hit)));
    }
}

fn send_click(window: &Window, x: f32, y: f32) {
    unsafe {
        let Some(v) = native_view(window) else { return };
        let w: *mut AnyObject = msg_send![&*v, window];
        let num: isize = msg_send![w, windowNumber];
        let b: NSRect = msg_send![&*v, bounds];
        let loc = NSPoint::new(x as f64, b.size.height - y as f64);
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        for ty in [1usize, 2usize] {
            // 1 = NSEventTypeLeftMouseDown, 2 = NSEventTypeLeftMouseUp
            let ev: *mut AnyObject = msg_send![class!(NSEvent),
                mouseEventWithType: ty, location: loc, modifierFlags: 0usize, timestamp: 0.0f64,
                windowNumber: num, context: std::ptr::null::<AnyObject>(), eventNumber: 1isize,
                clickCount: 1isize, pressure: 1.0f32];
            let _: () = msg_send![app, sendEvent: ev];
        }
    }
}

fn list_windows() {
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let ws: *mut AnyObject = msg_send![app, windows];
        let n: usize = msg_send![ws, count];
        for i in 0..n {
            let w: *mut AnyObject = msg_send![ws, objectAtIndex: i];
            let t: *mut AnyObject = msg_send![w, title];
            let title = if t.is_null() {
                String::new()
            } else {
                let c: *const std::ffi::c_char = msg_send![t, UTF8String];
                std::ffi::CStr::from_ptr(c).to_string_lossy().into_owned()
            };
            let vis: Bool = msg_send![w, isVisible];
            let fr: NSRect = msg_send![w, frame];
            log(&format!("window {i}: {} title={title:?} visible={} frame={:?}", class_name(w), vis.as_bool(), fr));
        }
    }
}

fn take_snapshot(wv: &WebView, id: &str) {
    let id = id.to_string();
    let block = RcBlock::new(move |image: *mut AnyObject, err: *mut AnyObject| unsafe {
        if image.is_null() {
            log(&format!("[{id}] snapshot failed (err null: {})", err.is_null()));
            return;
        }
        let tiff: *mut AnyObject = msg_send![image, TIFFRepresentation];
        let rep: *mut AnyObject = msg_send![class!(NSBitmapImageRep), imageRepWithData: tiff];
        let png: *mut AnyObject = msg_send![rep, representationUsingType: 4usize, properties: std::ptr::null::<AnyObject>()];
        let len: usize = msg_send![png, length];
        let bytes: *const u8 = msg_send![png, bytes];
        let data = std::slice::from_raw_parts(bytes, len).to_vec();
        SNAPS.lock().unwrap().push((id.clone(), data));
    });
    let view = wv.webview();
    unsafe {
        let _: () = msg_send![&*view, takeSnapshotWithConfiguration: std::ptr::null::<AnyObject>(), completionHandler: &*block];
    }
}

fn send_key(window: &Window, chars: &str, cmd: bool) {
    unsafe {
        let Some(v) = native_view(window) else { return };
        let w: *mut AnyObject = msg_send![&*v, window];
        let num: isize = msg_send![w, windowNumber];
        let s = NSString::from_str(chars);
        let flags: usize = if cmd { 1 << 20 } else { 0 };
        for ty in [10usize, 11usize] {
            // 10 = NSEventTypeKeyDown, 11 = NSEventTypeKeyUp
            let ev: *mut AnyObject = msg_send![class!(NSEvent),
                keyEventWithType: ty, location: NSPoint::new(0.0, 0.0), modifierFlags: flags,
                timestamp: 0.0f64, windowNumber: num, context: std::ptr::null::<AnyObject>(),
                characters: &*s, charactersIgnoringModifiers: &*s, isARepeat: Bool::NO, keyCode: 40u16];
            if cmd && ty == 10 {
                // The real pipeline offers a Cmd chord to the window's views
                // as a key equivalent before it is a key-down.
                let handled: Bool = msg_send![w, performKeyEquivalent: ev];
                log(&format!("window.performKeyEquivalent => {}", handled.as_bool()));
                continue;
            }
            let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, sendEvent: ev];
        }
    }
}

/// A local monitor sees every key event before the window dispatches it, the
/// way infiniterm's keycode.rs does; here it only logs which view was first
/// responder.
fn install_key_monitor() {
    let block = RcBlock::new(|ev: *mut AnyObject| -> *mut AnyObject {
        unsafe {
            let ty0: usize = msg_send![ev, type];
            if ty0 == 1 {
                let w: *mut AnyObject = msg_send![ev, window];
                let loc: NSPoint = msg_send![ev, locationInWindow];
                log(&format!("monitor: mouseDown window={} at {:?}", class_name(w), loc));
                return ev;
            }
            let ch: *mut AnyObject = msg_send![ev, characters];
            let c: *const std::ffi::c_char = msg_send![ch, UTF8String];
            let s = std::ffi::CStr::from_ptr(c).to_string_lossy().into_owned();
            let flags: usize = msg_send![ev, modifierFlags];
            let ty: usize = msg_send![ev, type];
            if ty == 10 {
                log(&format!("monitor: keyDown {s:?} cmd={}", flags & (1 << 20) != 0));
            }
        }
        ev
    });
    unsafe {
        let _: *mut AnyObject = msg_send![class!(NSEvent), addLocalMonitorForEventsMatchingMask: (1usize << 10) | (1usize << 1), handler: &*block];
    }
    std::mem::forget(block);
}

impl Render for Spike {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .size_full()
            .bg(rgb(0x1b1d22))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_this, ev: &KeyDownEvent, _w, _cx| {
                log(&format!(
                    "gpui: keyDown {:?} cmd={}",
                    ev.keystroke.key, ev.keystroke.modifiers.platform
                ));
            }))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|_t, ev: &gpui::MouseDownEvent, _w, _cx| {
                log(&format!("gpui: mouseDown at {:?}", ev.position));
            }))
            .relative();
        for c in &self.cards {
            let r = self.screen_rect(c);
            let (bg, label) = match &c.kind {
                Kind::Plain { color } => (*color, format!("plain {}", c.id)),
                Kind::Web { url } => (0x303844, format!("web {} {url}", c.id)),
            };
            let mut d = div()
                .absolute()
                .left(px(r[0]))
                .top(px(r[1]))
                .w(px(r[2]))
                .h(px(r[3]))
                .bg(rgb(bg))
                .border_1()
                .border_color(rgb(0x8a93a3))
                .text_color(rgb(0xffffff))
                .text_size(px(14.0))
                .child(label);
            if matches!(c.kind, Kind::Plain { .. }) {
                d = d.opacity(c.dim);
            }
            if let Some(s) = &c.snapshot {
                d = d.child(
                    div().absolute().top_0().left_0().size_full().child(
                        img(s.clone()).size_full().object_fit(ObjectFit::Fill),
                    ),
                );
            }
            root = root.child(d);
        }
        root
    }
}

fn read_new_commands(dir: &PathBuf, seen: &mut usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(dir.join("cmd")) else { return vec![] };
    let lines: Vec<&str> = text.lines().collect();
    let out = lines[(*seen).min(lines.len())..].iter().map(|s| s.to_string()).collect();
    *seen = lines.len();
    out
}

fn decode_snapshot(png: &[u8]) -> Option<Arc<RenderImage>> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let mut raw = img.into_raw();
    for px in raw.chunks_exact_mut(4) {
        px.swap(0, 2); // gpui stores BGRA
    }
    let buf = image::RgbaImage::from_raw(w, h, raw)?;
    Some(Arc::new(RenderImage::new(SmallVec::from_elem(ImageFrame::new(buf), 1))))
}

pub fn run() {
    let dir = PathBuf::from(std::env::var("WV_DIR").expect("set WV_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    *LOG_PATH.lock().unwrap() = Some(dir.join("out.log"));
    let store = std::env::var("WV_STORE").unwrap_or_else(|_| "default".into());
    let ua = std::env::var("WV_UA").ok();
    let quit_after = std::env::var("WV_QUIT_AFTER_MS").ok().and_then(|s| s.parse().ok());

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::new(point(px(60.0), px(60.0)), size(px(1100.0), px(760.0)));
        let handle = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus);
                    cx.new(|_| Spike {
                        cards: vec![],
                        zoom: 1.0,
                        pan: (0.0, 0.0),
                        zoom_mode: "page".into(),
                        cover: "none".into(),
                        dir: dir.clone(),
                        cmd_len: 0,
                        store: store.clone(),
                        ua: ua.clone(),
                        focus,
                        dirty: true,
                        last_sig: String::new(),
                        anim: None,
                        shields: HashMap::new(),
                        ticks: 0,
                        quit_after,
                    })
                },
            )
            .expect("window");
        install_key_monitor();
        cx.activate(true);
        cx.spawn(async move |cx: &mut AsyncApp| loop {
            cx.background_executor().timer(Duration::from_millis(16)).await;
            let alive = handle
                .update(cx, |this, window, cx| {
                    {
                        this.ticks += 1;
                        let lines = read_new_commands(&this.dir.clone(), &mut this.cmd_len);
                        let mut alive = true;
                        for l in lines {
                            log(&format!("> {l}"));
                            match cmd::parse(&l) {
                                Ok(c) => alive &= this.apply(c, window),
                                Err(e) => log(&format!("bad command: {e}")),
                            }
                        }
                        let snaps: Vec<_> = std::mem::take(&mut *SNAPS.lock().unwrap());
                        for (id, png) in snaps {
                            if let Some(img) = decode_snapshot(&png) {
                                let _ = std::fs::write(this.dir.join(format!("snap-{id}.png")), &png);
                                log(&format!("[{id}] snapshot ok, {} bytes png", png.len()));
                                if let Some(c) = this.card_mut(&id) {
                                    c.snapshot = Some(img);
                                }
                                this.dirty = true;
                            }
                        }
                        let vh: f32 = window.viewport_size().height.into();
                        if let Some((t0, ms, costs)) = this.anim.take() {
                            let t = t0.elapsed().as_secs_f32() * 1000.0 / ms as f32;
                            if t >= 1.0 {
                                costs.iter().for_each(|_| ());
                                let n = costs.len().max(1);
                                let avg = costs.iter().sum::<f32>() / n as f32;
                                let max = costs.iter().cloned().fold(0.0, f32::max);
                                log(&format!("anim done: {} steps, sync_native avg {avg:.2} ms max {max:.2} ms", costs.len()));
                                this.zoom = 1.0;
                            } else {
                                this.zoom = 1.0 + 0.5 * (t * 4.0 * std::f32::consts::PI).sin();
                                let s = std::time::Instant::now();
                                this.sync_native(vh);
                                let mut costs = costs;
                                costs.push(s.elapsed().as_secs_f32() * 1000.0);
                                this.anim = Some((t0, ms, costs));
                                this.dirty = true;
                            }
                        } else {
                            this.sync_native(vh);
                        }
                        if this.dirty {
                            this.dirty = false;
                            cx.notify();
                        }
                        if let Some(ms) = this.quit_after {
                            if this.ticks * 16 > ms {
                                alive = false;
                            }
                        }
                        alive
                    }
                })
                .unwrap_or(false);
            if !alive {
                let _ = cx.update(|app| app.quit());
                break;
            }
        })
        .detach();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersect_gives_the_shared_rect() {
        assert_eq!(intersect([0., 0., 10., 10.], [5., 5., 10., 10.]), Some([5., 5., 5., 5.]));
        assert_eq!(intersect([0., 0., 10., 10.], [10., 0., 5., 5.]), None);
    }

    #[test]
    fn store_ids_are_stable_and_differ_by_seed() {
        assert_eq!(store_id("a"), store_id("a"));
        assert_ne!(store_id("a"), store_id("b"));
        assert_eq!(store_id("a")[6] >> 4, 4);
    }
}
