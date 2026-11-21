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
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
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
use crate::gesture;

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

/// The height of a card's label strip, in screen pixels at any zoom. The
/// page sits below it, as a browser card's tab strip sits above its page.
const STRIP: f32 = 24.0;

/// What the left or middle button is doing on the canvas.
enum Press {
    /// Panning: the last pointer position.
    Pan((f32, f32)),
    /// Dragging a card by its strip: the card, and where in world units the
    /// pointer took it.
    Card(String, (f32, f32)),
}

struct Spike {
    win: (f32, f32),
    selected: Option<String>,
    inactive_dim: bool,
    press: Option<Press>,
    ghost: Option<[f32; 4]>,
    palette: bool,
    pointer: (f32, f32),
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

    /// Where the page of a web card is: the card's rect under its strip.
    fn page_rect(&self, c: &Card) -> [f32; 4] {
        let r = self.screen_rect(c);
        [r[0], r[1] + STRIP, r[2], (r[3] - STRIP).max(1.0)]
    }

    /// An explicit `dim` wins; otherwise unselected cards dim to 0.8 when
    /// `inactivedim` is on, as `ui.inactiveDim` does.
    fn eff_dim(&self, c: &Card) -> f32 {
        if c.dim < 1.0 {
            c.dim
        } else if self.inactive_dim && self.selected.as_deref() != Some(c.id.as_str()) {
            0.8
        } else {
            1.0
        }
    }

    /// The command-palette stand-in: a panel gpui draws over the middle of the
    /// window, over whatever pages are there.
    fn palette_rect(&self, win: (f32, f32)) -> [f32; 4] {
        [win.0 * 0.3, win.1 * 0.15, win.0 * 0.4, 160.0]
    }

    /// The card under a window point, topmost first, and whether the point is
    /// in its strip.
    fn card_at(&self, p: (f32, f32)) -> Option<(usize, bool)> {
        self.cards.iter().enumerate().rev().find_map(|(i, c)| {
            let r = self.screen_rect(c);
            gesture::inside(&r, p).then_some((i, p.1 < r[1] + STRIP))
        })
    }

    /// The canvas zoom about a pointer: the world point under it stays put.
    fn zoom_at(&mut self, factor: f32, p: (f32, f32)) {
        let z0 = self.zoom;
        let z1 = (z0 * factor).clamp(0.3, 4.0);
        let w = (self.pan.0 + p.0 / z0, self.pan.1 + p.1 / z0);
        self.zoom = z1;
        self.pan = (w.0 - p.0 / z1, w.1 - p.1 / z1);
        log(&format!("canvas: zoom {z0:.3} -> {z1:.3} about {p:?}"));
        self.dirty = true;
    }

    /// A pointer press on the canvas (gpui's own event, however it got here).
    fn press_at(&mut self, p: (f32, f32), left: bool, cmd: bool) {
        self.pointer = p;
        if !left || cmd {
            self.press = Some(Press::Pan(p));
            log(&format!("gpui: pan starts at {p:?} ({})", if left { "cmd+left" } else { "middle" }));
        } else if let Some((i, strip)) = self.card_at(p) {
            let id = self.cards[i].id.clone();
            self.selected = Some(id.clone());
            if strip {
                let w = &self.cards[i].world;
                let grab = (p.0 / self.zoom + self.pan.0 - w[0], p.1 / self.zoom + self.pan.1 - w[1]);
                self.press = Some(Press::Card(id.clone(), grab));
                log(&format!("gpui: card {id} picked up by its strip"));
            } else {
                log(&format!("gpui: click on card {id}"));
            }
        } else {
            log(&format!("gpui: click on empty canvas at {p:?}"));
        }
        self.dirty = true;
    }

    fn press_move(&mut self, p: (f32, f32)) {
        self.pointer = p;
        let (zoom, pan) = (self.zoom, self.pan);
        match &mut self.press {
            Some(Press::Pan(last)) => {
                self.pan = (pan.0 - (p.0 - last.0) / zoom, pan.1 - (p.1 - last.1) / zoom);
                *last = p;
                self.dirty = true;
            }
            Some(Press::Card(id, grab)) => {
                let id = id.clone();
                let nx = p.0 / zoom + pan.0 - grab.0;
                let ny = p.1 / zoom + pan.1 - grab.1;
                // The ghost is the slot the drop would take: a 20 unit grid.
                let snap = |v: f32| (v / 20.0).round() * 20.0;
                // A drag moves a ghost and the drop decides (CLAUDE.md): the
                // card itself stays where it is until the button goes up.
                let mut ghost = None;
                if let Some(c) = self.card_mut(&id) {
                    ghost = Some([snap(nx), snap(ny), c.world[2], c.world[3]]);
                }
                self.ghost = ghost.map(|g| {
                    [(g[0] - pan.0) * zoom, (g[1] - pan.1) * zoom, g[2] * zoom, g[3] * zoom]
                });
                self.dirty = true;
            }
            None => {}
        }
    }

