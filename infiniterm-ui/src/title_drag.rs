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
//! does not drag it from the title bar at all, and the one place that should
//! drag it, the bare bar, starts the drag itself (`begin`, from the title bar
//! div's mouse-down, which a tab or the "+" stops, `overlays.rs`) through
//! `performWindowDragWithEvent:`. A double-click on the bar does the system's
//! title bar action (zoom or minimise) through gpui's `titlebar_double_click`.
//! The view override stays as belt and braces. Cost: the system's Ctrl+Cmd+drag
//! (move a window by any point) is off for this window, as it is for any window
//! that is not movable.
//!
//! macOS only. Called once after the first window exists, like the others.
use objc::runtime::{class_addMethod, Class, Object, Sel, BOOL, NO};
use objc::{class, msg_send, sel, sel_impl};

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
    // that is not movable is not dragged by its title bar; the bare bar drags
    // it by hand (`begin`).
    if let Some(win) = main_window() {
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

/// Starts dragging the window with the mouse-down being handled right now.
/// Blocks in AppKit's drag loop until the button is released.
pub fn begin() {
    unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let event: Id = msg_send![app, currentEvent];
        if event.is_null() {
            return;
        }
        let window: Id = msg_send![event, window];
        if window.is_null() {
            return;
        }
        let _: () = msg_send![window, performWindowDragWithEvent: event];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc::runtime::{class_getInstanceMethod, method_getImplementation};

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
