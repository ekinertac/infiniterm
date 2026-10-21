//! What the macOS-only ui modules answer on every other platform: the same
//! public names, doing nothing, so no call site needs a `cfg`.
//!
//! Responsibility: keep the Windows build compiling while seven modules that
//! talk to AppKit and CoreText (finder_service, fullscreen, icon_font,
//! instance_icon, middle_drag, native_menu, title_drag) stay Mac code. Where
//! it fits: `main.rs` declares the real modules on macOS and imports these
//! under the same names elsewhere, so `crate::fullscreen::toggle()` resolves
//! either way. Depends on: only core types; nothing here links a native
//! library.
//!
//! What each stub leaves undone on Windows, and what would finish it:
//! - `native_menu`: no right-click menu. `pop_up` answers "dismissed". A
//!   Windows port needs `TrackPopupMenu` or an in-app menu drawn by gpui.
//! - `fullscreen`: the app menu's toggle and `ui.fullscreen` do nothing; gpui's
//!   own `toggle_fullscreen` on the window would do it.
//! - `icon_font`: the bundled Nerd Font is not registered, so prompt icons
//!   come from whatever Nerd Font the user installed; DirectWrite resolves
//!   fallbacks by family name and gpui's `add_fonts` is the way in.
//! - `instance_icon`: no Dock badge; Windows has taskbar overlays instead.
//! - `finder_service`: no "open folder in infiniterm" service.
//! - `middle_drag`, `title_drag`: AppKit view patches; Windows needs neither
//!   (the system draws the caption and gpui handles the middle button).
//!
//! Related: the real modules beside this file; docs/windows-handoff.md.

pub mod finder_service {
    /// No service delivers folders here.
    pub fn take() -> Vec<String> {
        Vec::new()
    }

    /// Same pure filter as the Mac module, so the one that matters (a path
    /// the app can open) is not different per platform.
    pub fn directories(urls: &[String]) -> Vec<String> {
        urls.iter()
            .filter(|p| std::path::Path::new(p).is_absolute() && std::path::Path::new(p).is_dir())
            .cloned()
            .collect()
    }

    pub fn register() {}
}

pub mod fullscreen {
    use infiniterm_core::config::FullscreenMode;

    pub fn set_mode(_mode: FullscreenMode) {}
    pub fn covering() -> bool {
        false
    }
    pub fn install() {}
    pub fn toggle() {}
}

pub mod icon_font {
    /// Empty: nothing is registered, so nothing can be drawn from it.
    pub const ICON_FONT_TTF: &[u8] = &[];
    pub const ICON_FONT_POSTSCRIPT: &str = "SymbolsNFM";

    /// True, so the start-up warning about a missing bundled font does not
    /// fire on a platform where the font is not bundled yet.
    pub fn register() -> bool {
        true
    }

    pub fn resolves() -> bool {
        true
    }
}

pub mod instance_icon {
    use infiniterm_core::remote_identity::Rgb;

    pub fn init() {}
    pub fn clear_badge() {}
    pub fn set_badge(_color: Rgb) {}
}

pub mod middle_drag {
    pub fn install() {}
}

pub mod title_drag {
    pub fn install() {}
    pub fn begin() {}
}

pub mod native_menu {
    /// One row of the menu; the shape the Mac module takes, so `menus.rs`
    /// builds the same rows for either.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Row {
        Item {
            tag: i64,
            title: String,
            enabled: bool,
            chord: Option<String>,
        },
        Submenu {
            title: String,
            rows: Vec<Row>,
        },
        Separator,
    }

    /// No native menu: always dismissed.
    pub fn pop_up(_rows: &[Row]) -> Option<i64> {
        None
    }
}
