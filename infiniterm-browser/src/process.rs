//! CEF, once per process. `early` runs before anything else in `main`:
//! it loads the framework from the bundle and lets a helper invocation
//! (renderer, GPU, utility) run and exit. `Process::start` initialises
//! the browser process inside gpui's run closure, after
//! `app_protocol::install`; the ui then calls `pump` from a 4 ms task and
//! `stop` on quit.
//!
//! The profile (cache path) lives in the app's data directory, so a test
//! run on `INFINITERM_DATA_DIR` gets its own; the two Anthropic native
//! messaging manifests are copied from Chrome's directory into it at every
//! start, so a Claude Code update is picked up on the next launch. The
//! extension is loaded unpacked from `<data>/browser/extension`, seeded
//! from Chrome's copy when absent. A missing extension is a browser
//! without Claude, not a failure; a missing framework (the bare binary,
//! outside a bundle) is an app without browser cards.
use cef::{args::Args, *};
use std::path::{Path, PathBuf};

/// The Claude in Chrome extension's store id, which the unpacked copy keeps
/// because its manifest carries the `key`.
const EXTENSION_ID: &str = "fcoeoabgfenejglbffodgkkbkcdhcgfn";

pub fn browser_dir() -> PathBuf {
    infiniterm_core::paths::app_support_dir().join("browser")
}

fn chrome_support() -> PathBuf {
    infiniterm_core::paths::home_dir().join("Library/Application Support/Google/Chrome")
}

/// Chrome's newest installed copy of the extension, if any.
fn chrome_extension() -> Option<PathBuf> {
    let dir = chrome_support()
        .join("Default/Extensions")
        .join(EXTENSION_ID);
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("manifest.json").is_file())
        .collect();
    versions.sort();
    versions.pop()
}

/// Regular files and directories only: a profile holds sockets and locks
/// (`Singleton*`) that must not travel, and a copy that stops at one of
/// them would leave half a profile.
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)?.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("Singleton") {
            continue;
        }
        let target = to.join(&name);
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// The extension and the native messaging manifests in place under
/// `data`. Returns the extension directory when there is one to load.
pub fn seed(data: &Path) -> Option<PathBuf> {
    let ext = data.join("extension");
    if !ext.join("manifest.json").is_file() {
        match chrome_extension() {
            Some(src) => {
                if let Err(e) = copy_dir(&src, &ext) {
                    eprintln!("[infiniterm/warn] could not copy the extension: {e}");
                }
            }
            None => eprintln!(
                "[infiniterm] no Claude in Chrome extension found in Chrome; the browser runs without it"
            ),
        }
    }
    // The first profile on this Mac is the spike's, which carries the
    // claude.ai sign-in the extension connects with; without it the
    // extension has to be signed in again inside a card. One-time, and
    // only when there is no profile yet.
    let profile = data.join("profile");
    if !profile.join("Default").is_dir() {
        let spike = infiniterm_core::paths::home_dir()
            .join("Code/infini-rust/spikes/cef-extension/profile");
        if spike.join("Default").is_dir() {
            match copy_dir(&spike, &profile) {
                Ok(()) => {
                    eprintln!("[infiniterm] browser profile seeded from the spike's (signed-in extension)");
                }
                Err(e) => eprintln!("[infiniterm/warn] could not copy the spike's profile: {e}"),
            }
        }
    }
    let hosts = profile.join("NativeMessagingHosts");
    let _ = std::fs::create_dir_all(&hosts);
    let src = chrome_support().join("NativeMessagingHosts");
    if let Ok(entries) = std::fs::read_dir(&src) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("com.anthropic.") && name.ends_with(".json") {
                let _ = std::fs::copy(e.path(), hosts.join(&name));
            }
        }
    }
    ext.join("manifest.json").is_file().then_some(ext)
}

wrap_app! {
    pub struct AppBuilder {
        extension: Option<String>,
    }

    impl App {
        fn on_before_command_line_processing(&self, _process_type: Option<&CefStringUtf16>, command_line: Option<&mut CommandLine>) {
            let Some(cmd) = command_line else { return };
            cmd.append_switch(Some(&"no-startup-window".into()));
            cmd.append_switch(Some(&"noerrdialogs".into()));
            cmd.append_switch(Some(&"use-mock-keychain".into()));
            if let Some(ext) = &self.extension {
                cmd.append_switch_with_value(Some(&"load-extension".into()), Some(&CefString::from(ext.as_str())));
            }
        }
    }
}

pub struct Process {
    args: Args,
    app: App,
    settings: Settings,
    _loader: library_loader::LibraryLoader,
}

/// Why CEF is not running, for the card to say.
#[derive(Debug)]
pub enum Unavailable {
    /// The framework is not beside the executable: not a bundle.
    NoFramework,
    /// This invocation was a helper process; it has run and `code` is
    /// what to exit with.
    Helper(i32),
}

/// Before gpui: load the framework and run as a helper if that is what
/// this invocation is.
pub fn early() -> Result<Process, Unavailable> {
    let loader = library_loader::LibraryLoader::new(&std::env::current_exe().unwrap(), false);
    if !loader.load() {
        return Err(Unavailable::NoFramework);
    }
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = Args::new();
    let data = browser_dir();
    let extension = seed(&data).map(|p| p.to_string_lossy().into_owned());
    let mut app = AppBuilder::new(extension);
    let ret = execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if ret >= 0 {
        return Err(Unavailable::Helper(ret));
    }
    let profile = data.join("profile");
    let settings = Settings {
        windowless_rendering_enabled: 1,
        external_message_pump: 1,
        cache_path: profile.to_string_lossy().as_ref().into(),
        log_file: data.join("cef.log").to_string_lossy().as_ref().into(),
        ..Default::default()
    };
    Ok(Process {
        args,
        app,
        settings,
        _loader: loader,
    })
}

impl Process {
    /// Inside gpui's run closure, after `app_protocol::install`.
    pub fn start(&mut self) -> bool {
        initialize(
            Some(self.args.as_main_args()),
            Some(&self.settings),
            Some(&mut self.app),
            std::ptr::null_mut(),
        ) == 1
    }
}

/// One turn of CEF's message loop; the ui calls it every 4 ms.
pub fn pump() {
    do_message_loop_work();
}

pub fn stop() {
    shutdown();
}
