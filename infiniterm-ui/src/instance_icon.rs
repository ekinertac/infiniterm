//! The Dock icon (#118, #160, `ui.icon`): a base picture, and over it a round
//! badge in the window's colour for a remote, joined or coloured window, so
//! two infiniterm tiles in the Dock are told apart without reading a label.
//!
//! `NSApplication.applicationIconImage` is per process, and `ift connect`
//! starts the instance with `open -n`, so a remote instance's tile is its own.
//! The local instance is badged only when it has a colour (`ui.windowColor`).
//!
//! Two inputs, both kept here: the BASE picture (the bundle's own icon, or the
//! `filled` one when the ui theme says `ui.icon: filled`) and the BADGE colour.
//! Every change redraws from both (`apply`), so a badge never stacks on a
//! badge and a new base keeps the badge. The bundle's icon is captured once as
//! `ORIGINAL`, before anything is set, because `applicationIconImage` returns
//! the last image set. The colour can change while the app runs (`window.color`
//! is previewed on every move of the highlight), and so can the theme.
//!
//! Drawing: a new NSImage of a fixed large size (ICON_PT), the base drawn into
//! it, the badge over its bottom-right corner (a white ring keeps it readable
//! on any colour).
//!
//! Called by `runtime.rs` (startup and the `WindowColor` and `AppIcon`
//! effects). Related: `infiniterm-core/src/app_icon.rs` (the choice and where
//! the picture comes from), `remote_identity.rs` (the colour), `fullscreen.rs`
//! (the same objc idiom).
//!
//! Non-obvious: nothing here can be checked by a unit test: it needs a running
//! app with a Dock. The Finder, Launchpad and Cmd+Tab list read the bundle's
//! file and never follow `ui.icon`. `INFINITERM_KEYLOG` logs the size of the
//! image that was set, which is how a scratch run is checked without a
//! screenshot of the Dock.

use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use infiniterm_core::app_icon::{AppIcon, IconSource};
use infiniterm_core::remote_identity::Rgb;
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

type Id = *mut Object;

/// The bundle's own icon, retained on the first call (0 until then).
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// The picture of `ui.icon` when it is not the bundle's (0: the bundle's).
static BASE: AtomicUsize = AtomicUsize::new(0);
/// The badge colour in force (None: no badge).
static BADGE: Mutex<Option<Rgb>> = Mutex::new(None);

/// The pictures compiled in, for a bundle without `Resources/icons/`.
const FILLED_PNG: &[u8] = include_bytes!("../../assets/icons/filled.png");

/// The badge's diameter as a share of the icon's width, and its inset from the corner.
const BADGE_SHARE: f64 = 0.42;
const BADGE_INSET: f64 = 0.03;
/// The image is drawn at this size in points, not at `[icon size]`:
/// `applicationIconImage` reports a small size (about 32 pt), so an image
/// made to match it is one ~64 px bitmap from the icns's smallest
/// representation, and the Cmd+Tab switcher, which shows the icon near 144 px
/// on a 4K display, stretched it into stair-step pixels. At 512 pt,
/// `drawInRect` picks the icns's 512@2x representation (1024 px on Retina);
/// the Dock scales that down and stays sharp.
const ICON_PT: f64 = 512.0;
/// The white ring's width as a share of the icon's width.
const RING_SHARE: f64 = 0.04;

/// The app's own icon at the size the Cmd+Tab switcher can enlarge cleanly.
/// `NSRunningApplication.icon` of this process reports 32 x 32 pt although
/// the image holds representations up to 2048 px, and the switcher picks a
/// representation for that declared size and scales it up: a pixelated icon
/// next to every other app's. A copy told to be `ICON_PT` points wide is
/// chosen at its large representation instead. Null when there is no icon.
unsafe fn original() -> Id {
    let kept = ORIGINAL.load(Ordering::SeqCst);
    if kept != 0 {
        return kept as Id;
    }
    let app: Id = msg_send![class!(NSApplication), sharedApplication];
    let current: Id = msg_send![app, applicationIconImage];
    if current.is_null() {
        return std::ptr::null_mut();
    }
    let copy: Id = msg_send![current, copy];
    let _: () = msg_send![copy, setSize: CGSize::new(ICON_PT, ICON_PT)];
    ORIGINAL.store(copy as usize, Ordering::SeqCst);
    copy
}

