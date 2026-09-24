//! Full screen that covers the notch (`ui.fullscreen: "cover"`), and the one
//! door every full screen toggle goes through.
//!
//! gpui's window class, `GPUIWindow`, inherits `toggleFullScreen:` from
//! NSWindow. `install` gives it its own, `toggle_full_screen` here, which
//! asks `infiniterm_core::fullscreen::decide` and either passes the message
//! up to NSWindow (macOS's full screen, its own Space) or covers the screen:
//! the window takes the whole `NSScreen.frame` with the menu bar and Dock
//! auto-hidden, so on a MacBook the strip beside the notch is the title bar
//! rather than black. The window stays TITLED, its traffic lights hidden and
//! its resize edges off: a borderless one has no standard buttons, and gpui's
//! `windowDidResize:` asks for them by pointer and panicked on nil (seen on
//! the Air, 2026-09-24). A titled window is kept under the menu bar by
//! `constrainFrameRect:toScreen:`, so that is answered here as well: while
//! covering, the frame is left as asked. The green button, `app.fullscreen`
//! (Cmd+Ctrl+F, `toggle` below) and the Window menu all send that message, so
//! they cannot disagree. Called from main.rs (install, the menu, launch), runtime.rs (the
//! effect, the window save) and the poll loop (the setting).
//!
//! Constraints: one window, so the saved frame is one static. The
//! presentation options are app-wide but AppKit applies them only while the
//! app is in front, so switching away shows the Dock and menu bar again by
//! itself. gpui reports a covered window as an ordinary one the
//! size of the screen, so the window save (`runtime.rs::note_window`) asks
//! `covering` and keeps the frame from before.
use core_graphics::geometry::CGRect;
use infiniterm_core::config::FullscreenMode;
use infiniterm_core::fullscreen::{decide, Toggle};
use objc::runtime::{class_addMethod, Class, Object, Sel, BOOL, NO, YES};
use objc::{class, msg_send, sel, sel_impl};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

type Id = *mut Object;

/// NSWindowStyleMaskFullScreen: set while the window is in macOS's own
/// full screen.
const STYLE_FULL_SCREEN: u64 = 1 << 14;
/// NSWindowStyleMaskResizable: off while covering, or the screen's edges
/// would offer to resize a window that fills the screen.
const STYLE_RESIZABLE: u64 = 1 << 3;
/// Close, minimise, zoom (NSWindowButton 0, 1, 2).
const TRAFFIC_LIGHTS: [u64; 3] = [0, 1, 2];
/// NSApplicationPresentationAutoHideDock | AutoHideMenuBar: both show again
/// when the pointer reaches their edge. AppKit refuses a hidden menu bar
/// without the Dock hidden too.
const PRESENT_COVER: u64 = (1 << 0) | (1 << 2);
/// `-(void)toggleFullScreen:(id)sender`.
const TYPES: &[u8] = b"v@:@\0";
/// `-(NSRect)constrainFrameRect:(NSRect)r toScreen:(NSScreen *)s`.
const CONSTRAIN_TYPES: &[u8] =
    b"{CGRect={CGPoint=dd}{CGSize=dd}}@:{CGRect={CGPoint=dd}{CGSize=dd}}@\0";

/// What cover mode changed, to put back on the way out.
#[derive(Clone, Copy)]
struct Saved {
    frame: CGRect,
    style: u64,
    presentation: u64,
    movable: BOOL,
}

static COVER: AtomicBool = AtomicBool::new(true);
static SAVED: Mutex<Option<Saved>> = Mutex::new(None);

/// The setting, pushed from the model; the toggle has no model to ask.
pub fn set_mode(mode: FullscreenMode) {
    COVER.store(mode == FullscreenMode::Cover, Ordering::Relaxed);
}

fn mode() -> FullscreenMode {
    if COVER.load(Ordering::Relaxed) {
        FullscreenMode::Cover
    } else {
        FullscreenMode::Native
    }
}

/// The window is covering the screen now.
pub fn covering() -> bool {
    SAVED.lock().map(|s| s.is_some()).unwrap_or(false)
}

/// Gives gpui's window class its own `toggleFullScreen:`. After the first
/// window exists, since gpui declares the class when it opens one. If a
/// later gpui defines the method itself, `class_addMethod` refuses and full
/// screen stays native; the warning says so.
pub fn install() {
    let Some(window) = Class::get("GPUIWindow") else {
        eprintln!("[infiniterm/warn] full screen: no GPUIWindow class; cover mode is off");
        return;
    };
    let added = unsafe {
        class_addMethod(
            window as *const Class as *mut Class,
            sel!(toggleFullScreen:),
            std::mem::transmute::<extern "C" fn(&Object, Sel, Id), objc::runtime::Imp>(
                toggle_full_screen,
            ),
            TYPES.as_ptr() as *const _,
        )
    };
    if added == NO {
        eprintln!("[infiniterm/warn] full screen: GPUIWindow has its own toggleFullScreen:; cover mode is off");
    }
    unsafe {
        class_addMethod(
            window as *const Class as *mut Class,
            sel!(constrainFrameRect:toScreen:),
            std::mem::transmute::<
                extern "C" fn(&Object, Sel, CGRect, Id) -> CGRect,
                objc::runtime::Imp,
            >(constrain),
            CONSTRAIN_TYPES.as_ptr() as *const _,
        );
    }
}

