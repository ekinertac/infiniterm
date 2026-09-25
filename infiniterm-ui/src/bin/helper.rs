//! The CEF helper process: renderer, GPU, utility. Never the browser. It
//! is its own binary because macOS runs each Chromium subprocess from the
//! `infiniterm Helper.app` bundles the bundler lays out, and the main
//! binary must not carry gpui into every renderer. Mirrors cef-rs's
//! cefsimple_helper and the spikes' helpers.
//!
//! macOS ONLY, and not by omission: Windows re-executes the MAIN exe for
//! every Chromium subprocess, which `process::early` already handles by
//! running `execute_process` before gpui starts. There is nothing for a
//! second binary to do there, and the two things this one leans on, the
//! sandbox and the framework loader, are both macOS-side of the cef crate.
//! It still builds to something on Windows because a `[[bin]]` cannot be
//! switched off per target, only per feature.
#[cfg(target_os = "macos")]
use cef::{args::Args, *};

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("infiniterm-helper is macOS only; every other platform re-executes the app itself");
    std::process::exit(2);
}

#[cfg(target_os = "macos")]
fn main() {
    let args = Args::new();
    let _sandbox = {
        let mut sandbox = cef::sandbox::Sandbox::new();
        sandbox.initialize(args.as_main_args());
        sandbox
    };
    let _loader = {
        let loader = library_loader::LibraryLoader::new(&std::env::current_exe().unwrap(), true);
        assert!(loader.load());
        loader
    };
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    execute_process(
        Some(args.as_main_args()),
        None::<&mut App>,
        std::ptr::null_mut(),
    );
}