    fn press_end(&mut self) {
        if let Some(Press::Card(id, _)) = self.press.take() {
            // The drop takes the ghost's slot (screen rect back to world).
            let (zoom, pan, ghost) = (self.zoom, self.pan, self.ghost);
            if let (Some(g), Some(c)) = (ghost, self.card_mut(&id)) {
                c.world[0] = g[0] / zoom + pan.0;
                c.world[1] = g[1] / zoom + pan.1;
            }
            log(&format!("gpui: card {id} dropped"));
        } else {
            self.press = None;
        }
        self.ghost = None;
        self.dirty = true;
    }

    /// The spike's keymap: Cmd+T a terminal-like card, Cmd+W close the
    /// selected card, Cmd+Shift+P the palette panel.
    fn chord(&mut self, key: &str, shift: bool) -> bool {
        match (key, shift) {
            ("t", false) => {
                let n = self.cards.len();
                let id = format!("t{n}");
                self.cards.push(Card {
                    id: id.clone(),
                    world: [600.0 + 30.0 * n as f32, 420.0, 300.0, 180.0],
                    kind: Kind::Plain { color: 0x2a6b4f },
                    dim: 1.0,
                    snapshot: None,
                    wv: None,
                });
                self.selected = Some(id.clone());
                log(&format!("gpui chord: Cmd+T new card {id}"));
            }
            ("w", false) => {
                if let Some(id) = self.selected.take() {
                    self.cards.retain(|c| c.id != id);
                    log(&format!("gpui chord: Cmd+W closed card {id}"));
                } else {
                    log("gpui chord: Cmd+W with no selected card");
                }
            }
            ("p", true) => {
                self.palette = !self.palette;
                log(&format!("gpui chord: Cmd+Shift+P palette {}", self.palette));
            }
            _ => return false,
        }
        self.dirty = true;
        true
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
                    self.page_rect(&tmp)
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
            Cmd::InactiveDim(on) => self.inactive_dim = on,
            Cmd::Select(id) => self.selected = Some(id),
            Cmd::Pinch(d) => {
                // The same function the monitor calls for a real pinch.
                let p = self.pointer;
                self.zoom_at(1.0 + d, p);
            }
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
        let mut s = format!(
            "{}|{:?}|{}|{}|{}|{:?}|{}|{:?}|{}",
            self.zoom, self.pan, self.zoom_mode, self.cover, view_h, self.ghost, self.palette,
            self.selected, self.inactive_dim
        );
        for c in &self.cards {
            s += &format!("|{}:{:?}:{}:{}", c.id, c.world, c.dim, c.snapshot.is_some());
        }
        s
    }

