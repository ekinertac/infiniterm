//! A native macOS context menu (#276): builds an `NSMenu`, pops it up at the
//! pointer and returns the tag of the item the user chose.
//!
//! Called by `menus.rs`. The menus themselves are data in
//! `infiniterm-core/src/context_menu.rs`; this file is only the presenter, the
//! one place AppKit is touched for menus. Related:
//! `fullscreen.rs`, `middle_drag.rs` and `instance_icon.rs` use the same
//! `objc` 0.2 messaging.
//!
//! Constraint: `popUpMenuPositioningItem:` runs its own tracking loop and
//! returns when the menu closes, so it blocks the caller, and the loop
//! re-enters gpui. Called from inside a gpui handler (which holds the app
//! borrowed) it aborts with "RefCell already borrowed" (seen in the spike,
//! #278): it must be called from a spawned task, as the open panel is. The
//! choice is returned, not run, so the caller runs it once the menu is gone.

use core_graphics::geometry::CGPoint;
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

type Id = *mut Object;

/// Nothing chosen (a dismissed menu).
const NONE: i64 = i64::MIN;

/// What the menu's items last reported through `pick:`.
static CHOSEN: AtomicI64 = AtomicI64::new(NONE);
/// The one `IftMenuTarget` instance, as an address (a raw pointer is not Send).
static TARGET: AtomicUsize = AtomicUsize::new(0);

/// One row of the menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Item {
        /// Handed back when chosen: the caller's index of the row.
        tag: i64,
        title: String,
        enabled: bool,
        /// The chord bound to the command, as the keymap spells it: drawn at
        /// the right edge as a Mac shortcut (`shortcuts::mac_key_equivalent`).
        chord: Option<String>,
    },
    /// One level of submenu (a row that opens more rows).
    Submenu {
        title: String,
        rows: Vec<Row>,
    },
    Separator,
}

fn ns_string(s: &str) -> Id {
    let c = CString::new(s.replace('\0', "")).unwrap_or_default();
    unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
}

/// `-[IftMenuTarget pick:]`: records the chosen item's tag.
extern "C" fn pick(_this: &Object, _sel: Sel, sender: Id) {
    let tag: i64 = unsafe { msg_send![sender, tag] };
    CHOSEN.store(tag, Ordering::SeqCst);
}

fn target() -> Id {
    let existing = TARGET.load(Ordering::SeqCst);
    if existing != 0 {
        return existing as Id;
    }
    let class = match Class::get("IftMenuTarget") {
        Some(c) => c,
        None => {
            let mut decl = ClassDecl::new("IftMenuTarget", class!(NSObject))
                .expect("IftMenuTarget is declared once");
            unsafe {
                decl.add_method(sel!(pick:), pick as extern "C" fn(&Object, Sel, Id));
            }
            decl.register()
        }
    };
    let instance: Id = unsafe { msg_send![class, new] };
    TARGET.store(instance as usize, Ordering::SeqCst);
    instance
}

/// Builds `rows` into `menu`, a submenu's rows recursively.
unsafe fn fill(menu: Id, rows: &[Row], target: Id) {
    for row in rows {
        match row {
            Row::Separator => {
                let sep: Id = msg_send![class!(NSMenuItem), separatorItem];
                let _: () = msg_send![menu, addItem: sep];
            }
            Row::Submenu { title, rows } => {
                let item: Id = msg_send![class!(NSMenuItem), alloc];
                let item: Id = msg_send![item, initWithTitle: ns_string(title)
                                          action: std::ptr::null_mut::<Object>()
                                   keyEquivalent: ns_string("")];
                let sub: Id = msg_send![class!(NSMenu), alloc];
                let sub: Id = msg_send![sub, initWithTitle: ns_string(title)];
                let _: () = msg_send![sub, setAutoenablesItems: false];
                fill(sub, rows, target);
                let _: () = msg_send![item, setSubmenu: sub];
                let _: () = msg_send![sub, release];
                let _: () = msg_send![menu, addItem: item];
                let _: () = msg_send![item, release];
            }
            Row::Item {
                tag,
                title,
                enabled,
                chord,
            } => {
                let key = chord
                    .as_deref()
                    .and_then(infiniterm_core::shortcuts::mac_key_equivalent);
                let item: Id = msg_send![class!(NSMenuItem), alloc];
                let item: Id = msg_send![item, initWithTitle: ns_string(title)
                                          action: sel!(pick:)
                                   keyEquivalent: ns_string(key.as_ref().map_or("", |k| k.0.as_str()))];
                if let Some((_, mask)) = &key {
                    let _: () = msg_send![item, setKeyEquivalentModifierMask: *mask];
                }
                let _: () = msg_send![item, setTag: *tag];
                let _: () = msg_send![item, setTarget: target];
                let _: () = msg_send![item, setEnabled: *enabled];
                let _: () = msg_send![menu, addItem: item];
                let _: () = msg_send![item, release];
            }
        }
    }
}

/// Pops the menu up at the mouse and blocks until it closes. `Some(tag)` of
/// the chosen row, `None` when it was dismissed.
pub fn pop_up(rows: &[Row]) -> Option<i64> {
    unsafe {
        CHOSEN.store(NONE, Ordering::SeqCst);
        let menu: Id = msg_send![class!(NSMenu), alloc];
        let menu: Id = msg_send![menu, initWithTitle: ns_string("")];
        let _: () = msg_send![menu, setAutoenablesItems: false];
        fill(menu, rows, target());
        // Screen coordinates, bottom-left origin: where the pointer is now.
        let at: CGPoint = msg_send![class!(NSEvent), mouseLocation];
        let nil: Id = std::ptr::null_mut();
        let _shown: bool =
            msg_send![menu, popUpMenuPositioningItem: nil atLocation: at inView: nil];
        let _: () = msg_send![menu, release];
    }
    match CHOSEN.swap(NONE, Ordering::SeqCst) {
        NONE => None,
        tag => Some(tag),
    }
}
