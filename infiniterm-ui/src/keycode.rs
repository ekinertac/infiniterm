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
//!
//! Windows has both halves of the same problem and one extra: which SIDE
//! of Ctrl is down. See `windows_hook` at the bottom of this file, and
//! `ctrl_sides` for why the side is the whole answer to "Windows has no
//! Cmd key".
#![allow(unexpected_cfgs, clippy::missing_transmute_annotations)]
use infiniterm_core::config::CommandModifier;
#[cfg(target_os = "macos")]
use objc::runtime::Object;
#[cfg(target_os = "macos")]
use objc::{class, msg_send, sel, sel_impl};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

static LAST: AtomicI32 = AtomicI32::new(-1);
/// Whether Shift was down for that key, from the event's own flags: gpui
/// clears its shift flag when it reports a shifted character, so the
/// keystroke cannot say.
static LAST_SHIFT: AtomicBool = AtomicBool::new(false);
/// Whether that key-down was a DEAD key: one that types nothing by itself
/// and waits for the next letter (Option+E on a US layout, then E, for é).
/// The event's `characters` is empty for exactly that case, and it is the
/// only authoritative sign of it: gpui's keystroke reports the standalone
/// accent instead (`\u{b4}`), because it translates the key with a space
/// after it to have something to show, so nothing downstream of gpui can
/// tell a dead key from a layout where Option+E simply types an accent.
#[cfg(target_os = "macos")]
static LAST_DEAD: AtomicBool = AtomicBool::new(false);

/// NSEventMaskKeyDown.
#[cfg(target_os = "macos")]
const KEY_DOWN_MASK: u64 = 1 << 10;
/// NSEventModifierFlagShift.
#[cfg(target_os = "macos")]
const SHIFT_FLAG: u64 = 1 << 17;

