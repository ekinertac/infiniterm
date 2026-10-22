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
//!
//! Linux has the same problem and no event monitor: `x11_keys` asks the X
//! server which key is down when the key-down arrives (`note_key_down`) and
//! shares the Windows scan code table. Native Wayland is not covered; see
//! that module and docs/agent/linux-port.md.
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
/// Whether that key-down was typed with AltGr (Windows only: the Ctrl+Alt a
/// chord reader would see is Right Alt plus an injected Left Ctrl, see
/// `infiniterm_core::altgr`). Never set elsewhere.
static LAST_ALTGR: AtomicBool = AtomicBool::new(false);
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

// The numbers here are Carbon virtual key codes; Windows and Linux share the
// set 1 table, tested at the bottom of this file.
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
    use super::{LAST, LAST_ALTGR, LAST_SHIFT};
    use std::sync::atomic::Ordering;
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, GetKeyboardLayout, VkKeyScanExW, HKL, VK_LCONTROL, VK_RCONTROL, VK_RMENU,
        VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, SetWindowsHookExW, HC_ACTION, WH_KEYBOARD,
    };

    /// Each layout's answer to "do you have an AltGr?", asked once: the probe
    /// is about two hundred `VkKeyScanExW` calls and a key-down must not pay
    /// for it. Keyed by the layout handle; a user has a handful of layouts.
    static LAYOUTS: Mutex<Vec<(isize, bool)>> = Mutex::new(Vec::new());

    fn layout_has_altgr(hkl: HKL) -> bool {
        let key = hkl as isize;
        let mut cache = LAYOUTS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(&(_, has)) = cache.iter().find(|(k, _)| *k == key) {
            return has;
        }
        let has = infiniterm_core::altgr::layout_has_altgr(|c| {
            // A character outside the BMP cannot be asked; the probe has none.
            let r = unsafe { VkKeyScanExW(c as u16, hkl) };
            // -1: this layout cannot type it.
            (r != -1).then(|| ((r >> 8) & 0xFF) as u8)
        });
        cache.push((key, has));
        has
    }

    /// Whether the keys down right now are AltGr (`core::altgr`): the current
    /// layout has one, Right Alt is down, and the Left Ctrl is the injected
    /// one (no Right Ctrl with it).
    fn altgr_now() -> bool {
        let down = |vk: u16| unsafe { GetKeyState(vk as i32) } < 0;
        let hkl = unsafe { GetKeyboardLayout(0) };
        infiniterm_core::altgr::altgr_in_effect(
            layout_has_altgr(hkl),
            down(VK_RMENU),
            down(VK_LCONTROL),
            down(VK_RCONTROL),
        )
    }

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
                LAST_ALTGR.store(altgr_now(), Ordering::Relaxed);
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
}

/// Scan code set 1 to DOM `code` names, for the keys the keymap binds: the
/// letters, the digit row and the punctuation around it. Anything else
/// answers `None` and keeps gpui's name.
///
/// Windows and Linux share it: a Linux evdev key code is the set 1 scan code
/// for every key in this table (KEY_Q is 16, KEY_EQUAL 13, KEY_102ND 86),
/// and an X11 key code is the evdev code plus 8 (`x11_keys`). Compiled on
/// the Mac too, unused, so its tests run on every platform.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn set1_dom_code(scan: u16) -> Option<&'static str> {
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

#[cfg(not(target_os = "macos"))]
pub use self::set1_dom_code as dom_code;

/// An X11 key code is the evdev code plus this (the X protocol reserves
/// codes 0 to 7).
// The X11 items below exist where `x11_keys` uses them, and for the tests,
// which run on every platform; anywhere else they would be dead code.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
const X11_KEYCODE_OFFSET: u16 = 8;
/// Left and right Shift as X11 key codes (evdev 42 and 54).
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
const X11_SHIFT_KEYS: [u16; 2] = [50, 62];

/// The one key the table names among the keys down in an X11 `QueryKeymap`
/// bitmap (bit `k` of the 32 bytes is key code `k`), as a set 1 scan code.
/// Modifiers are not in the table, so a chord leaves exactly one key. Two
/// named keys down at once (typing fast) is ambiguous and answers `None`,
/// which hands the keymap gpui's own name, as for a key the table lacks.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn pressed_scan(keys: &[u8; 32]) -> Option<u16> {
    let mut found = None;
    for code in X11_KEYCODE_OFFSET..256 {
        if !key_down_in(keys, code) {
            continue;
        }
        let scan = code - X11_KEYCODE_OFFSET;
        if set1_dom_code(scan).is_some() {
            if found.is_some() {
                return None;
            }
            found = Some(scan);
        }
    }
    found
}

/// Whether either Shift is down in a `QueryKeymap` bitmap.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn shift_down(keys: &[u8; 32]) -> bool {
    X11_SHIFT_KEYS.iter().any(|&code| key_down_in(keys, code))
}

#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn key_down_in(keys: &[u8; 32], code: u16) -> bool {
    keys[(code / 8) as usize] & (1 << (code % 8)) != 0
}

