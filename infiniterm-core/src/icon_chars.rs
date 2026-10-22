//! Which terminal characters are Nerd Font icons.
//!
//! Responsibility: the one question "is this character drawn from the icon
//! font?", for a platform whose text system cannot fall back to a font the
//! app bundled. Nerd Font glyphs (powerline arrows, git and folder symbols,
//! the Material icon set) live in Unicode's private use areas, which no
//! ordinary font fills, so a terminal row has to get them from the icon font.
//!
//! Where it fits: macOS registers the bundled font with CoreText and names it
//! as a fallback, which works there. gpui on Windows resolves a fallback by
//! NAME in the SYSTEM font collection only, so a font added with `add_fonts`
//! is invisible to the fallback list; and one layout cannot hold both that
//! font (custom collection) and the terminal's (system collection). So on
//! Windows `terminal_body::shape_row` leaves a space in the line where an icon
//! is and `terminal_body::icon_glyph` paints the glyph from a layout of its
//! own. This file says which characters those are.
//!
//! Related: infiniterm-ui/src/terminal_body.rs (`SEPARATE_ICONS`, the
//! caller), infiniterm-ui/src/not_macos.rs (`icon_font`, the family name and
//! the font bytes), infiniterm-ui/src/icon_font.rs (the macOS way).
//!
//! Constraint: this must agree with what the bundled font actually covers; a
//! character it claims and the font lacks draws nothing, which is better than
//! the box the terminal showed before but is still a gap.

/// Whether `ch` is in a private use area a Nerd Font fills: the BMP one
/// (U+E000 to U+F8FF: powerline, Font Awesome, Devicons, Octicons and the
/// rest) and the supplementary plane A one (U+F0000 to U+FFFFD, where Nerd
/// Fonts 3 keeps the Material Design icons).
pub fn is_icon(ch: char) -> bool {
    matches!(ch as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_private_use_areas_are_icons_and_ordinary_text_is_not() {
        // Powerline arrow, Font Awesome, a Material Design icon (plane 15).
        for ch in ['\u{E0B0}', '\u{F015}', '\u{F0001}', '\u{F8FF}', '\u{E000}'] {
            assert!(is_icon(ch), "{ch:?}");
        }
        for ch in [
            'a', ' ', 'ç', '→', '❯', '⚡', '\u{D7FF}', '\u{F900}', '日', '🙂',
        ] {
            assert!(!is_icon(ch), "{ch:?}");
        }
        // The ends of the supplementary range and the first code point past it.
        assert!(is_icon('\u{FFFFD}'));
        assert!(!is_icon('\u{FFFFE}'));
        assert!(!is_icon('\u{EFFFF}'));
    }

    // What the prompt that started this uses: oh-my-posh's folder, home,
    // clock and powerline separators are all private use characters, and its
    // plain arrows and bolt are not (the font draws those itself).
    #[test]
    fn the_icons_of_a_real_prompt_are_told_from_its_plain_symbols() {
        for ch in ['\u{F07B}', '\u{F015}', '\u{F017}', '\u{E0B0}', '\u{E0B2}'] {
            assert!(is_icon(ch), "{ch:?}");
        }
        for ch in ['\u{2192}', '\u{276F}', '\u{26A1}'] {
            assert!(!is_icon(ch), "{ch:?}");
        }
    }
}