/// The picture under the badge: the theme's, else the bundle's.
unsafe fn base() -> Id {
    match BASE.load(Ordering::SeqCst) {
        0 => original(),
        set => set as Id,
    }
}

/// Sets the Dock image from the base and the badge in force.
unsafe fn apply() {
    let icon = base();
    if icon.is_null() {
        return;
    }
    let app: Id = msg_send![class!(NSApplication), sharedApplication];
    let badge = *BADGE.lock().unwrap();
    let shown = match badge {
        None => icon,
        Some(color) => badged(icon, color),
    };
    let _: () = msg_send![app, setApplicationIconImage: shown];
    if std::env::var_os("INFINITERM_KEYLOG").is_some() {
        let now: Id = msg_send![app, applicationIconImage];
        let size: CGSize = msg_send![now, size];
        eprintln!(
            "[infiniterm/icon] dock image {}x{} pt, base {}, badge {}",
            size.width,
            size.height,
            if BASE.load(Ordering::SeqCst) == 0 {
                "bundle"
            } else {
                "theme"
            },
            badge.is_some()
        );
    }
}

/// Makes the Dock icon the bundle's, at the large size, before anything sets
/// another one. Called once at startup; harmless to call again.
pub fn init() {
    unsafe {
        original();
        apply();
    }
}

/// The plain icon again (the colour was removed).
pub fn clear_badge() {
    *BADGE.lock().unwrap() = None;
    unsafe { apply() }
}

pub fn set_badge(color: Rgb) {
    *BADGE.lock().unwrap() = Some(color);
    unsafe { apply() }
}

/// The picture `ui.icon` names, under the badge in force. `Bundle` goes back
/// to the bundle's own icon.
pub fn set_base(icon: AppIcon) {
    let icons_dir = std::env::current_exe().ok().and_then(|exe| {
        let dir = infiniterm_core::paths::shipped_resource(exe.parent()?, "icons", true);
        Some(dir)
    });
    let source = infiniterm_core::app_icon::source(icon, icons_dir.as_deref(), |p| p.is_file());
    unsafe {
        // Captured first: once a base is set, `applicationIconImage` is it.
        original();
        let image = match source {
            IconSource::Bundle => std::ptr::null_mut(),
            IconSource::File(path) => match std::fs::read(&path) {
                Ok(bytes) => from_png(&bytes),
                Err(_) => from_png(compiled(icon)),
            },
            IconSource::Compiled(which) => from_png(compiled(which)),
        };
        let old = BASE.swap(image as usize, Ordering::SeqCst);
        apply();
        if old != 0 {
            let _: () = msg_send![old as Id, release];
        }
    }
}

fn compiled(icon: AppIcon) -> &'static [u8] {
    match icon {
        AppIcon::Filled => FILLED_PNG,
        // `default` is the bundle's icon and is never drawn from a file.
        AppIcon::Default => &[],
    }
}

/// A retained NSImage of `ICON_PT` points from PNG bytes; null if they are bad.
unsafe fn from_png(bytes: &[u8]) -> Id {
    if bytes.is_empty() {
        return std::ptr::null_mut();
    }
    let data: Id = msg_send![class!(NSData), dataWithBytes: bytes.as_ptr() length: bytes.len()];
    let image: Id = msg_send![class!(NSImage), alloc];
    let image: Id = msg_send![image, initWithData: data];
    if !image.is_null() {
        let _: () = msg_send![image, setSize: CGSize::new(ICON_PT, ICON_PT)];
    }
    image
}

/// A new image: `icon` with the colour badge over its bottom-right corner.
unsafe fn badged(icon: Id, color: Rgb) -> Id {
    let size = CGSize::new(ICON_PT, ICON_PT);
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
    // The app keeps the one it is given; each colour preview makes a new one.
    let _: Id = msg_send![image, autorelease];
    image
}
