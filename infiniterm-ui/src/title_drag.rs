//! The window drag from the title bar, by hand (#414).
//!
//! The main window has a transparent title bar (`appears_transparent`), so the
//! top strip of gpui's view is the window's title bar to AppKit. AppKit starts
//! a window drag on a press in that strip whenever the view answers
//! `mouseDownCanMoveWindow` with YES, which NSView does by default for a view
//! that is not opaque, and gpui's `GPUIView` never overrides it. The drag began
//! at the PRESS, before gpui saw enough movement to start the workspace tab's
//! own drag, so dragging a tab moved the window and reordering was hardly
//! possible.
//!
//! What did NOT work (found on screen): overriding `mouseDownCanMoveWindow` on
//! the view. The press in the title bar strip is taken by AppKit's own title
//! bar layer, above the view, so the view's answer never mattered. What works:
//! the main window is not movable (`install`: `setMovable: NO`), so AppKit
//! does not drag it from the title bar at all, except while the pointer rests
//! on the bare bar (`hover`, from every mouse move, `overlays.rs`): there the
//! window is movable and AppKit's own title bar drag moves it. A press always
//! comes after a move, so a tab or the "+" is pressed on a window that is not
//! movable. A double-click on the bar does the system's title bar action
//! (zoom or minimise) through gpui's `titlebar_double_click`.
//!
//! Until macOS 27.2 the bare bar started the drag itself through
//! `performWindowDragWithEvent:` on the press, with the window never movable.
//! Since 27.2 that call returns at once and moves nothing on a window that is
//! not movable, and setting the window movable just before it does not help
//! either (found on screen, 2026-11-17). A drag loop of our own cannot run in
//! gpui's mouse handler: waiting for the next event there runs gpui's timers,
//! which borrow the app the handler holds ("RefCell already borrowed").
//!
//! The view override stays as belt and braces. Cost: the system's Ctrl+Cmd+drag
//! (move a window by any point) is off for this window, as it is for any window
//! that is not movable.
//!
//! macOS only. Called once after the first window exists, like the others.
use gpui::{Bounds, Pixels, Point};
use objc::runtime::{class_addMethod, Class, Object, Sel, BOOL, NO, YES};
use objc::{class, msg_send, sel, sel_impl};
use std::sync::atomic::{AtomicPtr, Ordering};

/// `-(BOOL)mouseDownCanMoveWindow`: BOOL's encoding is `B` on Apple Silicon.
#[cfg(target_arch = "aarch64")]
const TYPES: &[u8] = b"B@:\0";
#[cfg(not(target_arch = "aarch64"))]
const TYPES: &[u8] = b"c@:\0";

type Id = *mut Object;

/// The view never starts a window drag by itself.
pub extern "C" fn cannot_move_window(_this: &Object, _sel: Sel) -> BOOL {
    NO
}

/// The main window: the first `GPUIWindow` when this runs, right after it
/// opens (the extracted terminal windows come later and are left alone).
fn main_window() -> Option<Id> {
    let class = Class::get("GPUIWindow")?;
    unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let windows: Id = msg_send![app, windows];
        let count: usize = msg_send![windows, count];
        (0..count)
            .map(|i| -> Id { msg_send![windows, objectAtIndex: i] })
            .find(|&w| {
                let yes: BOOL = msg_send![w, isKindOfClass: class];
                yes == objc::runtime::YES
            })
    }
}

pub fn install() {
    // AppKit's own title bar layer starts the window drag from a press on a
    // tab (the view's answer below did not stop it, found on screen). A window
    // that is not movable is not dragged by its title bar; the bare bar makes
    // it movable while the pointer is on it (`hover`).
    if let Some(win) = main_window() {
        MAIN.store(win, Ordering::Relaxed);
        unsafe {
            let _: () = msg_send![win, setMovable: NO];
        }
    }
    let Some(view) = Class::get("GPUIView") else {
        eprintln!("[infiniterm/warn] title drag: no GPUIView class; tabs may move the window");
        return;
    };
    unsafe {
        let _ = class_addMethod(
            view as *const Class as *mut Class,
            sel!(mouseDownCanMoveWindow),
            std::mem::transmute::<extern "C" fn(&Object, Sel) -> BOOL, objc::runtime::Imp>(
                cannot_move_window,
            ),
            TYPES.as_ptr() as *const _,
        );
    }
}

/// The main window `install` found, for `hover`.
static MAIN: AtomicPtr<Object> = AtomicPtr::new(std::ptr::null_mut());

/// Whether `at` is on the bare title bar: inside the bar and on none of the
/// controls in it (the tabs, the "+"), which a press must not drag.
pub fn on_bare_bar(
    bar: Bounds<Pixels>,
    controls: impl IntoIterator<Item = Bounds<Pixels>>,
    at: Point<Pixels>,
) -> bool {
    bar.contains(&at) && !controls.into_iter().any(|c| c.contains(&at))
}

/// The pointer moved with no button held: the main window is movable only
/// while it is on the bare bar. Not while covering the screen, which keeps
/// the window where it is (`fullscreen.rs` makes it not movable then).
pub fn hover(on_bare_bar: bool) {
    let win = MAIN.load(Ordering::Relaxed);
    if win.is_null() {
        return;
    }
    let want = if on_bare_bar && !crate::fullscreen::covering() {
        YES
    } else {
        NO
    };
    unsafe {
        let now: BOOL = msg_send![win, isMovable];
        if now != want {
            let _: () = msg_send![win, setMovable: want];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px, size};
    use objc::runtime::{class_getInstanceMethod, method_getImplementation};

    /// The bar is draggable around its tabs and the "+", never on them, and
    /// nothing below the bar is.
    #[test]
    fn only_the_bare_bar_drags_the_window() {
        let bar = Bounds::new(point(px(0.), px(0.)), size(px(800.), px(38.)));
        let tab = Bounds::new(point(px(80.), px(6.)), size(px(120.), px(26.)));
        let plus = Bounds::new(point(px(204.), px(6.)), size(px(24.), px(26.)));
        let bare = |x: f32, y: f32| on_bare_bar(bar, [tab, plus], point(px(x), px(y)));
        assert!(bare(500., 20.), "empty bar right of the tabs");
        assert!(bare(80., 2.), "above a tab, still in the bar");
        assert!(!bare(100., 20.), "on a tab");
        assert!(!bare(210., 20.), "on the +");
        assert!(!bare(500., 60.), "below the bar");
    }

    /// gpui's `GPUIView` is registered in the test process too: after
    /// `install` it answers `mouseDownCanMoveWindow` with our NO, so AppKit
    /// no longer starts a window drag from a press on a tab.
    #[test]
    fn the_view_no_longer_lets_a_press_move_the_window() {
        let view = Class::get("GPUIView").expect("gpui registers its view class");
        install();
        unsafe {
            let m = class_getInstanceMethod(view, sel!(mouseDownCanMoveWindow));
            assert!(!m.is_null(), "mouseDownCanMoveWindow is answered");
            let ours: extern "C" fn(&Object, Sel) -> BOOL = cannot_move_window;
            assert_eq!(
                method_getImplementation(m) as usize,
                ours as usize,
                "and by ours"
            );
        }
    }
}
