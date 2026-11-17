//! Progress of the web view's downloads (#819, `browser.engine` = `webview`):
//! bytes received and bytes expected for every download in flight, for the
//! one status bar readout (`infiniterm_core::download_status`).
//!
//! The problem: wry hands its download handlers only the address and the
//! destination, never the `WKDownload`, and a `WKDownload` is where the
//! progress lives (`progress`, an NSProgress with `completedUnitCount` and
//! `totalUnitCount`). wry's own delegate class, `WryDownloadDelegate`, is told
//! about each download in `download:decideDestinationUsingResponse:
//! suggestedFilename:completionHandler:`, which is also where wry calls our
//! "started" handler. So that one method's implementation is replaced by one
//! that keeps the download (and the response's announced size) and then calls
//! wry's own, unchanged. The started handler, which runs inside it, claims
//! the kept download under the download's address; the completed handler
//! (success, failure or cancel alike) lets it go.
//!
//! Chesterton's fence for the swizzle: it depends on wry's class and method
//! name; if a wry update renames either, `install` finds nothing, says so on
//! stderr, and the readout simply never shows (the downloads still work).
//!
//! Where it fits: `webview.rs` installs it with the first page and calls
//! `on_started` / `on_finished` from its download handlers; `browsers.rs`
//! reads `snapshot` each frame and `wants_frame` asks for the frames.
//! Constraint: main thread only (thread-locals, AppKit objects).
use infiniterm_core::download_status::DownloadProgress;
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Sel},
};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

/// How often a download in progress asks for a frame, ms: the readout only
/// needs to move a few times a second, and a frame every 16 ms for a long
/// download would be the cost of an animation.
const FRAME_EVERY_MS: f64 = 250.;

struct Tracked {
    key: String,
    download: Retained<AnyObject>,
    /// `NSURLResponse.expectedContentLength`: -1 when the server gave none.
    expected: i64,
}

thread_local! {
    static PENDING: RefCell<Option<(Retained<AnyObject>, i64)>> = const { RefCell::new(None) };
    static ACTIVE: RefCell<Vec<Tracked>> = const { RefCell::new(Vec::new()) };
    static LAST_FRAME_MS: Cell<f64> = const { Cell::new(0.) };
}

/// wry's implementation of the method, kept as a plain address.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);

type DecideDestination = unsafe extern "C-unwind" fn(
    *mut AnyObject,
    Sel,
    *mut AnyObject,
    *mut AnyObject,
    *mut AnyObject,
    *mut c_void,
);

/// Replaces wry's method with `decide`, once. Called after the first page is
/// built, which is when wry's class exists.
pub fn install() {
    if ORIGINAL.load(Ordering::Relaxed) != 0 {
        return;
    }
    // objc2 registers wry's class under its module path and crate version
    // ("wry::wkwebview::class::...::WryDownloadDelegate0.57.2"), so it is
    // found by the name's ending, not looked up by name.
    let found = AnyClass::classes().iter().copied().find(|c| {
        c.name()
            .to_str()
            .is_ok_and(|n| n.contains("WryDownloadDelegate"))
    });
    let Some(cls) = found else {
        eprintln!("[infiniterm] webview: no WryDownloadDelegate; download progress is off");
        return;
    };
    let sel =
        objc2::sel!(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:);
    // SAFETY: `cls` is a live class; `decide` has exactly the signature of
    // the method it replaces (self, _cmd, download, response, filename, block),
    // and the type string says the same; the old implementation is kept and
    // called from `decide`, so wry's behaviour is unchanged.
    unsafe {
        let Some(method) = cls.instance_method(sel) else {
            eprintln!(
                "[infiniterm] webview: wry's download method is gone; download progress is off"
            );
            return;
        };
        let old = method.implementation();
        ORIGINAL.store(old as usize, Ordering::Relaxed);
        let new: unsafe extern "C-unwind" fn() = std::mem::transmute(decide as DecideDestination);
        method.set_implementation(new);
    }
}

unsafe extern "C-unwind" fn decide(
    this: *mut AnyObject,
    sel: Sel,
    download: *mut AnyObject,
    response: *mut AnyObject,
    name: *mut AnyObject,
    handler: *mut c_void,
) {
    // SAFETY: WebKit passes live objects on the main thread; `response` is an
    // NSURLResponse, which answers `expectedContentLength`.
    unsafe {
        let expected: i64 = msg_send![response, expectedContentLength];
        if let Some(d) = Retained::retain(download) {
            PENDING.with(|p| *p.borrow_mut() = Some((d, expected)));
        }
        // SAFETY: `ORIGINAL` holds wry's implementation of this very method,
        // stored by `install` before `decide` could run.
        let orig: DecideDestination = std::mem::transmute(ORIGINAL.load(Ordering::Relaxed));
        orig(this, sel, download, response, name, handler);
    }
    PENDING.with(|p| p.borrow_mut().take());
}

/// From wry's "started" handler, which runs inside `decide`: the download
/// that was just kept is claimed under its address.
pub fn on_started(url: &str) {
    if let Some((download, expected)) = PENDING.with(|p| p.borrow_mut().take()) {
        ACTIVE.with(|a| {
            a.borrow_mut().push(Tracked {
                key: url.to_string(),
                download,
                expected,
            })
        });
    }
}

/// From wry's "completed" handler: finished, failed or cancelled, it leaves
/// the readout.
pub fn on_finished(url: &str) {
    ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        if let Some(i) = a.iter().position(|t| t.key == url) {
            a.remove(i);
        }
    });
}

/// Received and expected bytes of every download in flight.
pub fn snapshot() -> Vec<DownloadProgress> {
    ACTIVE.with(|a| {
        a.borrow()
            .iter()
            .map(|t| {
                // SAFETY: `download` is a live WKDownload; `progress` is its
                // NSProgress, whose unit counts are `int64_t`.
                let (done, total): (i64, i64) = unsafe {
                    let p: *mut AnyObject = msg_send![&*t.download, progress];
                    if p.is_null() {
                        (0, -1)
                    } else {
                        (
                            msg_send![p, completedUnitCount],
                            msg_send![p, totalUnitCount],
                        )
                    }
                };
                let total = [total, t.expected].into_iter().find(|n| *n > 0);
                DownloadProgress {
                    received: done.max(0) as u64,
                    total: total.map(|n| n as u64),
                }
            })
            .collect()
    })
}

/// Whether a frame is due for the readout: a download is going and the last
/// frame it asked for was `FRAME_EVERY_MS` ago.
pub fn wants_frame(now_ms: f64) -> bool {
    if ACTIVE.with(|a| a.borrow().is_empty()) {
        return false;
    }
    LAST_FRAME_MS.with(|l| {
        if now_ms - l.get() >= FRAME_EVERY_MS {
            l.set(now_ms);
            true
        } else {
            false
        }
    })
}
