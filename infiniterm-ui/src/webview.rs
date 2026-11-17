//! The system web view as a browser card's page (`browser.engine` =
//! `webview`, macOS only, #761): a WKWebView made with the `wry` crate as a
//! subview of gpui's own view, placed over the card's page area every frame.
//!
//! How it differs from the CEF page: CEF paints into a texture gpui draws, so
//! the page zooms, overlaps and dims like any card. A native view floats ABOVE
//! everything gpui draws, so this engine (1) follows the card's rect and gives
//! WebKit `pageZoom` for the canvas zoom, (2) swaps itself for a still picture
//! (a snapshot, drawn by gpui through the same texture path CEF frames take)
//! whenever the page cannot be shown live: far zoom, a card over it, a card
//! being dragged, and (3) dims through a colour view above it, because the
//! web view's own alpha does nothing. The rules for those choices are
//! `infiniterm_core::webview_policy`; this file carries them out.
//!
//! The snapshot is kept fresh while the page is live (`snapshot_due`), because
//! it is asked for asynchronously and has to exist BEFORE the cover appears.
//!
//! Where it fits: `browser_body.rs` builds a `WebSurface` for a tab when the
//! card's engine is `Engine::Webview`, calls `place` from `paint`, and takes
//! the snapshot as the tab's texture. Created lazily in `place`, because
//! wry needs the window and a tab is made before one is at hand.
//! Related: `page_surface.rs` (the trait), the spike in PR #812 and its
//! report for the measurements.
//! Constraints: main thread only (AppKit and wry both require it); the
//! per-instance data store is mandatory, never the global default store.
use crate::page_surface::{PageSurface, Placement};
use block2::RcBlock;
use gpui::Window;
use infiniterm_browser::{Button, ContextMenuRequest, Frame, Mods};
use infiniterm_core::webview_policy as policy;
use objc2::{
    class, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool},
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    sync::{Arc, Mutex},
};
use wry::{
    dpi::{LogicalPosition, LogicalSize},
    NewWindowResponse, Rect, WebView, WebViewBuilder, WebViewBuilderExtDarwin, WebViewExtMacOS,
};

/// What the page's handlers write and the card reads: plain data, behind an
/// `Rc` because wry's closures outlive any borrow of the surface.
#[derive(Default)]
struct Shared {
    title: Option<String>,
    popups: Vec<String>,
    /// A snapshot that arrived and was not taken yet (PNG bytes).
    snapshot_png: Option<Vec<u8>>,
    snapshot_pending: bool,
    /// When (frame clock, ms) the last load finished.
    loaded_at: Option<f64>,
    /// Something changed that `sync` has to hear about on a frame it would
    /// not otherwise paint (a title, a finished load, a snapshot).
    dirty: bool,
    /// The newest frame clock `place` saw, so a handler can stamp events.
    clock: f64,
    /// Links to another app the page tried to open (#668), not yet taken.
    external_links: Vec<String>,
    /// The page called `window.close()` (#672).
    close_requested: bool,
    /// What a finished download says, not yet taken, and the name the one in
    /// flight was given.
    downloads: Vec<String>,
    download_name: Option<String>,
    /// Find in page: the count of matches, which one is current.
    find_count: i32,
    find_active: i32,
    /// What a right-click found under the point, and where (world pixels).
    menu: Option<(policy::ContextInfo, (f64, f64))>,
}

struct State {
    url: String,
    store: [u8; 16],
    /// `browser.devtools`: the page is inspectable from Safari.
    devtools: bool,
    view: Option<WebView>,
    dimmer: Option<Retained<AnyObject>>,
    placed: bool,
    last: Option<Placement>,
    world: (i32, i32),
    last_snapshot_ms: Option<f64>,
    /// The layout size the newest snapshot was taken at.
    snapshot_world: (i32, i32),
    /// The page's own zoom (`card.zoom`), to turn a click's world pixels into
    /// the CSS pixels the page's scripts use.
    own_zoom: f64,
}