/// Starts watching key-downs. Once per process, before the window opens.
#[cfg(target_os = "macos")]
pub fn install() {
    let block = block::ConcreteBlock::new(|event: *mut Object| -> *mut Object {
        let code: u16 = unsafe { msg_send![event, keyCode] };
        let flags: u64 = unsafe { msg_send![event, modifierFlags] };
        // `characters` is nil for a modifier-only event, empty for a dead
        // key, and the typed text otherwise. Only "empty" is a dead key.
        let dead = unsafe {
            let chars: *mut Object = msg_send![event, characters];
            if chars.is_null() {
                false
            } else {
                let len: usize = msg_send![chars, length];
                len == 0
            }
        };
        LAST.store(code as i32, Ordering::Relaxed);
        LAST_SHIFT.store(flags & SHIFT_FLAG != 0, Ordering::Relaxed);
        LAST_DEAD.store(dead, Ordering::Relaxed);
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

/// Whether the most recent key-down was a dead key. A body or a field
/// that sees this must NOT take the key: taking it tells macOS it was
/// handled and the composition never happens. See `LAST_DEAD`.
#[cfg(target_os = "macos")]
pub fn last_dead() -> bool {
    LAST_DEAD.load(Ordering::Relaxed)
}

/// macOS virtual key codes (Carbon `kVK_*`) to DOM `code` names.
#[cfg(target_os = "macos")]
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

// The numbers here are Carbon virtual key codes; Windows has its own table
// and its own tests at the bottom of this file.
#[cfg(all(test, target_os = "macos"))]
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

/// The Windows half: a thread-local keyboard hook, for the same reason the
/// Mac has an NSEvent monitor.
///
/// gpui names a Windows key by running its virtual key through the CURRENT
/// LAYOUT (`MapVirtualKeyW(vk, MAPVK_VK_TO_CHAR)`), and for punctuation and
/// digits with Shift held it hands back the SHIFTED character with the shift
/// flag cleared. That is the macOS trap word for word: on Turkish Q the `[`
/// key is `ğ`, so a binding written `[` would never match, and `ctrl+shift+=`
/// and `ctrl+shift+0` would collide the way `cmd+=` and `cmd+shift+0` did.
///
/// The fix is the same too: take the key from the hardware. A SCAN CODE, not
/// a virtual key, because a virtual key is itself layout-dependent for the
/// OEM keys (the ones this table exists for) while a scan code names the
/// physical switch under the finger.
///
/// `WH_KEYBOARD` on our own thread, not `WH_KEYBOARD_LL`: the low-level hook
/// is global and would have this process watching every key Ekin types in
/// every other application. A thread hook sees only messages on our own
/// queue, which is what the Mac's LOCAL monitor sees.
#[cfg(windows)]
mod windows_hook {
    use super::{LAST, LAST_SHIFT};
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_SHIFT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, SetWindowsHookExW, HC_ACTION, WH_KEYBOARD,
    };

    /// An extended key (the arrows, Home/End, the numpad's Enter) shares its
    /// scan code with a key that is not extended. None of them are in the
    /// table, so rather than encode the flag this records "not a key we
    /// name" and lets gpui's own name through, which is already stable
    /// across layouts for those.
    const EXTENDED: i32 = -1;

    unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let flags = lparam as usize;
            // Bit 31 is the transition state: set on the way up.
            let going_down = (flags >> 31) & 1 == 0;
            if going_down {
                let extended = (flags >> 24) & 1 == 1;
                let scan = ((flags >> 16) & 0xFF) as i32;
                LAST.store(if extended { EXTENDED } else { scan }, Ordering::Relaxed);
                LAST_SHIFT.store(
                    unsafe { GetKeyState(VK_SHIFT as i32) } < 0,
                    Ordering::Relaxed,
                );
            }
        }
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    /// The hook lives for the life of the process; there is nothing to
    /// unhook it for, and the handle is deliberately dropped.
    pub fn install() {
        unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD,
                Some(hook),
                std::ptr::null_mut(),
                GetCurrentThreadId(),
            );
        }
    }

    /// Scan code set 1 to DOM `code` names, for the keys the keymap binds:
    /// the letters, the digit row and the punctuation around it. Anything
    /// else answers `None` and keeps gpui's name.
    pub fn dom_code(scan: u16) -> Option<&'static str> {
        Some(match scan {
            0x02 => "Digit1",
            0x03 => "Digit2",
            0x04 => "Digit3",
            0x05 => "Digit4",
            0x06 => "Digit5",
            0x07 => "Digit6",
            0x08 => "Digit7",
            0x09 => "Digit8",
            0x0A => "Digit9",
            0x0B => "Digit0",
            0x0C => "Minus",
            0x0D => "Equal",
            0x10 => "KeyQ",
            0x11 => "KeyW",
            0x12 => "KeyE",
            0x13 => "KeyR",
            0x14 => "KeyT",
            0x15 => "KeyY",
            0x16 => "KeyU",
            0x17 => "KeyI",
            0x18 => "KeyO",
            0x19 => "KeyP",
            0x1A => "BracketLeft",
            0x1B => "BracketRight",
            0x1E => "KeyA",
            0x1F => "KeyS",
            0x20 => "KeyD",
            0x21 => "KeyF",
            0x22 => "KeyG",
            0x23 => "KeyH",
            0x24 => "KeyJ",
            0x25 => "KeyK",
            0x26 => "KeyL",
            0x27 => "Semicolon",
            0x28 => "Quote",
            0x29 => "Backquote",
            0x2B => "Backslash",
            0x2C => "KeyZ",
            0x2D => "KeyX",
            0x2E => "KeyC",
            0x2F => "KeyV",
            0x30 => "KeyB",
            0x31 => "KeyN",
            0x32 => "KeyM",
            0x33 => "Comma",
            0x34 => "Period",
            0x35 => "Slash",
            // The extra key a 102-key European board has beside the left
            // Shift, which Turkish Q uses for `<` and `>`.
            0x56 => "IntlBackslash",
            _ => return None,
        })
    }
}

#[cfg(windows)]
pub use windows_hook::dom_code;

#[cfg(windows)]
pub fn install() {
    windows_hook::install();
}

