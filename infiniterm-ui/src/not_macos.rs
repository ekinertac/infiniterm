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
//! - `native_menu`: `pop_up` answers "dismissed". The app's own right-click
//!   menus no longer use it where there is no `NSMenu` (they are drawn by
//!   gpui, `menu_view.rs`), but the browser page's menu
//!   (`browsers.rs::show_page_menu`) still does, so on Windows that one
//!   does nothing until it is drawn the same way.
//! - `fullscreen`: the app menu's toggle and `ui.fullscreen` do nothing; gpui's
//!   own `toggle_fullscreen` on the window would do it.
//! - `icon_font`: the bundled Nerd Font is handed to gpui at start-up
//!   (`main.rs`, `add_fonts`) and the terminal paints icons from it
//!   (`terminal_body::icon_glyph`); only `ICON_FONT_FAMILY` and the bytes
//!   live here.
//! - `instance_icon`: no Dock badge; Windows has taskbar overlays instead.
//! - `finder_service`: no "open folder in infiniterm" service.
//! - `title_drag`: an AppKit view patch; Windows draws its own caption.
//! - `middle_drag` has no stand-in at all: its one call is macOS-only.
//!
//! Related: the real modules beside this file; docs/windows-handoff.md.

// A stand-in exists for the whole API of its module, whether or not this
// build still reaches every name; that is the point of keeping them equal.
#![allow(dead_code)]

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
    /// Windows bundles the font and hands it to gpui at start-up
    /// (`main.rs`, `add_fonts`); icons are then painted from glyphs shaped
    /// by `ICON_FONT_FAMILY` on their own (`terminal_body::icon_glyph`),
    /// because gpui's Windows fallback list looks in the SYSTEM font
    /// collection only and cannot see a font added this way, and a line
    /// that mixes the two collections loses one of them. Other platforms
    /// have no bundled copy.
    #[cfg(windows)]
    pub const ICON_FONT_TTF: &[u8] =
        include_bytes!("../../assets/fonts/SymbolsNerdFontMono-Regular.ttf");
    #[cfg(not(windows))]
    pub const ICON_FONT_TTF: &[u8] = &[];
    pub const ICON_FONT_POSTSCRIPT: &str = "SymbolsNFM";
    /// The family name in the font's own name table, which is what gpui's
    /// DirectWrite lookup matches (the PostScript name above is CoreText's).
    pub const ICON_FONT_FAMILY: &str = "Symbols Nerd Font Mono";

    /// True, so the start-up warning about a missing bundled font does not
    /// fire on a platform where the font is not bundled yet. On Windows the
    /// real work is `add_fonts` in `main.rs`, which has the `App` this lacks.
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