/// The Linux half. gpui's X11 and Wayland keystrokes come from xkb with the
/// key code dropped (`Keystroke::from_xkb` names the key through the layout
/// and clears Shift on a symbol), so the Mac's trap is here word for word.
/// There is no local event monitor to hook, so the key is read the other
/// way round: when a key-down reaches the app (`note_key_down`, called at
/// the top of `input.rs` and `own_window.rs`), ask the X server which keys
/// are down right now (`QueryKeymap`, one round trip on a connection of our
/// own) and keep the one the table names.
///
/// Native Wayland has no such query: there our connection, if any, reaches
/// XWayland, which sees no keys while a Wayland window has focus, so every
/// query answers `None` and the keymap gets gpui's layout names (the
/// Turkish Q `Cmd+=` bug comes back there). A key sent with no time between
/// press and release (`xdotool key`) can be released before the query runs
/// and also answers `None`; a test holds the key (`keydown`, `keyup`).
#[cfg(all(unix, not(target_os = "macos")))]
mod x11_keys {
    use super::{pressed_scan, shift_down, LAST, LAST_SHIFT};
    use std::sync::atomic::Ordering;
    use std::sync::OnceLock;
    use x11rb::protocol::xproto::ConnectionExt;
    use x11rb::rust_connection::RustConnection;

    static CONN: OnceLock<Option<RustConnection>> = OnceLock::new();

    /// Opens the connection once, from `DISPLAY`. No display (a Wayland
    /// session without XWayland) is one line on stderr and every later
    /// query answering `None`, never a failure.
    pub fn install() {
        CONN.get_or_init(|| match x11rb::connect(None) {
            Ok((conn, _screen)) => Some(conn),
            Err(err) => {
                eprintln!("[infiniterm] no X display for physical keys ({err}): layout key names are used");
                None
            }
        });
    }