extern "C" fn constrain(this: &Object, _: Sel, frame: CGRect, screen: Id) -> CGRect {
    if covering() {
        frame
    } else {
        unsafe {
            msg_send![super(this, class!(NSWindow)), constrainFrameRect: frame toScreen: screen]
        }
    }
}

fn show_traffic_lights(win: &Object, show: bool) {
    for kind in TRAFFIC_LIGHTS {
        unsafe {
            let button: Id = msg_send![win, standardWindowButton: kind];
            if !button.is_null() {
                let _: () = msg_send![button, setHidden: if show { NO } else { YES }];
            }
        }
    }
}

/// `app.fullscreen` and the menu: the green button's message, to our window,
/// on the next turn of the run loop. Sent at once from inside gpui's update
/// (a key, an effect), the resize it causes reaches gpui while gpui is busy,
/// is dropped, and the app keeps drawing at the old size: on the Air the
/// status bar sat below the window's bottom edge after Cmd+Ctrl+F until the
/// window was resized again. gpui's own `toggle_fullscreen` defers for the
/// same reason.
pub fn toggle() {
    if let Some(win) = our_window() {
        unsafe {
            let _: () = msg_send![win, performSelector: sel!(toggleFullScreen:)
                withObject: std::ptr::null_mut::<Object>()
                afterDelay: 0.0f64];
        }
    }
}

fn our_window() -> Option<Id> {
    let class = Class::get("GPUIWindow")?;
    unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let windows: Id = msg_send![app, windows];
        let count: usize = msg_send![windows, count];
        (0..count)
            .map(|i| -> Id { msg_send![windows, objectAtIndex: i] })
            .find(|&w| {
                let yes: BOOL = msg_send![w, isKindOfClass: class];
                yes == YES
            })
    }
}

extern "C" fn toggle_full_screen(this: &Object, _: Sel, sender: Id) {
    let style: u64 = unsafe { msg_send![this, styleMask] };
    match decide(mode(), covering(), style & STYLE_FULL_SCREEN != 0) {
        Toggle::Native => unsafe {
            let _: () = msg_send![super(this, class!(NSWindow)), toggleFullScreen: sender];
        },
        Toggle::EnterCover => enter(this, style),
        Toggle::ExitCover => exit(this),
    }
}

fn enter(win: &Object, style: u64) {
    unsafe {
        let screen: Id = msg_send![win, screen];
        if screen.is_null() {
            return;
        }
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let saved = Saved {
            frame: msg_send![win, frame],
            style,
            presentation: msg_send![app, presentationOptions],
            movable: msg_send![win, isMovable],
        };
        if let Ok(mut s) = SAVED.lock() {
            *s = Some(saved);
        }
        let cover: CGRect = msg_send![screen, frame];
        let _: () = msg_send![app, setPresentationOptions: PRESENT_COVER];
        let _: () = msg_send![win, setStyleMask: style & !STYLE_RESIZABLE];
        show_traffic_lights(win, false);
        // The title bar is ours and draggable; a covered window must not
        // slide off the screen it covers.
        let _: () = msg_send![win, setMovable: NO];
        let _: () = msg_send![win, setFrame: cover display: YES];
        let _: () = msg_send![win, makeKeyAndOrderFront: std::ptr::null_mut::<Object>()];
    }
}

fn exit(win: &Object) {
    let Some(saved) = SAVED.lock().ok().and_then(|mut s| s.take()) else {
        return;
    };
    unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, setPresentationOptions: saved.presentation];
        let _: () = msg_send![win, setFrame: saved.frame display: YES];
        let _: () = msg_send![win, setStyleMask: saved.style];
        show_traffic_lights(win, true);
        let _: () = msg_send![win, setMovable: saved.movable];
        let _: () = msg_send![win, makeKeyAndOrderFront: std::ptr::null_mut::<Object>()];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_reaches_the_toggle() {
        set_mode(FullscreenMode::Native);
        assert_eq!(mode(), FullscreenMode::Native);
        set_mode(FullscreenMode::Cover);
        assert_eq!(mode(), FullscreenMode::Cover);
        assert!(!covering(), "nothing is covered before a toggle");
    }

    /// gpui's class exists in the test process too (see middle_drag.rs):
    /// after `install`, its `toggleFullScreen:` is ours, not NSWindow's.
    #[test]
    fn gpuis_window_answers_the_toggle_with_ours() {
        use objc::runtime::{class_getInstanceMethod, method_getImplementation};
        let window = Class::get("GPUIWindow").expect("gpui registers its window class");
        install();
        unsafe {
            let m = class_getInstanceMethod(window, sel!(toggleFullScreen:));
            assert!(!m.is_null());
            let ours: extern "C" fn(&Object, Sel, Id) = toggle_full_screen;
            assert_eq!(method_getImplementation(m) as usize, ours as usize);
        }
    }
}