/// A script's answer. WebKit calls the script callback on the main thread,
/// but wry asks it to be `Send`, so it cannot hold the `Rc` state: it leaves
/// the raw answer here and `drain_inbox` applies it on the next frame.
enum Inbox {
    FindCount(String),
    Menu(String, (f64, f64)),
    /// The link under a Cmd or middle click (#821).
    Link(String),
}

struct Core {
    /// The key `webview_input` knows this page by.
    key: u64,
    inbox: Arc<Mutex<Vec<Inbox>>>,
    shared: Rc<RefCell<Shared>>,
    state: RefCell<State>,
}

pub struct WebSurface {
    core: Rc<Core>,
}

thread_local! {
    /// Every live web page, for `end_frame` to hide the ones nobody placed.
    static LIVE: RefCell<Vec<Weak<Core>>> = const { RefCell::new(Vec::new()) };
}

pub fn begin_frame() {
    crate::webview_input::begin_frame();
    LIVE.with(|l| {
        l.borrow_mut().retain(|w| w.upgrade().is_some());
        for c in l.borrow().iter().filter_map(Weak::upgrade) {
            c.state.borrow_mut().placed = false;
        }
    });
}

pub fn end_frame() {
    LIVE.with(|l| {
        for c in l.borrow().iter().filter_map(Weak::upgrade) {
            let mut s = c.state.borrow_mut();
            if !s.placed {
                hide_native(&mut s);
            }
        }
    });
}

fn hide_native(s: &mut State) {
    if let Some(v) = &s.view {
        let _ = v.set_visible(false);
    }
    if let Some(d) = &s.dimmer {
        unsafe {
            let _: () = msg_send![&**d, setHidden: Bool::YES];
        }
    }
    // The next `place` must apply everything again.
    s.last = None;
}

/// macOS 14 introduced `dataStoreForIdentifier`; before it there is no way
/// to name a store, and the global default store must not be touched.
fn macos_14_or_later() -> bool {
    macos_major() >= 14
}

fn macos_major() -> i64 {
    objc2_foundation::NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion as i64
}

impl WebSurface {
    pub fn open(url: &str, store: [u8; 16], world: (i32, i32), devtools: bool) -> WebSurface {
        let core = Rc::new(Core {
            key: crate::webview_input::next_key(),
            inbox: Arc::new(Mutex::new(Vec::new())),
            shared: Rc::new(RefCell::new(Shared::default())),
            state: RefCell::new(State {
                url: url.to_string(),
                store,
                devtools,
                view: None,
                dimmer: None,
                placed: false,
                last: None,
                world,
                last_snapshot_ms: None,
                snapshot_world: world,
                own_zoom: 1.,
            }),
        });
        LIVE.with(|l| l.borrow_mut().push(Rc::downgrade(&core)));
        WebSurface { core }
    }

    /// Applies the script answers that arrived since last frame.
    fn drain_inbox(&self) {
        let items = std::mem::take(&mut *self.core.inbox.lock().unwrap());
        if items.is_empty() {
            return;
        }
        if infiniterm_browser::surface::keylog() {
            for item in &items {
                match item {
                    Inbox::FindCount(raw) => eprintln!("[webview] find count answer {raw:?}"),
                    Inbox::Menu(raw, _) => eprintln!("[webview] context answer {raw:?}"),
                    Inbox::Link(raw) => eprintln!("[webview] link answer {raw:?}"),
                }
            }
        }
        let mut s = self.core.shared.borrow_mut();
        for item in items {
            match item {
                Inbox::FindCount(raw) => {
                    s.find_count = policy::parse_count(&raw);
                    s.find_active = if s.find_count > 0 { 1 } else { 0 };
                }
                Inbox::Menu(raw, world) => {
                    s.menu = policy::parse_context(&raw).map(|info| (info, world));
                }
                Inbox::Link(raw) => match policy::parse_link(&raw)
                    .as_deref()
                    .map(policy::classify_link)
                {
                    Some(policy::LinkOpen::Tab(url)) => s.popups.push(url),
                    Some(policy::LinkOpen::App(url)) => s.external_links.push(url),
                    Some(policy::LinkOpen::Ignore) | None => {}
                },
            }
        }
        s.dirty = true;
    }

