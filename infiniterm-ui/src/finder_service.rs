//! Finder's Services > "Open in infiniterm" (#344): the folders selected in
//! Finder arrive here and become terminal cards in the workspace on screen.
//!
//! `register` hands AppKit a service provider whose `openFolder:userData:
//! error:` (named by the `NSServices` entry `tools/bundle.sh` writes into
//! Info.plist) reads the file URLs off the pasteboard and queues the
//! directories. It does NOT touch the model: AppKit calls it from inside its
//! own run loop, where re-entering gpui could abort (the `RefCell already
//! borrowed` of native_menu.rs), so it only pushes onto a static queue.
//! `runtime.rs` drains `take()` on the 16 ms poll, once the model exists,
//! which also covers a cold start where the service arrives before the first
//! frame. Called from main.rs (register) and runtime.rs (take).
//!
//! Constraints: the entry exists only in a bundled app. If macOS starts a
//! second process for the service instead of messaging the running one, that
//! process exits on the socket lock (`main`) and the folders are lost; the
//! fallback then is to hand them to the running instance over its socket
//! (`ift terminal`).
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel, BOOL, YES};
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::CStr;
use std::sync::Mutex;

type Id = *mut Object;

/// Folders waiting for the poll loop.
static QUEUE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The folders the service has received since the last call.
pub fn take() -> Vec<String> {
    QUEUE
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}

/// Absolute directory paths among `urls`, in order. Pure, so the filter is
/// tested without AppKit: a file, a relative or empty string is dropped.
pub fn directories(urls: &[String]) -> Vec<String> {
    urls.iter()
        .filter(|p| std::path::Path::new(p).is_absolute() && std::path::Path::new(p).is_dir())
        .cloned()
        .collect()
}

/// `-[IftFinderService openFolder:userData:error:]`.
extern "C" fn open_folder(_this: &Object, _sel: Sel, pasteboard: Id, _data: Id, _error: Id) {
    let mut paths = Vec::new();
    unsafe {
        // Ask for NSURL objects: a folder dragged from Finder is a file URL.
        let classes: Id = msg_send![class!(NSArray), arrayWithObject: class!(NSURL)];
        let urls: Id = msg_send![pasteboard, readObjectsForClasses: classes options: std::ptr::null_mut::<Object>()];
        if !urls.is_null() {
            let count: usize = msg_send![urls, count];
            for i in 0..count {
                let url: Id = msg_send![urls, objectAtIndex: i];
                let is_file: BOOL = msg_send![url, isFileURL];
                if is_file != YES {
                    continue;
                }
                let path: Id = msg_send![url, path];
                if path.is_null() {
                    continue;
                }
                let c: *const std::os::raw::c_char = msg_send![path, UTF8String];
                if !c.is_null() {
                    paths.push(CStr::from_ptr(c).to_string_lossy().into_owned());
                }
            }
        }
    }
    let dirs = directories(&paths);
    if dirs.is_empty() {
        return;
    }
    if let Ok(mut q) = QUEUE.lock() {
        q.extend(dirs);
    }
    // The service runs from Finder: bring the app forward so the new card is
    // seen.
    unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, activateIgnoringOtherApps: YES];
    }
}

/// Makes the provider and tells macOS to re-read the Services list. Before
/// the window opens, so a service message that launched the app finds a
/// receiver; the queue keeps whatever comes until the model can take it.
pub fn register() {
    let class = match Class::get("IftFinderService") {
        Some(c) => c,
        None => {
            let mut decl = ClassDecl::new("IftFinderService", class!(NSObject))
                .expect("IftFinderService is declared once");
            unsafe {
                decl.add_method(
                    sel!(openFolder:userData:error:),
                    open_folder as extern "C" fn(&Object, Sel, Id, Id, Id),
                );
            }
            decl.register()
        }
    };
    unsafe {
        let provider: Id = msg_send![class, new];
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, setServicesProvider: provider];
        // The nudge a first bundled launch needs for the item to show up.
        extern "C" {
            fn NSUpdateDynamicServices();
        }
        NSUpdateDynamicServices();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_absolute_directories_are_kept() {
        let dir = std::env::temp_dir().canonicalize().unwrap();
        let file = dir.join(format!("ift-finder-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let got = directories(&[
            dir.to_string_lossy().into_owned(),
            file.to_string_lossy().into_owned(),
            "relative".into(),
            String::new(),
        ]);
        let _ = std::fs::remove_file(&file);
        assert_eq!(got, vec![dir.to_string_lossy().into_owned()]);
    }
}
