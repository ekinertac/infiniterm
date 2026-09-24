//! Middle-button drags reach the app. gpui's macOS view (`GPUIView`) answers
//! `mouseDragged:` (moving with the left button held) but never registers
//! `otherMouseDragged:`, the message AppKit sends while the middle button is
//! held, so a middle press started a pan (`pan_mode::starts_pan`) and then
//! no movement ever arrived: middle-drag panning did nothing, while Cmd+drag,
//! a left-button drag, worked. Found 2026-09-24 with Karabiner's event viewer
//! showing the button reaching macOS.
//!
//! gpui's handler ignores which message called it and translates the event by
//! its type, and it already maps `NSOtherMouseDragged` to a mouse move with
//! the middle button pressed (`platform/mac/events.rs`). So the fix is to give
//! the class the missing method, pointing at the same implementation
//! `mouseDragged:` has. Called once, after the first window exists, because
//! gpui declares the class when it opens one. macOS only; gpui's Windows and
//! Linux platforms deliver these drags already.
//!
//! If a later gpui registers `otherMouseDragged:` itself, `class_addMethod`
//! refuses to replace it and this does nothing, which is the right outcome.
use objc::runtime::{class_addMethod, class_getInstanceMethod, method_getImplementation, Class};
use objc::sel;
use objc::sel_impl;

/// `-(void)otherMouseDragged:(NSEvent *)event`: returns void, takes self,
/// the selector and one object.
const TYPES: &[u8] = b"v@:@\0";

pub fn install() {
    let Some(view) = Class::get("GPUIView") else {
        eprintln!("[infiniterm] middle drag: no GPUIView class yet; middle-button panning is off");
        return;
    };
    unsafe {
        let dragged = class_getInstanceMethod(view, sel!(mouseDragged:));
        if dragged.is_null() {
            return;
        }
        let imp = method_getImplementation(dragged);
        let _ = class_addMethod(
            view as *const Class as *mut Class,
            sel!(otherMouseDragged:),
            imp,
            TYPES.as_ptr() as *const _,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gpui's own `GPUIView` is registered in the test process too: after
    /// `install`, it answers `otherMouseDragged:` with the very handler its
    /// `mouseDragged:` runs.
    #[test]
    fn gpuis_view_answers_middle_drags_with_its_left_drag_handler() {
        let view = Class::get("GPUIView").expect("gpui registers its view class");
        install();
        unsafe {
            let other = class_getInstanceMethod(view, sel!(otherMouseDragged:));
            assert!(!other.is_null(), "otherMouseDragged: was added");
            let left = class_getInstanceMethod(view, sel!(mouseDragged:));
            // Compared as addresses: both come from the Objective-C runtime's
            // method table, not from Rust codegen, so the numbers are exact.
            assert_eq!(
                method_getImplementation(other) as usize,
                method_getImplementation(left) as usize,
                "and it is the left drag's own handler"
            );
        }
    }
}