    pub fn note_key_down() {
        let keys = CONN
            .get()
            .and_then(Option::as_ref)
            .and_then(|conn| conn.query_keymap().ok()?.reply().ok())
            .map(|reply| reply.keys);
        let (scan, shift) = match keys {
            Some(keys) => (pressed_scan(&keys), shift_down(&keys)),
            None => (None, false),
        };
        LAST.store(scan.map_or(-1, i32::from), Ordering::Relaxed);
        LAST_SHIFT.store(shift, Ordering::Relaxed);
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn install() {
    x11_keys::install();
}

/// Reads the physical key of the key-down being handled, where a platform
/// cannot record it before gpui sees it (Linux, `x11_keys`). Called first
/// thing in every `key_down`; the Mac's monitor and the Windows hook have
/// already recorded it, so it does nothing there.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn note_key_down() {
    x11_keys::note_key_down();
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
pub fn note_key_down() {}

/// gpui composes dead keys itself on Linux (xkbcommon's compose state in its
/// X11 and Wayland clients) and delivers the finished character, so nothing
/// here has to refuse a key for the composition to happen: on US
/// International the dead key arrives with no character and the next key as
/// the composed letter (checked, docs/agent/linux-port.md).
#[cfg(all(unix, not(target_os = "macos")))]
pub fn last_dead() -> bool {
    false
}

#[cfg(windows)]
pub fn install() {
    windows_hook::install();
}

/// A MOUSE event's modifiers with the roles on them: `platform` is the
/// app's modifier, `control` the terminal's. Off the Mac with
/// `keyboard.commandModifier` = `ctrl`, Ctrl plays Cmd's part for the mouse
/// (Ctrl+click opens a link, Ctrl+scroll zooms, Ctrl+drag pans, the Linux
/// and Windows convention). Keys are rewritten by the chord instead
/// (`input.rs::platform_keystroke`, `platform_keys.rs`), since plain Ctrl
/// on a key is the terminal's. The Mac, and `super`, keep the keys as they
/// are. Modifier-only events never come here: the card switcher commits
/// when the PHYSICAL Ctrl comes up.
pub fn roles(m: gpui::Modifiers, choice: CommandModifier) -> gpui::Modifiers {
    if cfg!(target_os = "macos") || choice == CommandModifier::Super {
        return m;
    }
    gpui::Modifiers {
        platform: m.platform || m.control,
        control: false,
        ..m
    }
}

/// Forgets the physical key recorded for this key-down, so the chord is
/// built from the keystroke alone. For an event `input.rs` rewrote into a
/// Mac chord whose Shift or key differs from what was pressed (Ctrl+Shift+T
/// is Cmd+T, F11 is Cmd+Ctrl+F): the physical record would put them back.
pub fn forget_last() {
    LAST.store(-1, Ordering::Relaxed);
}

/// Whether the most recent key-down was AltGr typing: its Ctrl and Alt are
/// the keyboard's own and must not count as a chord. Always false off Windows.
pub fn altgr_active() -> bool {
    LAST_ALTGR.load(Ordering::Relaxed)
}

/// The keystroke without the Ctrl and Alt AltGr put on it. Unchanged unless
/// the last key-down was AltGr typing, so a real Ctrl+Alt chord still counts
/// and every platform but Windows is untouched. The character the key types
/// travels in `key_char`, which this leaves alone.
pub fn without_altgr(k: &gpui::Keystroke) -> gpui::Keystroke {
    let mut k = k.clone();
    if altgr_active() {
        k.modifiers.control = false;
        k.modifiers.alt = false;
    }
    k
}

/// No dead keys are recorded on Windows yet. Turkish Q, the layout this was
/// all written for, has none: it puts ç, ğ, ı, ö, ş and ü on keys of their
/// own. Turkish F and the US-International layout do have them, and when one
/// matters the signal is `ToUnicode` answering -1 for the key.
#[cfg(windows)]
pub fn last_dead() -> bool {
    false
}

// The set 1 table and the X11 bitmap rules, which Windows and Linux use; on
// every platform, so a Mac run checks them too.
#[cfg(test)]
mod set1_tests {
    use super::{pressed_scan, set1_dom_code as dom_code, shift_down};

    /// A `QueryKeymap` bitmap with these X11 key codes down.
    fn keymap(down: &[u16]) -> [u8; 32] {
        let mut keys = [0u8; 32];
        for &code in down {
            keys[(code / 8) as usize] |= 1 << (code % 8);
        }
        keys
    }

    #[test]
    fn the_key_under_a_chord_is_found_with_its_modifiers_down() {
        // Super (133) + Shift (50) + the key US prints `=` on (evdev 13,
        // X 21), which Turkish Q prints `-` on: the physical key wins.
        let keys = keymap(&[133, 50, 21]);
        assert_eq!(pressed_scan(&keys).and_then(dom_code), Some("Equal"));
        assert!(shift_down(&keys));
        // Right Shift counts too; Control (37) is not Shift.
        assert!(shift_down(&keymap(&[62])));
        assert!(!shift_down(&keymap(&[37, 21])));
    }

    #[test]
    fn no_named_key_or_two_of_them_hands_the_keymap_gpuis_name() {
        assert_eq!(pressed_scan(&keymap(&[])), None);
        // Modifiers alone, and keys the table does not name (Return 36,
        // Left 113).
        assert_eq!(pressed_scan(&keymap(&[133, 50, 36, 113])), None);
        // Q (24) and W (25) at once: which one this event is cannot be told.
        assert_eq!(pressed_scan(&keymap(&[24, 25])), None);
        assert_eq!(
            pressed_scan(&keymap(&[24])).and_then(dom_code),
            Some("KeyQ")
        );
    }

    #[test]
    fn the_european_key_beside_left_shift_is_named() {
        // KEY_102ND is evdev 86, X 94: Turkish Q's `<` and `>`.
        assert_eq!(
            pressed_scan(&keymap(&[94])).and_then(dom_code),
            Some("IntlBackslash")
        );
    }

    /// The keys the scan code table can name, spelled as a chord spells them.
    fn scan_named_keys() -> Vec<String> {
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
        named
    }

    /// The chords Windows and Linux actually press (the table #537 builds
    /// from the default keymap, `platform_keys.rs`) use physical keys, and a
    /// key with no scan code falls back to gpui's layout-dependent name: the
    /// chord would then work on a US board and not on Turkish Q. The Mac
    /// keymap's keys are covered by the test below, but the platform table
    /// can move a chord to another key (Alt+digit for workspaces, F11), so it
    /// is walked as well.
    #[test]
    fn every_key_the_platform_chords_use_has_a_scan_code() {
        let named = scan_named_keys();
        let (table, problems) = infiniterm_core::platform_keys::PlatformKeys::build(
            infiniterm_core::keymap::DEFAULT_KEYMAP
                .iter()
                .map(|(chord, _, _)| *chord),
        );
        assert!(problems.is_empty(), "{problems:?}");
        let mut checked = 0;
        let mut missing: Vec<String> = Vec::new();
        for (mac, _, _) in infiniterm_core::keymap::DEFAULT_KEYMAP {
            let Some(platform) = table.platform(mac) else {
                continue;
            };
            let key = platform.rsplit('+').next().unwrap_or("");
            // Named keys (enter, escape, tab, the arrows, space, F11) keep
            // gpui's name, stable across layouts.
            if key.chars().count() != 1 {
                continue;
            }
            checked += 1;
            if !named.iter().any(|n| n == key) {
                missing.push(format!("{platform} (from {mac})"));
            }
        }
        assert!(checked > 40, "the walk saw only {checked} chords");
        assert!(missing.is_empty(), "no scan code for {missing:?}");
    }

    /// Every key the default keymap can bind must have a scan code here, or
    /// that binding silently falls back to gpui's layout-dependent name and
    /// breaks on a keyboard that is not US. This walks the real keymap
    /// rather than a list copied beside it.
    #[test]
    fn every_key_the_keymap_binds_has_a_scan_code() {
        let named = scan_named_keys();
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
