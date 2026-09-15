#![allow(unexpected_cfgs, clippy::missing_transmute_annotations)]
//! CEF requires the NSApplication to implement `CefAppProtocol`; gpui owns
//! the subclass, so the two methods are added at runtime before
//! `initialize`. Without them CEF aborts on `isHandlingSendEvent` (see
//! spikes/cef-frame/NOTES.md).
use objc::runtime::{class_addMethod, Class, Object, Sel, BOOL, NO, YES};
use objc::{class, msg_send, sel, sel_impl};
use std::sync::atomic::{AtomicBool, Ordering};

static HANDLING: AtomicBool = AtomicBool::new(false);

extern "C" fn is_handling_send_event(_this: &Object, _sel: Sel) -> BOOL {
    if HANDLING.load(Ordering::Relaxed) {
        YES
    } else {
        NO
    }
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