    fn with_view<R>(&self, f: impl FnOnce(&WebView) -> R) -> Option<R> {
        self.core.state.borrow().view.as_ref().map(f)
    }

    fn build(&self, window: &Window, p: &Placement) {
        let (url, store, devtools) = {
            let s = self.core.state.borrow();
            (s.url.clone(), s.store, s.devtools)
        };
        let sh = self.core.shared.clone();
        let sh_title = sh.clone();
        let sh_load = sh.clone();
        let sh_popup = sh.clone();
        let sh_nav = sh.clone();
        let sh_ipc = sh.clone();
        let sh_dl = sh.clone();
        let sh_done = sh.clone();
        let mut b = WebViewBuilder::new()
            .with_bounds(Rect {
                position: LogicalPosition::new(p.rect[0], p.rect[1]).into(),
                size: LogicalSize::new(p.rect[2].max(1.), p.rect[3].max(1.)).into(),
            })
            .with_url(&url)
            .with_accept_first_mouse(true)
            .with_document_title_changed_handler(move |t| {
                let mut s = sh_title.borrow_mut();
                s.title = Some(t);
                s.dirty = true;
            })
            .with_on_page_load_handler(move |e, _url| {
                let mut s = sh_load.borrow_mut();
                if matches!(e, wry::PageLoadEvent::Finished) {
                    s.loaded_at = Some(s.clock);
                }
                s.dirty = true;
            })
            // A page's `window.open`: the card opens a tab for it (#672), the
            // web view itself opens nothing.
            .with_new_window_req_handler(move |u, _features| {
                let mut s = sh_popup.borrow_mut();
                // `window.open("slack://...")` is no tab (#668).
                if infiniterm_core::app_link::is_app_link(&u) {
                    s.external_links.push(u);
                } else {
                    s.popups.push(u);
                }
                s.dirty = true;
                NewWindowResponse::Deny
            })
            // A navigation to another app's scheme (`slack:`, `mailto:`),
            // from a click or a redirect, never commits: the page stays and
            // the card asks (#668).
            .with_navigation_handler(move |u| {
                if infiniterm_core::app_link::is_app_link(&u) {
                    let mut s = sh_nav.borrow_mut();
                    s.external_links.push(u);
                    s.dirty = true;
                    false
                } else {
                    true
                }
            })
            // `window.close()`: WebKit gives wry no callback, so the page's
            // own function posts a message (#672).
            .with_initialization_script(policy::CLOSE_SCRIPT)
            .with_ipc_handler(move |req| {
                if req.body() == policy::CLOSE_MESSAGE {
                    let mut s = sh_ipc.borrow_mut();
                    s.close_requested = true;
                    s.dirty = true;
                }
            })
            // Downloads go to the folder WebKit proposed (Downloads), under
            // its name, never over a file that is there; the card says so.
            .with_download_started_handler(move |_url, dest| {
                let name = dest
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("download")
                    .to_string();
                // `INFINITERM_DOWNLOAD_DIR` moves downloads for a scratch
                // instance (a test must not write into the real Downloads
                // folder, and macOS asks about that folder).
                let dir = std::env::var_os("INFINITERM_DOWNLOAD_DIR")
                    .map(std::path::PathBuf::from)
                    .or_else(|| dest.parent().map(std::path::Path::to_path_buf))
                    .unwrap_or_default();
                *dest = policy::download_destination(&dir, &name, |p| p.exists());
                sh_dl.borrow_mut().download_name = dest
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string);
                true
            })
            .with_download_completed_handler(move |_url, _path, ok| {
                let mut s = sh_done.borrow_mut();
                let name = s.download_name.take().unwrap_or_else(|| "a file".into());
                s.downloads.push(if ok {
                    format!("saved {name} in Downloads")
                } else {
                    format!("could not save {name}")
                });
                s.dirty = true;
            });
        // Safari's user agent, so a site that reads the tail sees Safari.
        b = b.with_user_agent(policy::safari_user_agent(macos_major()));
        if devtools {
            // Inspectable from Safari's Develop menu (`NATIVE_DEVTOOLS_NOTE`).
            b = b.with_devtools(true);
        }
        if macos_14_or_later() {
            b = b.with_data_store_identifier(store);
        } else {
            // One shared store that is never written to disk: NOT the global
            // default store, which a scratch instance must not touch.
            eprintln!("[infiniterm] {}", policy::NO_CUSTOM_STORE_NOTE);
            b = b.with_incognito(true);
        }
        match b.build_as_child(window) {
            Ok(wv) => {
                // The input monitor and gpui's own view (the superview of
                // every web view), which it forwards the canvas's events to.
                unsafe {
                    let view = wv.webview();
                    let gpui_view: *mut AnyObject = msg_send![&*view, superview];
                    crate::webview_input::set_gpui_view(gpui_view);
                }
                crate::webview_input::install();
                self.core.state.borrow_mut().view = Some(wv);
            }
            Err(e) => eprintln!("[infiniterm] webview: could not create the page: {e}"),
        }
    }

    fn take_snapshot(&self) {
        let sh = self.core.shared.clone();
        let Some(()) = self.with_view(|wv| {
            let block = RcBlock::new(move |image: *mut AnyObject, _err: *mut AnyObject| unsafe {
                let mut s = sh.borrow_mut();
                s.snapshot_pending = false;
                if image.is_null() {
                    return;
                }
                let tiff: *mut AnyObject = msg_send![image, TIFFRepresentation];
                let rep: *mut AnyObject = msg_send![class!(NSBitmapImageRep), imageRepWithData: tiff];
                // 4 is NSBitmapImageFileTypePNG.
                let png: *mut AnyObject = msg_send![rep, representationUsingType: 4usize, properties: std::ptr::null::<AnyObject>()];
                if png.is_null() {
                    return;
                }
                let len: usize = msg_send![png, length];
                let bytes: *const u8 = msg_send![png, bytes];
                s.snapshot_png = Some(std::slice::from_raw_parts(bytes, len).to_vec());
                s.dirty = true;
            });
            let view = wv.webview();
            unsafe {
                let _: () = msg_send![&*view, takeSnapshotWithConfiguration: std::ptr::null::<AnyObject>(), completionHandler: &*block];
            }
        }) else {
            return;
        };
        let mut s = self.core.state.borrow_mut();
        s.last_snapshot_ms = Some(self.core.shared.borrow().clock);
        s.snapshot_world = s.world;
        self.core.shared.borrow_mut().snapshot_pending = true;
    }
}