    fn sync_native(&mut self, view_h: f32, win_w: f32) {
        let sig = self.native_signature(view_h);
        if sig == self.last_sig {
            return;
        }
        self.last_sig = sig;
        let rects: Vec<[f32; 4]> = self.cards.iter().map(|c| self.screen_rect(c)).collect();
        // What gpui draws over the pages besides the cards in front: the drag
        // ghost and the palette panel.
        let mut extra: Vec<[f32; 4]> = self.ghost.into_iter().collect();
        if self.palette {
            extra.push(self.palette_rect((win_w, view_h)));
        }
        // Published for the input monitor (gesture.rs).
        {
            let mut lay = gesture::Layout::default();
            for (i, c) in self.cards.iter().enumerate() {
                if c.wv.is_some() {
                    lay.web.push(self.page_rect(c));
                    lay.cover.extend(rects[i + 1..].iter().cloned());
                }
            }
            lay.cover.extend(extra.iter().cloned());
            *LAYOUT.lock().unwrap() = (lay, view_h);
        }
        let zoom = self.zoom;
        let mode = self.zoom_mode.clone();
        let cover = self.cover.clone();
        for i in 0..self.cards.len() {
            let c = &self.cards[i];
            let Some(wv) = c.wv.as_ref() else { continue };
            let r = self.page_rect(c);
            let dim = self.eff_dim(c);
            // Cards later in the vec are drawn over this one.
            let holes: Vec<[f32; 4]> = rects[i + 1..]
                .iter()
                .chain(extra.iter())
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
                // A translucent colour view above the webview dims it (the
                // webview's own alpha does nothing, see the report). It gets
                // the same mask, or it would wash the cards drawn in front.
                let dsup: *mut AnyObject = msg_send![&*view, superview];
                let dk = format!("dim-{}", c.id);
                let want_dim = dim < 1.0;
                if want_dim && !self.shields.contains_key(&dk) {
                    let dv: Retained<AnyObject> = msg_send![msg_send![class!(NSView), alloc], initWithFrame: NSRect::ZERO];
                    let _: () = msg_send![&*dv, setWantsLayer: Bool::YES];
                    let _: () = msg_send![dsup, addSubview: &*dv, positioned: 1isize, relativeTo: &*view];
                    self.shields.insert(dk.clone(), dv);
                }
                if let Some(dv) = self.shields.get(&dk) {
                    let _: () = msg_send![&**dv, setHidden: Bool::new(!want_dim)];
                    let fr: NSRect = msg_send![&*view, frame];
                    let _: () = msg_send![&**dv, setFrame: fr];
                    let dl: *mut AnyObject = msg_send![&**dv, layer];
                    let col: *mut AnyObject = msg_send![class!(NSColor), colorWithSRGBRed: 0.106f64, green: 0.114f64, blue: 0.133f64, alpha: (1.0 - dim) as f64];
                    let cg: *mut AnyObject = msg_send![col, CGColor];
                    let _: () = msg_send![dl, setBackgroundColor: cg];
                    apply_mask(dv, r, if cover == "hole" { &holes } else { &[] });
                }
                apply_mask(&view, r, if cover == "hole" { &holes } else { &[] });
                // A layer mask does not change hit testing, so a click in a
                // hole would still reach the page. The input monitor
                // (gesture.rs) routes it to gpui instead; there are no
                // shield views any more.
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
    for ho in &gesture::disjoint(holes) {
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
        unsafe { route_event(ev) }
    });
    // Key down, scroll, magnify and the left and other mouse events.
    let mask: usize = (1 << 10) | (1 << 22) | (1 << 30) | (1 << 1) | (1 << 2) | (1 << 6) | (1 << 25) | (1 << 26) | (1 << 27);
    unsafe {
        let _: *mut AnyObject = msg_send![class!(NSEvent), addLocalMonitorForEventsMatchingMask: mask, handler: &*block];
    }
    std::mem::forget(block);
}

/// gpui's view answers `mouseDragged:` but never registers
/// `otherMouseDragged:`, so a middle-button drag moves nothing. The app
/// already gives the class that method (infiniterm-ui/src/middle_drag.rs);
/// the spike does the same so a middle drag forwarded by the monitor pans.
fn teach_middle_drags() {
    use objc2::ffi;
    unsafe {
        let Some(cls) = objc2::runtime::AnyClass::get(c"GPUIView") else { return };
        let cls_ptr = cls as *const _ as *mut ffi::objc_class;
        let from = objc2::sel!(mouseDragged:);
        let to = objc2::sel!(otherMouseDragged:);
        let m = ffi::class_getInstanceMethod(cls_ptr, from);
        if m.is_null() {
            return;
        }
        let Some(imp) = ffi::method_getImplementation(m) else { return };
        let added = ffi::class_addMethod(cls_ptr, to, imp, c"v@:@".as_ptr());
        log(&format!("middle drag method added to GPUIView: {added:?}"));
    }
}

/// The gpui view and what the monitor needs to know about the canvas.
static GPUI_VIEW: AtomicUsize = AtomicUsize::new(0);
static GPUI_DRAG: AtomicBool = AtomicBool::new(false);
static LAYOUT: Mutex<(gesture::Layout, f32)> = Mutex::new((gesture::Layout { web: Vec::new(), cover: Vec::new() }, 0.0));
/// Pinch deltas the monitor saw, with the pointer, for the next tick.
static MAGNIFY: Mutex<Vec<(f32, (f32, f32))>> = Mutex::new(Vec::new());

/// The monitor body. Returns the event to let AppKit have it, or null when
/// the event was handled here (given to gpui's view, or turned into a zoom).
unsafe fn route_event(ev: *mut AnyObject) -> *mut AnyObject {
    use gesture::{Kind, KeyAction, Target};
    let gv = GPUI_VIEW.load(Ordering::Relaxed) as *mut AnyObject;
    if gv.is_null() {
        return ev;
    }
    let w: *mut AnyObject = msg_send![ev, window];
    if !class_name(w).contains("GPUIWindow") {
        return ev;
    }
    let ty: usize = msg_send![ev, type];
    let flags: usize = msg_send![ev, modifierFlags];
    let mods = gesture::Mods { cmd: flags & (1 << 20) != 0, ctrl: flags & (1 << 18) != 0 };

    if ty == 10 {
        // Key down. Which view is first responder decides whether the page
        // is typing.
        let fr: *mut AnyObject = msg_send![w, firstResponder];
        let page_focused = class_name(fr).contains("WryWebView");
        let ch: *mut AnyObject = msg_send![ev, charactersIgnoringModifiers];
        let c: *const std::ffi::c_char = msg_send![ch, UTF8String];
        let s = std::ffi::CStr::from_ptr(c).to_string_lossy().to_lowercase();
        log(&format!("monitor: keyDown {s:?} cmd={} shift={} page_focused={page_focused}", mods.cmd, flags & (1 << 17) != 0));
        return match gesture::key_route(mods.cmd, &s, page_focused) {
            KeyAction::Pass => ev,
            KeyAction::PageAction(name) => {
                // The page's own editing chords: the action goes down the
                // responder chain to the focused WKWebView.
                let sel = objc2::runtime::Sel::register(&std::ffi::CString::new(name).unwrap());
                let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
                let ok: Bool = msg_send![app, sendAction: sel, to: std::ptr::null::<AnyObject>(), from: std::ptr::null::<AnyObject>()];
                log(&format!("monitor: {name} sent to the page, handled={}", ok.as_bool()));
                std::ptr::null_mut()
            }
            KeyAction::TryGpui => {
                let handled: Bool = msg_send![gv, performKeyEquivalent: ev];
                log(&format!("monitor: chord offered to gpui, handled={}", handled.as_bool()));
                if handled.as_bool() { std::ptr::null_mut() } else { ev }
            }
        };
    }

    let (kind, sel) = match ty {
        22 => (Kind::Scroll, objc2::sel!(scrollWheel:)),
        30 => (Kind::Magnify, objc2::sel!(magnifyWithEvent:)),
        1 => (Kind::LeftDown, objc2::sel!(mouseDown:)),
        2 => (Kind::LeftUp, objc2::sel!(mouseUp:)),
        6 => (Kind::LeftDragged, objc2::sel!(mouseDragged:)),
        25 => (Kind::OtherDown, objc2::sel!(otherMouseDown:)),
        26 => (Kind::OtherUp, objc2::sel!(otherMouseUp:)),
        27 => (Kind::OtherDragged, objc2::sel!(otherMouseDragged:)),
        _ => return ev,
    };
    let loc: NSPoint = msg_send![ev, locationInWindow];
    let (layout, view_h) = {
        let g = LAYOUT.lock().unwrap();
        (g.0.clone(), g.1)
    };
    let p = (loc.x as f32, view_h - loc.y as f32);
    let owns = GPUI_DRAG.load(Ordering::Relaxed);
    let target = gesture::route(kind, mods, p, &layout, owns);
    if target == Target::Page && !owns {
        return ev;
    }
    log(&format!("monitor: {kind:?} at {p:?} -> {target:?}"));
    if gesture::starts_gpui_drag(kind, target) {
        GPUI_DRAG.store(true, Ordering::Relaxed);
    }
    if matches!(kind, Kind::LeftUp | Kind::OtherUp) {
        GPUI_DRAG.store(false, Ordering::Relaxed);
    }
    match target {
        Target::Gpui => {
            let _: () = msg_send![gv, performSelector: sel, withObject: ev];
            std::ptr::null_mut()
        }
        Target::Canvas => {
            let d: f64 = msg_send![ev, magnification];
            MAGNIFY.lock().unwrap().push((d as f32, p));
            std::ptr::null_mut()
        }
        Target::Page => ev,
    }
}

impl Render for Spike {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .size_full()
            .bg(rgb(0x1b1d22))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| {
                let m = &ev.keystroke.modifiers;
                log(&format!(
                    "gpui: keyDown {:?} cmd={} shift={}",
                    ev.keystroke.key, m.platform, m.shift
                ));
                if m.platform && this.chord(ev.keystroke.key.as_str(), m.shift) {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, ev: &gpui::MouseDownEvent, _w, cx| {
                let p = (f32::from(ev.position.x), f32::from(ev.position.y));
                log(&format!("gpui: mouseDown left at {p:?} cmd={}", ev.modifiers.platform));
                this.press_at(p, true, ev.modifiers.platform);
                cx.notify();
            }))
            .on_mouse_down(gpui::MouseButton::Middle, cx.listener(|this, ev: &gpui::MouseDownEvent, _w, cx| {
                let p = (f32::from(ev.position.x), f32::from(ev.position.y));
                log(&format!("gpui: mouseDown middle at {p:?}"));
                this.press_at(p, false, false);
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, _w, cx| {
                let p = (f32::from(ev.position.x), f32::from(ev.position.y));
                if this.press.is_some() {
                    this.press_move(p);
                    cx.notify();
                } else {
                    this.pointer = p;
                }
            }))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(|this, _ev: &gpui::MouseUpEvent, _w, cx| {
                log("gpui: mouseUp left");
                this.press_end();
                cx.notify();
            }))
            .on_mouse_up(gpui::MouseButton::Middle, cx.listener(|this, _ev: &gpui::MouseUpEvent, _w, cx| {
                log("gpui: mouseUp middle");
                this.press_end();
                cx.notify();
            }))
            .on_scroll_wheel(cx.listener(|this, ev: &gpui::ScrollWheelEvent, _w, cx| {
                let p = (f32::from(ev.position.x), f32::from(ev.position.y));
                let d = ev.delta.pixel_delta(px(20.0));
                let dy = f32::from(d.y);
                log(&format!("gpui: scroll dy={dy} cmd={} ctrl={} at {p:?}", ev.modifiers.platform, ev.modifiers.control));
                if ev.modifiers.platform || ev.modifiers.control {
                    this.zoom_at((dy / 200.0).exp(), p);
                    cx.notify();
                }
            }))
            .relative();
        for c in &self.cards {
            let r = self.screen_rect(c);
            let (bg, label) = match &c.kind {
                Kind::Plain { color } => (*color, format!("plain {}", c.id)),
                Kind::Web { url } => (0x303844, format!("web {} {url}", c.id)),
            };
            let selected = self.selected.as_deref() == Some(c.id.as_str());
            let mut d = div()
                .absolute()
                .left(px(r[0]))
                .top(px(r[1]))
                .w(px(r[2]))
                .h(px(r[3]))
                .bg(rgb(bg))
                .border_1()
                .border_color(rgb(if selected { 0xe0a030 } else { 0x8a93a3 }))
                .text_color(rgb(0xffffff))
                .text_size(px(13.0))
                .child(div().h(px(STRIP - 2.0)).px_2().bg(rgb(0x20242b)).child(label));
            let dim = self.eff_dim(c);
            if matches!(c.kind, Kind::Plain { .. }) {
                d = d.opacity(dim);
            }
            if let Some(s) = &c.snapshot {
                d = d.child(
                    div().absolute().top(px(STRIP - 1.0)).left_0().right_0().bottom_0().child(
                        img(s.clone()).size_full().object_fit(ObjectFit::Fill),
                    ),
                );
            }
            root = root.child(d);
        }
        if let Some(g) = self.ghost {
            root = root.child(
                div()
                    .absolute()
                    .left(px(g[0]))
                    .top(px(g[1]))
                    .w(px(g[2]))
                    .h(px(g[3]))
                    .bg(gpui::rgba(0xe0a03055))
                    .border_2()
                    .border_color(rgb(0xe0a030)),
            );
        }
        if self.palette {
            let r = self.palette_rect(self.win);
            root = root.child(
                div()
                    .absolute()
                    .left(px(r[0]))
                    .top(px(r[1]))
                    .w(px(r[2]))
                    .h(px(r[3]))
                    .bg(rgb(0x14161a))
                    .border_1()
                    .border_color(rgb(0xe0a030))
                    .text_color(rgb(0xffffff))
                    .p_2()
                    .child("palette (gpui panel over the pages)"),
            );
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
                        win: (1100.0, 760.0),
                        selected: None,
                        inactive_dim: false,
                        press: None,
                        ghost: None,
                        palette: false,
                        pointer: (0.0, 0.0),
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
        teach_middle_drags();
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
                        let vw: f32 = window.viewport_size().width.into();
                        this.win = (vw, vh);
                        if GPUI_VIEW.load(Ordering::Relaxed) == 0 {
                            if let Some(v) = native_view(window) {
                                GPUI_VIEW.store(Retained::as_ptr(&v) as usize, Ordering::Relaxed);
                            }
                        }
                        // A pinch the monitor caught: the same zoom path.
                        let pinches: Vec<_> = std::mem::take(&mut *MAGNIFY.lock().unwrap());
                        for (d, p) in pinches {
                            log(&format!("monitor: pinch {d:.3} at {p:?} turned into a canvas zoom"));
                            this.zoom_at(1.0 + d, p);
                        }
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
                                this.sync_native(vh, vw);
                                let mut costs = costs;
                                costs.push(s.elapsed().as_secs_f32() * 1000.0);
                                this.anim = Some((t0, ms, costs));
                                this.dirty = true;
                            }
                        } else {
                            this.sync_native(vh, vw);
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
