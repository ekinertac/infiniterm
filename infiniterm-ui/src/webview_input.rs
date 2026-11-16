//! Input for the webview browser card (#761): an NSEvent local monitor that
//! sees every scroll, mouse and key event BEFORE AppKit hit-tests it, and
//! decides with `infiniterm_core::webview_gesture` whether the page or the
//! canvas gets it. A native view is above everything gpui draws, so without
//! this a Cmd+scroll over a page would scroll the page, a middle-drag would
//! be the page's, and a click on a card drawn over a page would reach the
//! page behind it.
//!
//! What goes where (CLAUDE.md's rules, in the order they were checked on
//! screen in the spike): a plain wheel over a page scrolls the page; Cmd or
//! Ctrl + wheel, Cmd+drag, middle-drag and a click on a card drawn in front
//! go to gpui's own view, which runs the app's normal mouse path (so the
//! zoom, the pan and `scroll_nav` are the code that was always there); the
//! first click on an unfocused card is gpui's too, because it is the one
//! that focuses; a pinch has no gpui path, so it is queued and the app makes
//! the same zoom a Ctrl+wheel makes; Cmd chords go to gpui's
//! `performKeyEquivalent:` while the page is first responder, and every
//! other key stays with the page (dead keys and input methods included).
//!
//! The pages' rects and covers are registered by `WebSurface::place` every
//! frame, from the very list the mask holes are cut from, so what the eye
//! sees and what the hand reaches cannot disagree.
//!
//! Where it fits: `webview.rs` registers pages and installs the monitor when
//! the first page is made; `browser_body.rs` takes `take_page_click` and
//! `input.rs`/`runtime.rs` take `take_pinches`. Related: `keycode.rs` (the
//! physical key a Cmd chord is named by: `record_event` is called before a
//! chord is forwarded, because the monitor that keeps it may run after this
//! one) and the spike's `gesture.rs` in PR #812.
//! Constraint: main thread only; every table is thread-local for that reason.
use infiniterm_core::webview_gesture::{self as gesture, KeyAction, Kind, Mods, PageRoute, Target};
use objc2::{
    class, msg_send,
    runtime::{AnyObject, Bool},
};
use objc2_foundation::{NSPoint, NSRect};
use std::cell::{Cell, RefCell};

struct Entry {
    key: u64,
    route: PageRoute,
    locked: bool,
}

thread_local! {
    static PAGES: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    static GPUI_VIEW: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) };
    static GPUI_DRAG: Cell<bool> = const { Cell::new(false) };
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    /// Pages whose card has the keyboard focus but whose page has not been
    /// locked yet, and got a click that reached the page: the card locks.
    static CLICKS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    /// Pinches over a page: (window point, magnification).
    static PINCHES: RefCell<Vec<((f64, f64), f64)>> = const { RefCell::new(Vec::new()) };
}

/// A new key for a page; the registry above is keyed by it.
pub fn next_key() -> u64 {
    thread_local!(static N: Cell<u64> = const { Cell::new(0) });
    N.with(|n| {
        n.set(n.get() + 1);
        n.get()
    })
}

pub fn begin_frame() {
    PAGES.with(|p| p.borrow_mut().clear());
}

pub fn register(key: u64, route: PageRoute, locked: bool) {
    PAGES.with(|p| p.borrow_mut().push(Entry { key, route, locked }));
}

/// gpui's own view, the superview of every web view.
pub fn set_gpui_view(view: *mut AnyObject) {
    GPUI_VIEW.with(|v| v.set(view));
}

/// Whether a click reached this page since last asked, while its card had
/// the focus but not the lock.
pub fn take_click(key: u64) -> bool {
    CLICKS.with(|c| {
        let mut c = c.borrow_mut();
        let before = c.len();
        c.retain(|k| *k != key);
        c.len() != before
    })
}