/// PNG bytes to a BGRA frame, the byte order gpui keeps images in (the same
/// as CEF's frames, so the card draws both alike).
pub fn png_to_frame(png: &[u8], css: (i32, i32)) -> Option<Frame> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let (width, height) = img.dimensions();
    let mut bgra = img.into_raw();
    for px in bgra.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    Some(Frame {
        width,
        height,
        bgra,
        css_width: css.0,
        css_height: css.1,
    })
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPathCreateMutable() -> *mut std::ffi::c_void;
    fn CGPathAddRect(p: *mut std::ffi::c_void, t: *const std::ffi::c_void, r: NSRect);
    fn CGPathRelease(p: *mut std::ffi::c_void);
}

/// A CAShapeLayer mask on a view's layer: the whole view minus the holes
/// (even-odd, so the holes must not overlap: `holes_of` guarantees it). No
/// holes removes the mask. A mask changes what is DRAWN only; hit testing is
/// `webview_input`'s job.
unsafe fn apply_mask(view: &AnyObject, rect: [f64; 4], holes: &[[f64; 4]]) {
    let layer: *mut AnyObject = msg_send![view, layer];
    if layer.is_null() {
        return;
    }
    if holes.is_empty() {
        let _: () = msg_send![layer, setMask: std::ptr::null::<AnyObject>()];
        return;
    }
    let flipped: Bool = msg_send![view, isFlipped];
    let (w, h) = (rect[2], rect[3]);
    let path = CGPathCreateMutable();
    CGPathAddRect(
        path,
        std::ptr::null(),
        NSRect::new(NSPoint::new(0., 0.), NSSize::new(w, h)),
    );
    for ho in holes {
        let x = ho[0] - rect[0];
        let mut y = ho[1] - rect[1];
        // AppKit layers are bottom-left unless the view is flipped.
        if !flipped.as_bool() {
            y = h - y - ho[3];
        }
        CGPathAddRect(
            path,
            std::ptr::null(),
            NSRect::new(NSPoint::new(x, y), NSSize::new(ho[2], ho[3])),
        );
    }
    let mask: *mut AnyObject = msg_send![class!(CAShapeLayer), layer];
    let _: () = msg_send![mask, setFrame: NSRect::new(NSPoint::new(0., 0.), NSSize::new(w, h))];
    let rule = NSString::from_str("even-odd");
    let _: () = msg_send![mask, setFillRule: &*rule];
    let _: () = msg_send![mask, setPath: path];
    let _: () = msg_send![layer, setMask: mask];
    CGPathRelease(path);
}

