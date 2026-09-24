//! The bundled icon font (`assets/fonts/SymbolsNerdFontMono-Regular.ttf`),
//! registered with CoreText for this process so every terminal font can fall
//! back to it (`terminal_body::term_font`). Nerd Font icons (the prompt's
//! powerline arrows, git and folder glyphs) live in Unicode's private use
//! area, which only a Nerd Font has, and CoreText does not borrow an
//! installed font for that range on its own.
//!
//! Why registration and not gpui's `add_fonts`: gpui builds a font's fallback
//! list by asking CoreText for each fallback BY NAME
//! (`CTFontDescriptorCreateWithNameAndSize`, gpui's `mac/open_type.rs`), and
//! `add_fonts` keeps the font in gpui's own memory store, where CoreText
//! cannot see it. The first attempt did only that; its unit test checked the
//! fallback was asked for, and the MacBook Air still drew boxes
//! (2026-09-24). Registered with CoreText, the name resolves.
//!
//! Called once from `main.rs` before the window opens. macOS only.
use core_graphics::data_provider::CGDataProvider;
use core_graphics::font::CGFont;
use foreign_types::ForeignType;

/// The font's bytes, compiled in so the app cannot lose them.
pub const ICON_FONT_TTF: &[u8] =
    include_bytes!("../../assets/fonts/SymbolsNerdFontMono-Regular.ttf");

/// The PostScript name CoreText resolves a fallback by; the family name
/// ("Symbols Nerd Font Mono") is not what the lookup matches.
pub const ICON_FONT_POSTSCRIPT: &str = "SymbolsNFM";

#[link(name = "CoreText", kind = "framework")]
extern "C" {
    // Not bound by the core-text crate. Registers a font for this process
    // only; `error` may be null.
    fn CTFontManagerRegisterGraphicsFont(
        font: core_graphics::sys::CGFontRef,
        error: *mut *const std::ffi::c_void,
    ) -> bool;
}

/// Makes the bundled icon font known to CoreText by name. Registering it a
/// second time fails harmlessly (it is already there), so this reports
/// whether the name resolves afterwards, not whether this call added it.
pub fn register() -> bool {
    // SAFETY: the slice is 'static, so the provider may keep pointing at it.
    let provider = unsafe { CGDataProvider::from_slice(ICON_FONT_TTF) };
    let Ok(font) = CGFont::from_data_provider(provider) else {
        eprintln!("[infiniterm/warn] the bundled icon font would not load");
        return false;
    };
    unsafe {
        let _ = CTFontManagerRegisterGraphicsFont(font.as_ptr(), std::ptr::null_mut());
    }
    // Kept for the life of the process: CoreText holds the registration.
    std::mem::forget(font);
    resolves()
}

/// Whether CoreText now finds the icon font by the name the fallback uses.
/// Asking for a name it does not know returns some other font, so the test
/// is the name that comes back.
pub fn resolves() -> bool {
    core_text::font::new_from_name(ICON_FONT_POSTSCRIPT, 12.)
        .is_ok_and(|f| f.postscript_name() == ICON_FONT_POSTSCRIPT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The failure the first attempt had: the fallback was asked for by a
    /// name CoreText could not resolve. After `register`, it resolves.
    #[test]
    fn coretext_finds_the_icon_font_by_its_fallback_name() {
        assert!(
            register(),
            "CoreText resolves {ICON_FONT_POSTSCRIPT} after registering"
        );
        let f = core_text::font::new_from_name(ICON_FONT_POSTSCRIPT, 12.).unwrap();
        assert_eq!(f.family_name(), "Symbols Nerd Font Mono");
        // And it holds a private-use glyph a prompt uses (U+E0B0, the
        // powerline arrow), which is the whole point.
        let mut glyph = [0u16; 1];
        let ok =
            unsafe { f.get_glyphs_for_characters([0xE0B0u16].as_ptr(), glyph.as_mut_ptr(), 1) };
        assert!(ok && glyph[0] != 0, "U+E0B0 has a glyph");
    }
}