/// A pinch seen over a page since last asked; the app turns each into a
/// wheel zoom (`webview_gesture::pinch_to_wheel_dy`).
pub fn take_pinches() -> Vec<((f64, f64), f64)> {
    PINCHES.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

/// Anything waiting for the app to look at it: a frame is needed.
pub fn has_pending() -> bool {
    CLICKS.with(|c| !c.borrow().is_empty()) || PINCHES.with(|p| !p.borrow().is_empty())
}

pub fn install() {
    if INSTALLED.with(|i| i.replace(true)) {
        return;
    }
    let block = block2::RcBlock::new(|ev: *mut AnyObject| -> *mut AnyObject {
        // SAFETY: AppKit hands the monitor a live NSEvent on the main thread.
        unsafe { route_event(ev) }
    });
    // Key down, the wheel, a pinch, and the left and other mouse events.
    let mask: usize = (1 << 10)
        | (1 << 22)
        | (1 << 30)
        | (1 << 1)
        | (1 << 2)
        | (1 << 6)
        | (1 << 25)
        | (1 << 26)
        | (1 << 27);
    unsafe {
        let _: *mut AnyObject = msg_send![class!(NSEvent), addLocalMonitorForEventsMatchingMask: mask, handler: &*block];
    }
    // The monitor holds the block for the life of the process.
    std::mem::forget(block);
}

unsafe fn class_name(o: *mut AnyObject) -> String {
    if o.is_null() {
        return "nil".into();
    }
    (*o).class().name().to_str().unwrap_or("?").to_string()
}

/// Returns the event to let AppKit have it, or null when it was handled here.
unsafe fn route_event(ev: *mut AnyObject) -> *mut AnyObject {
    let gv = GPUI_VIEW.with(Cell::get);
    if gv.is_null() || PAGES.with(|p| p.borrow().is_empty()) {
        return ev;
    }
    let w: *mut AnyObject = msg_send![ev, window];
    if w.is_null() || !class_name(w).contains("GPUIWindow") {
        return ev;
    }
    let ty: usize = msg_send![ev, type];
    let flags: usize = msg_send![ev, modifierFlags];
    let mods = Mods {
        cmd: flags & (1 << 20) != 0,
        ctrl: flags & (1 << 18) != 0,
    };

    if ty == 10 {
        // A key. Which view is first responder says whether the page types.
        let fr: *mut AnyObject = msg_send![w, firstResponder];
        let page_focused = class_name(fr).contains("WryWebView");
        return match gesture::key_route(mods.cmd, page_focused) {
            KeyAction::Pass => ev,
            KeyAction::TryGpui => {
                // The physical key the chord is named by, set before the
                // app reads it (see the module comment).
                crate::keycode::record_event(ev as *mut _);
                let handled: Bool = msg_send![gv, performKeyEquivalent: ev];
                if handled.as_bool() {
                    std::ptr::null_mut()
                } else {
                    ev
                }
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
    let bounds: NSRect = msg_send![gv, bounds];
    // Window pixels, top-left origin, as gpui paints.
    let p = (loc.x, bounds.size.height - loc.y);
    let owns = GPUI_DRAG.with(Cell::get);
    let routes: Vec<PageRoute> =
        PAGES.with(|pg| pg.borrow().iter().map(|e| e.route.clone()).collect());
    let target = gesture::route(kind, mods, p, &routes, owns);
    if gesture::starts_gpui_drag(kind, target) {
        GPUI_DRAG.with(|d| d.set(true));
    }
    if matches!(kind, Kind::LeftUp | Kind::OtherUp) {
        GPUI_DRAG.with(|d| d.set(false));
    }
    match target {
        Target::Page => {
            // A click that reaches a focused card's page locks the keyboard
            // to it (the card's own rule: the click that reaches the page).
            if kind == Kind::LeftDown {
                PAGES.with(|pg| {
                    let pages = pg.borrow();
                    let routes_only: Vec<PageRoute> =
                        pages.iter().map(|e| e.route.clone()).collect();
                    if let Some(at) = gesture::page_at(&routes_only, p) {
                        if let Some(e) = pages.iter().find(|e| &e.route == at) {
                            if !e.locked {
                                CLICKS.with(|c| c.borrow_mut().push(e.key));
                            }
                        }
                    }
                });
            }
            ev
        }
        Target::Gpui => {
            let _: () = msg_send![gv, performSelector: sel, withObject: ev];
            std::ptr::null_mut()
        }
        Target::Canvas => {
            let d: f64 = msg_send![ev, magnification];
            PINCHES.with(|pc| pc.borrow_mut().push((p, d)));
            std::ptr::null_mut()
        }
    }
}