unsafe fn rgba_color(rgb: [f32; 3], alpha: f64) -> *mut AnyObject {
    let col: *mut AnyObject = msg_send![class!(NSColor), colorWithSRGBRed: rgb[0] as f64, green: rgb[1] as f64, blue: rgb[2] as f64, alpha: alpha];
    msg_send![col, CGColor]
}

impl PageSurface for WebSurface {
    fn navigate(&self, url: &str) {
        self.core.state.borrow_mut().url = url.to_string();
        let _ = self.with_view(|wv| wv.load_url(url));
    }
    fn back(&self) {
        let _ = self.with_view(|wv| unsafe {
            let v = wv.webview();
            let _: *mut AnyObject = msg_send![&*v, goBack];
        });
    }
    fn forward(&self) {
        let _ = self.with_view(|wv| unsafe {
            let v = wv.webview();
            let _: *mut AnyObject = msg_send![&*v, goForward];
        });
    }
    fn can_go_back(&self) -> bool {
        self.with_view(|wv| unsafe {
            let v = wv.webview();
            let b: Bool = msg_send![&*v, canGoBack];
            b.as_bool()
        })
        .unwrap_or(false)
    }
    fn can_go_forward(&self) -> bool {
        self.with_view(|wv| unsafe {
            let v = wv.webview();
            let b: Bool = msg_send![&*v, canGoForward];
            b.as_bool()
        })
        .unwrap_or(false)
    }
    fn reload(&self) {
        let _ = self.with_view(|wv| unsafe {
            let v = wv.webview();
            let _: *mut AnyObject = msg_send![&*v, reload];
        });
    }
    // Step 3 of the plan (page features): find, the right-click menu, app
    // links and downloads. Until then these do nothing.
    // Find in page: WebKit selects and scrolls to the match
    // (`findString:withConfiguration:completionHandler:`), but says only
    // whether there was one, so the count the bar shows comes from a script
    // and the current match is stepped here (`policy::step_find`).
    fn find(&self, text: &str, forward: bool, next: bool) {
        let sh = self.core.shared.clone();
        let sh_found = self.core.shared.clone();
        let _ = self.with_view(|wv| {
            let block = RcBlock::new(move |res: *mut AnyObject| unsafe {
                // SAFETY: WebKit passes a live WKFindResult (or nil, handled)
                // on the main thread; `matchFound` is a BOOL property.
                let found: Bool = if res.is_null() {
                    Bool::NO
                } else {
                    msg_send![res, matchFound]
                };
                let mut s = sh_found.borrow_mut();
                if !found.as_bool() {
                    s.find_count = 0;
                    s.find_active = 0;
                }
                s.dirty = true;
            });
            let needle = NSString::from_str(text);
            let view = wv.webview();
            // SAFETY: a WKFindConfiguration made with `new`, owned by the
            // call; the block lives until WebKit has run it (RcBlock copies
            // it to the heap).
            unsafe {
                let cfg: *mut AnyObject = msg_send![class!(WKFindConfiguration), new];
                let _: () = msg_send![cfg, setWraps: Bool::YES];
                let _: () = msg_send![cfg, setBackwards: Bool::new(!forward)];
                let _: () = msg_send![&*view, findString: &*needle, withConfiguration: cfg, completionHandler: &*block];
            }
            // The count: the first search counts; stepping only moves.
            if !next {
                let inbox = self.core.inbox.clone();
                let _ = wv.evaluate_script_with_callback(&policy::find_count_js(text), move |raw| {
                    inbox.lock().unwrap().push(Inbox::FindCount(raw));
                });
            } else {
                let mut s = sh.borrow_mut();
                s.find_active = policy::step_find(s.find_active, s.find_count, forward);
            }
        });
    }
    fn stop_find(&self) {
        let _ = self.with_view(|wv| {
            let _ = wv
                .evaluate_script("window.getSelection && window.getSelection().removeAllRanges();");
        });
        let mut s = self.core.shared.borrow_mut();
        s.find_count = 0;
        s.find_active = 0;
    }
    fn find_result(&self) -> (i32, i32) {
        self.drain_inbox();
        let s = self.core.shared.borrow();
        (s.find_count, s.find_active)
    }
    fn device_scale(&self) -> f32 {
        1.
    }
    fn resize(&self, width: i32, height: i32, _scale: f32) {
        self.core.state.borrow_mut().world = (width, height);
    }
    fn take_popups(&self) -> Vec<String> {
        std::mem::take(&mut self.core.shared.borrow_mut().popups)
    }
    fn take_close_request(&self) -> bool {
        std::mem::take(&mut self.core.shared.borrow_mut().close_requested)
    }
    fn take_external_links(&self) -> Vec<String> {
        std::mem::take(&mut self.core.shared.borrow_mut().external_links)
    }
    fn take_downloads(&self) -> Vec<String> {
        std::mem::take(&mut self.core.shared.borrow_mut().downloads)
    }
    // A right-click, in two steps: the monitor swallowed it and left the
    // point (`webview_input::take_menu`); a script asks the page what is
    // there; its answer comes back on a later frame and becomes the request.
    fn take_context_menu(&self) -> Option<ContextMenuRequest> {
        self.drain_inbox();
        if let Some(at) = crate::webview_input::take_menu(self.core.key) {
            let inbox = self.core.inbox.clone();
            let world = at.world;
            let _ = self.with_view(|wv| {
                let _ = wv.evaluate_script_with_callback(
                    &policy::context_js(at.css.0, at.css.1),
                    move |raw| inbox.lock().unwrap().push(Inbox::Menu(raw, world)),
                );
            });
        }
        let (info, (x, y)) = self.core.shared.borrow_mut().menu.take()?;
        Some(ContextMenuRequest {
            x: x.round() as i32,
            y: y.round() as i32,
            link_url: info.link,
            editable: info.editable,
            has_selection: info.selection,
            can_go_back: self.can_go_back(),
            can_go_forward: self.can_go_forward(),
            page_url: self.url().unwrap_or_default(),
        })
    }
    fn title(&self) -> Option<String> {
        self.core.shared.borrow().title.clone()
    }
    fn url(&self) -> Option<String> {
        self.with_view(|wv| wv.url().ok()).flatten()
    }
    fn close(&self) {
        let mut s = self.core.state.borrow_mut();
        if let Some(d) = s.dimmer.take() {
            unsafe {
                let _: () = msg_send![&*d, removeFromSuperview];
            }
        }
        // Dropping the WebView removes its view from the window.
        s.view = None;
    }
    fn focus(&self, on: bool) {
        let _ = self.with_view(|wv| {
            let _ = if on { wv.focus() } else { wv.focus_parent() };
        });
    }
    fn edit_chord(&self, key: &str, shift: bool) -> bool {
        self.with_view(|wv| unsafe {
            let v = wv.webview();
            let nil = std::ptr::null::<AnyObject>();
            match (key, shift) {
                ("c", _) => {
                    let _: () = msg_send![&*v, copy: nil];
                }
                ("x", _) => {
                    let _: () = msg_send![&*v, cut: nil];
                }
                ("v", _) => {
                    let _: () = msg_send![&*v, paste: nil];
                }
                ("a", _) => {
                    let _: () = msg_send![&*v, selectAll: nil];
                }
                ("z", sh) => {
                    let um: *mut AnyObject = msg_send![&*v, undoManager];
                    if !um.is_null() {
                        if sh {
                            let _: () = msg_send![um, redo];
                        } else {
                            let _: () = msg_send![um, undo];
                        }
                    }
                }
                _ => return false,
            }
            true
        })
        .unwrap_or(false)
    }
    // The page's own zoom reaches WebKit inside `Placement::zoom` (times the
    // canvas zoom), because WebKit has one page zoom, not two; it is kept
    // here only to place a click on the page (`mouse_button`).
    fn set_zoom(&self, factor: f64) {
        self.core.state.borrow_mut().own_zoom = factor.max(0.1);
    }

