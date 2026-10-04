//! The Dock icon of a coloured window (#118, #160): the app's own icon with a
//! round badge in the host's colour, so two infiniterm tiles in the Dock are
//! told apart without reading a label.
//!
//! `NSApplication.applicationIconImage` is per process, and `ift connect`
//! starts the instance with `open -n`, so a remote instance's tile is its own.
//! The local instance is badged only when it has a colour (`ui.windowColor`).
//! A new NSImage the size of the app's icon, the icon drawn into it, the badge
//! over its bottom-right corner (a white ring keeps it readable on any colour).
//! The colour can change while the app runs (`window.color`, previewed on every
//! move of the highlight), so the ORIGINAL icon is kept on the first call and
//! every badge is drawn from it: drawing over the last result would stack
//! badges.
//!
//! Called by `runtime.rs::startup`. Related: `infiniterm-core/src/
//! remote_identity.rs` (the colour), `fullscreen.rs` (the same objc idiom).
//!
//! Non-obvious: nothing here can be checked by a unit test: it needs a running
//! app with a Dock. `tools/drive/connect.sh` opens the instance; looking at
//! the Dock is what checks it.

use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use infiniterm_core::remote_identity::Rgb;
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use std::sync::atomic::{AtomicUsize, Ordering};

type Id = *mut Object;

/// The app's own icon, retained on the first `set_badge` (0 until then).
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// The badge's diameter as a share of the icon's width, and its inset from the corner.
const BADGE_SHARE: f64 = 0.42;
const BADGE_INSET: f64 = 0.03;
/// The white ring's width as a share of the icon's width.
const RING_SHARE: f64 = 0.04;

/// The plain icon again (the colour was removed): the app's own icon, kept on
/// the first `set_badge`, or the default when none was ever badged.
pub fn clear_badge() {
    unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        match ORIGINAL.load(Ordering::SeqCst) {
            // Nothing was badged, so the icon is still the app's own.
            0 => {}
            kept => {
                let _: () = msg_send![app, setApplicationIconImage: kept as Id];
            }
        }
    }
}

pub fn set_badge(color: Rgb) {
    unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let icon: Id = match ORIGINAL.load(Ordering::SeqCst) {
            0 => {
                let current: Id = msg_send![app, applicationIconImage];
                if current.is_null() {
                    return;
                }
                let _: Id = msg_send![current, retain];
                ORIGINAL.store(current as usize, Ordering::SeqCst);
                current
            }
            kept => kept as Id,
        };
        let size: CGSize = msg_send![icon, size];
        let image: Id = msg_send![class!(NSImage), alloc];
        let image: Id = msg_send![image, initWithSize: size];
        let _: () = msg_send![image, lockFocus];
        let whole = CGRect::new(&CGPoint::new(0., 0.), &size);
        let _: () = msg_send![icon, drawInRect: whole];

        let d = size.width * BADGE_SHARE;
        let inset = size.width * BADGE_INSET;
        let badge = CGRect::new(
            &CGPoint::new(size.width - d - inset, inset),
            &CGSize::new(d, d),
        );
        let path: Id = msg_send![class!(NSBezierPath), bezierPathWithOvalInRect: badge];
        let channel = |v: u8| v as f64 / 255.;
        let fill: Id = msg_send![class!(NSColor),
            colorWithSRGBRed: channel(color.0)
            green: channel(color.1)
            blue: channel(color.2)
            alpha: 1.0f64];
        let white: Id = msg_send![class!(NSColor), whiteColor];
        let _: () = msg_send![fill, setFill];
        let _: () = msg_send![path, fill];
        let _: () = msg_send![white, setStroke];
        let _: () = msg_send![path, setLineWidth: size.width * RING_SHARE];
        let _: () = msg_send![path, stroke];

        let _: () = msg_send![image, unlockFocus];
        let _: () = msg_send![app, setApplicationIconImage: image];
    }
}