/// gpui's modifiers with the ROLES on them rather than the raw keys.
///
/// The app is written against the Mac's split: `platform` is the app's
/// modifier, `control` is the terminal's. Windows has no Cmd key, so WHICH
/// keys carry those two roles is a setting, `keyboard.commandModifier`, and
/// `keymap::modifier_roles` is the rule it selects. The default splits the
/// two SIDES of Ctrl, left for the app and right for the terminal, because a
/// keyboard remapped into Mac order already puts left Ctrl where Cmd sits
/// and right Ctrl on Caps Lock.
///
/// Rewriting the modifiers once, where an event enters (`input.rs`), is what
/// keeps the other thirty-odd reads of `.platform` across this crate correct
/// without a cfg at each of them, and keeps the next one correct too.
///
/// gpui cannot answer this on its own: `current_modifiers` sets one
/// `control` flag from `VK_CONTROL`, which is either side, and the sides are
/// the whole point. They are read live rather than recorded by the keyboard
/// hook, because a modifier is a STATE while the hook records the last key
/// pressed, and a chord asks about both at the same moment.
///
/// A no-op everywhere else, so the Mac path is the code it always was and
/// the setting is read nowhere.
#[cfg(windows)]
pub fn roles(m: gpui::Modifiers, choice: CommandModifier) -> gpui::Modifiers {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_LCONTROL, VK_RCONTROL};
    // The high bit of GetKeyState is "down right now".
    let down = |vk: u16| unsafe { GetKeyState(vk as i32) } < 0;
    let (platform, control) = infiniterm_core::keymap::modifier_roles(
        choice,
        down(VK_LCONTROL),
        down(VK_RCONTROL),
        m.platform,
    );
    gpui::Modifiers {
        platform,
        control,
        ..m
    }
}

#[cfg(not(windows))]
pub fn roles(m: gpui::Modifiers, _choice: CommandModifier) -> gpui::Modifiers {
    m
}

/// No dead keys are recorded on Windows yet. Turkish Q, the layout this was
/// all written for, has none: it puts ç, ğ, ı, ö, ş and ü on keys of their
/// own. Turkish F and the US-International layout do have them, and when one
/// matters the signal is `ToUnicode` answering -1 for the key.
#[cfg(windows)]
pub fn last_dead() -> bool {
    false
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::dom_code;

    /// Every key the default keymap can bind must have a scan code here, or
    /// that binding silently falls back to gpui's layout-dependent name and
    /// breaks on a keyboard that is not US. This walks the real keymap
    /// rather than a list copied beside it.
    #[test]
    fn every_key_the_keymap_binds_has_a_scan_code() {
        // What the table can produce, as the keymap spells it.
        let mut named: Vec<String> = Vec::new();
        for scan in 0..=0xFFu16 {
            if let Some(code) = dom_code(scan) {
                named.push(
                    code.strip_prefix("Key")
                        .or_else(|| code.strip_prefix("Digit"))
                        .unwrap_or(code)
                        .to_ascii_lowercase(),
                );
            }
        }
        // The punctuation the keymap writes as the character itself.
        let punctuation = [
            ("bracketleft", "["),
            ("bracketright", "]"),
            ("minus", "-"),
            ("equal", "="),
            ("comma", ","),
            ("period", "."),
            ("slash", "/"),
            ("backslash", "\\"),
            ("semicolon", ";"),
            ("quote", "'"),
            ("backquote", "`"),
        ];
        for (name, ch) in punctuation {
            if named.iter().any(|n| n == name) {
                named.push(ch.to_string());
            }
        }

        let missing: Vec<&str> = infiniterm_core::keymap::DEFAULT_KEYMAP
            .iter()
            .filter_map(|(chord, _, _)| chord.rsplit('+').next())
            // Named keys (enter, escape, tab, the arrows, space) keep gpui's
            // name, which is already stable across layouts.
            .filter(|key| key.chars().count() == 1)
            .filter(|key| !named.iter().any(|n| n == key))
            .collect();
        assert!(missing.is_empty(), "no scan code for {missing:?}");
    }

    #[test]
    fn the_table_names_physical_keys_not_characters() {
        // Scan 0x10 is the key a US board prints Q on and a Turkish Q board
        // also prints Q on; scan 0x1A is US `[` and Turkish `ğ`. Both must
        // answer with the physical name.
        assert_eq!(dom_code(0x10), Some("KeyQ"));
        assert_eq!(dom_code(0x1A), Some("BracketLeft"));
        assert_eq!(dom_code(0x0D), Some("Equal"));
        assert_eq!(dom_code(0x0B), Some("Digit0"));
        // Nothing is claimed for a key the keymap never binds.
        assert_eq!(dom_code(0x00), None);
        assert_eq!(dom_code(0x3B), None);
    }
}