    // A Cmd+click, Cmd+Shift+click or middle-click on a link opens a tab, as
    // in the Chromium card (#821). The press of such a click is the canvas's
    // (a Cmd or middle drag pans); `input.rs` delivers it here only when it
    // was released without moving, so this is the click. WebKit says a
    // navigation was a Cmd click only through its delegate, which wry owns
    // and does not forward, so the page is asked for the link at the point.
    // (x, y) are page pixels, the zoom of the canvas undone.
    fn mouse_button(&self, x: f32, y: f32, mods: Mods, b: Button, up: bool, _clicks: usize) {
        if up || !policy::opens_in_new_tab(mods.command, b == Button::Middle) {
            return;
        }
        let zoom = self.core.state.borrow().own_zoom;
        let js = policy::link_js(x as f64 / zoom, y as f64 / zoom);
        let inbox = self.core.inbox.clone();
        let _ = self.with_view(|wv| {
            let _ = wv.evaluate_script_with_callback(&js, move |raw| {
                inbox.lock().unwrap().push(Inbox::Link(raw))
            });
        });
    }

    fn has_new_frame(&self) -> bool {
        self.core.shared.borrow().dirty
            || !self.core.inbox.lock().unwrap().is_empty()
            || crate::webview_input::has_pending()
    }
    fn take_escapes(&self) -> Vec<f64> {
        crate::webview_input::take_escapes(self.core.key)
    }
    fn take_page_click(&self) -> bool {
        crate::webview_input::take_click(self.core.key)
    }
    fn take_frame(&self) -> Option<Arc<Frame>> {
        self.drain_inbox();
        let png = {
            let mut s = self.core.shared.borrow_mut();
            s.dirty = false;
            s.snapshot_png.take()?
        };
        let css = self.core.state.borrow().snapshot_world;
        png_to_frame(&png, css).map(Arc::new)
    }
    fn wants_frame(&self, now_ms: f64) -> bool {
        let s = self.core.state.borrow();
        let sh = self.core.shared.borrow();
        let live = s.last.as_ref().is_some_and(|p| !p.snapshot);
        policy::snapshot_due(
            live,
            sh.snapshot_pending,
            s.last_snapshot_ms,
            sh.loaded_at,
            now_ms,
        )
    }

