//! Which terminal characters are Nerd Font icons, and the cutter that splits a
//! row's text where an icon starts or ends.
//!
//! Responsibility: the pure half of drawing prompt icons on a platform whose
//! text system cannot fall back to a font the app bundled. Nerd Font glyphs
//! (powerline arrows, git and folder symbols, the Material icon set) live in
//! Unicode's private use areas, which no ordinary font fills, so a terminal
//! row has to get them from the icon font.
//!
//! Where it fits: macOS registers the bundled font with CoreText and names it
//! as a fallback, which works there. gpui on Windows resolves a fallback by
//! NAME in the SYSTEM font collection only, so a font added with `add_fonts`
//! is invisible to the fallback list; it IS found when a text run names its
//! family directly. So on Windows `terminal_body::shape_row` puts each icon in
//! a run of its own that names the bundled family, and this file decides which
//! characters those are and where one run ends and the next begins.
//!
//! Related: infiniterm-ui/src/terminal_body.rs (`shape_row`, the caller),
//! infiniterm-ui/src/not_macos.rs (`icon_font`, the family name and the font
//! bytes), infiniterm-ui/src/icon_font.rs (the macOS way).
//!
//! Constraint: this must agree with what the bundled font actually covers; a
//! character it claims and the font lacks draws as the font's missing glyph,
//! which is the box the terminal showed before.

/// Whether `ch` is in a private use area a Nerd Font fills: the BMP one
/// (U+E000 to U+F8FF: powerline, Font Awesome, Devicons, Octicons and the
/// rest) and the supplementary plane A one (U+F0000 to U+FFFFD, where Nerd
/// Fonts 3 keeps the Material Design icons).
pub fn is_icon(ch: char) -> bool {
    matches!(ch as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD)
}

/// Groups the characters of a row's run into stretches of one kind, in bytes,
/// as the shaper wants text runs: a length and a font.
///
/// Fed one cell's bytes at a time with `push`, which hands back the stretch
/// that just ended when the kind changes; `finish` hands back the last one.
/// It counts bytes, not characters, because a cell's zero-width characters
/// (U+FE0F and the like) travel with their cell's character in the shaped
/// text and belong to its stretch.
#[derive(Debug, Default)]
pub struct Cutter {
    kind: Option<bool>,
    bytes: usize,
}

impl Cutter {
    pub fn new() -> Cutter {
        Cutter::default()
    }

    /// Adds a cell of `bytes` bytes whose character is an icon or not.
    /// Returns the finished stretch `(bytes, is_icon)` when this cell starts
    /// a new one.
    pub fn push(&mut self, icon: bool, bytes: usize) -> Option<(usize, bool)> {
        let done = match self.kind {
            Some(kind) if kind != icon && self.bytes > 0 => Some((self.bytes, kind)),
            _ => None,
        };
        if done.is_some() {
            self.bytes = 0;
        }
        self.kind = Some(icon);
        self.bytes += bytes;
        done
    }

    /// The stretch still open, if any, and resets the cutter for the next run.
    pub fn finish(&mut self) -> Option<(usize, bool)> {
        let done = self
            .kind
            .filter(|_| self.bytes > 0)
            .map(|k| (self.bytes, k));
        *self = Cutter::default();
        done
    }
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

    fn cut(cells: &[(bool, usize)]) -> Vec<(usize, bool)> {
        let mut c = Cutter::new();
        let mut out: Vec<_> = cells.iter().filter_map(|&(i, n)| c.push(i, n)).collect();
        out.extend(c.finish());
        out
    }

    #[test]
    fn a_row_is_cut_where_the_kind_changes_and_the_bytes_add_up() {
        // "ab" + icon + "cd" + icon + icon: the prompt's shape.
        let cells = [
            (false, 1),
            (false, 1),
            (true, 3),
            (false, 1),
            (false, 1),
            (true, 3),
            (true, 3),
        ];
        let got = cut(&cells);
        assert_eq!(got, vec![(2, false), (3, true), (2, false), (6, true)]);
        let total: usize = cells.iter().map(|c| c.1).sum();
        assert_eq!(got.iter().map(|s| s.0).sum::<usize>(), total);
    }

    #[test]
    fn a_run_of_one_kind_is_one_stretch_and_an_empty_run_is_none() {
        assert_eq!(cut(&[(false, 1), (false, 2)]), vec![(3, false)]);
        assert_eq!(cut(&[(true, 3)]), vec![(3, true)]);
        assert_eq!(cut(&[]), vec![]);
    }

    // A cell's zero-width characters ride with it: the cell is bigger than
    // its character, and the stretch counts all of it.
    #[test]
    fn a_cells_zero_width_bytes_stay_in_its_stretch() {
        assert_eq!(
            cut(&[(true, 3 + 3), (false, 1)]),
            vec![(6, true), (1, false)]
        );
    }

    // `finish` leaves the cutter ready for the next style run of the row.
    #[test]
    fn finishing_resets_the_cutter() {
        let mut c = Cutter::new();
        assert_eq!(c.push(true, 3), None);
        assert_eq!(c.finish(), Some((3, true)));
        assert_eq!(c.finish(), None);
        assert_eq!(c.push(false, 1), None, "nothing open, so nothing ends here");
        assert_eq!(c.finish(), Some((1, false)));
    }
}
