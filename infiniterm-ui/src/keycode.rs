//! The physical key behind a keystroke. gpui's `Keystroke` carries the
//! character the layout produced and no key code, and the keymap is
//! written for physical keys (the reference's rule: chords come from
//! `e.code`, never `e.key`, or `Cmd+=` on a Turkish Q keyboard becomes
//! `Cmd+Shift+0` and `Cmd+Shift+=` collides with it). macOS still knows
//! the code: an NSEvent local monitor sees every key-down before gpui
//! does and keeps the last code here; `key_down` reads it for the same
//! event and hands the keymap a DOM-style code name.
//!
//! Only letters, digits and the US punctuation row are named; every other
//! key keeps gpui's name, which is already stable across layouts.
#![allow(unexpected_cfgs, clippy::missing_transmute_annotations)]
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

static LAST: AtomicI32 = AtomicI32::new(-1);
/// Whether Shift was down for that key, from the event's own flags: gpui
/// clears its shift flag when it reports a shifted character, so the
/// keystroke cannot say.
static LAST_SHIFT: AtomicBool = AtomicBool::new(false);

/// NSEventMaskKeyDown.
const KEY_DOWN_MASK: u64 = 1 << 10;
/// NSEventModifierFlagShift.
const SHIFT_FLAG: u64 = 1 << 17;

/// Starts watching key-downs. Once per process, before the window opens.
pub fn install() {
    let block = block::ConcreteBlock::new(|event: *mut Object| -> *mut Object {
        let code: u16 = unsafe { msg_send![event, keyCode] };
        let flags: u64 = unsafe { msg_send![event, modifierFlags] };
        LAST.store(code as i32, Ordering::Relaxed);
        LAST_SHIFT.store(flags & SHIFT_FLAG != 0, Ordering::Relaxed);
        event
    });
    // The monitor holds the block for the life of the process.
    let block = Box::leak(Box::new(block.copy()));
    let _: *mut Object = unsafe {
        msg_send![class!(NSEvent), addLocalMonitorForEventsMatchingMask: KEY_DOWN_MASK handler: &**block]
    };
}

/// The DOM code of the most recent key-down, when it is one the keymap
/// names.
pub fn last_code() -> Option<&'static str> {
    let code = LAST.load(Ordering::Relaxed);
    (code >= 0).then(|| dom_code(code as u16)).flatten()
}

/// Whether Shift was held for the most recent key-down.
pub fn last_shift() -> bool {
    LAST_SHIFT.load(Ordering::Relaxed)
}

/// macOS virtual key codes (Carbon `kVK_*`) to DOM `code` names.
pub fn dom_code(code: u16) -> Option<&'static str> {
    Some(match code {
        0x00 => "KeyA",
        0x01 => "KeyS",
        0x02 => "KeyD",
        0x03 => "KeyF",
        0x04 => "KeyH",
        0x05 => "KeyG",
        0x06 => "KeyZ",
        0x07 => "KeyX",
        0x08 => "KeyC",
        0x09 => "KeyV",
        0x0B => "KeyB",
        0x0C => "KeyQ",
        0x0D => "KeyW",
        0x0E => "KeyE",
        0x0F => "KeyR",
        0x10 => "KeyY",
        0x11 => "KeyT",
        0x12 => "Digit1",
        0x13 => "Digit2",
        0x14 => "Digit3",
        0x15 => "Digit4",
        0x16 => "Digit6",
        0x17 => "Digit5",
        0x18 => "Equal",
        0x19 => "Digit9",
        0x1A => "Digit7",
        0x1B => "Minus",
        0x1C => "Digit8",
        0x1D => "Digit0",
        0x1E => "BracketRight",
        0x1F => "KeyO",
        0x20 => "KeyU",
        0x21 => "BracketLeft",
        0x22 => "KeyI",
        0x23 => "KeyP",
        0x25 => "KeyL",
        0x26 => "KeyJ",
        0x27 => "Quote",
        0x28 => "KeyK",
        0x29 => "Semicolon",
        0x2A => "Backslash",
        0x2B => "Comma",
        0x2C => "Slash",
        0x2D => "KeyN",
        0x2E => "KeyM",
        0x2F => "Period",
        0x32 => "Backquote",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_punctuation_row_and_the_letters_have_codes() {
        assert_eq!(dom_code(0x18), Some("Equal"));
        assert_eq!(dom_code(0x1B), Some("Minus"));
        assert_eq!(dom_code(0x00), Some("KeyA"));
        assert_eq!(dom_code(0x1D), Some("Digit0"));
        assert_eq!(dom_code(0x24), None); // Return keeps gpui's name
        assert_eq!(dom_code(0x7B), None); // Left arrow too
    }
}