    fn is_native(&self) -> bool {
        true
    }
    fn place(&self, window: &Window, p: &Placement) {
        if self.core.state.borrow().view.is_none() {
            self.build(window, p);
        }
        {
            let mut sh = self.core.shared.borrow_mut();
            sh.clock = p.now_ms;
        }
        {
            let mut s = self.core.state.borrow_mut();
            s.placed = true;
            s.world = p.world;
        }
        let changed = self.core.state.borrow().last.as_ref() != Some(p);
        if changed {
            let s = self.core.state.borrow();
            let Some(wv) = s.view.as_ref() else { return };
            let _ = wv.set_bounds(Rect {
                position: LogicalPosition::new(p.rect[0], p.rect[1]).into(),
                size: LogicalSize::new(p.rect[2].max(1.), p.rect[3].max(1.)).into(),
            });
            let _ = wv.zoom(p.zoom);
            let _ = wv.set_visible(!p.snapshot);
            let view = wv.webview();
            drop(s);
            // The dimmer: a colour view above the page, made on first use.
            let mut s = self.core.state.borrow_mut();
            unsafe {
                let want = p.dim > 0. && !p.snapshot;
                if want && s.dimmer.is_none() {
                    let sup: *mut AnyObject = msg_send![&*view, superview];
                    let dv: Retained<AnyObject> =
                        msg_send![msg_send![class!(NSView), alloc], initWithFrame: NSRect::ZERO];
                    let _: () = msg_send![&*dv, setWantsLayer: Bool::YES];
                    // 1 is NSWindowAbove.
                    let _: () =
                        msg_send![sup, addSubview: &*dv, positioned: 1isize, relativeTo: &*view];
                    s.dimmer = Some(dv);
                }
                if let Some(d) = &s.dimmer {
                    let _: () = msg_send![&**d, setHidden: Bool::new(!want)];
                    if want {
                        let fr: NSRect = msg_send![&*view, frame];
                        let _: () = msg_send![&**d, setFrame: fr];
                        let layer: *mut AnyObject = msg_send![&**d, layer];
                        let _: () =
                            msg_send![layer, setBackgroundColor: rgba_color(p.dim_rgb, p.dim)];
                    }
                }
            }
            // The holes: what gpui draws over the page is cut out of the
            // live view (and its wash), so the card in front, the drag ghost
            // or a panel shows through. The same cover list the click
            // routing uses (`webview_input`), so eye and hand agree.
            let holes = policy::holes_of(&p.rect, &p.covers);
            unsafe {
                apply_mask(&view, p.rect, &holes);
                if let Some(d) = &s.dimmer {
                    apply_mask(d, p.rect, &holes);
                }
            }
            s.last = Some(p.clone());
        }
        // Registered every frame (the table is cleared at its start): a page
        // shown as a snapshot takes no input.
        if !p.snapshot {
            crate::webview_input::register(
                self.core.key,
                infiniterm_core::webview_gesture::PageRoute {
                    rect: p.rect,
                    covers: p.covers.clone(),
                    focused: p.focused,
                },
                p.locked,
                p.zoom,
                p.canvas_scale,
            );
        }
        let (live, pending, last, loaded) = {
            let s = self.core.state.borrow();
            let sh = self.core.shared.borrow();
            (
                !p.snapshot,
                sh.snapshot_pending,
                s.last_snapshot_ms,
                sh.loaded_at,
            )
        };
        if policy::snapshot_due(live, pending, last, loaded, p.now_ms) {
            self.take_snapshot();
        }
    }
    fn hide(&self) {
        hide_native(&mut self.core.state.borrow_mut());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png() -> Vec<u8> {
        let mut img = image::RgbaImage::new(2, 1);
        img.put_pixel(0, 0, image::Rgba([10, 20, 30, 255]));
        img.put_pixel(1, 0, image::Rgba([200, 100, 50, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn a_snapshot_becomes_a_bgra_frame_tagged_with_its_layout_size() {
        let f = png_to_frame(&tiny_png(), (300, 200)).unwrap();
        assert_eq!((f.width, f.height), (2, 1));
        assert_eq!((f.css_width, f.css_height), (300, 200));
        // RGBA 10,20,30 is BGRA 30,20,10.
        assert_eq!(&f.bgra[..4], &[30, 20, 10, 255]);
        assert_eq!(&f.bgra[4..8], &[50, 100, 200, 255]);
    }

    #[test]
    fn bytes_that_are_not_a_png_are_no_frame() {
        assert!(png_to_frame(b"nope", (1, 1)).is_none());
    }
}
