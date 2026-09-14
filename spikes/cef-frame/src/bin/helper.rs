//! The CEF helper process: renderer, GPU, utility. Never the browser. It is
//! its own binary because macOS runs each Chromium subprocess from the
//! `<name> Helper.app` bundles the bundler lays out, and the main binary
//! must not carry gpui into every renderer. Mirrors cef-rs's cefsimple_helper.
use cef::{args::Args, *};

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
    execute_process(Some(args.as_main_args()), None::<&mut App>, std::ptr::null_mut());
}
