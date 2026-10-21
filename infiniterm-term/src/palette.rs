//! The sixteen ANSI colours plus foreground, background and cursor, as the
//! terminal paints them: the theme's, else the app's fallback palette.
//! From the reference's `themeFromCss` in TerminalCard.svelte: all sixteen
//! matter, because bold text draws in the BRIGHT variant, and leaving the
//! brights undefined made bold output fall back to saturated defaults that
//! read as "the font looks wrong".
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use infiniterm_core::itermcolors::{Theme, ANSI};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub ansi: [[u8; 3]; 16],
    pub foreground: [u8; 3],
    pub background: [u8; 3],
    pub cursor: [u8; 3],
    pub cursor_text: [u8; 3],
    pub selection: [u8; 3],
    pub selection_text: [u8; 3],
}

fn to_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn hex(s: &str) -> Option<[u8; 3]> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// The app's first-paint palette (app.css), the last resort if a token is
/// missing; the defaults match the reference's literals.
const FALLBACK_ANSI: [u32; 16] = [
    0x0e101a, 0xe03600, 0x5dcd97, 0xe39500, 0x00a3cb, 0x795ccc, 0x00c0ff, 0xebefc0, 0x2b2f46,
    0xff4821, 0x58db9e, 0xf6a100, 0x00ffff, 0xf17eba, 0x00b4e0, 0xb3b692,
];

fn u(v: u32) -> [u8; 3] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

impl Palette {
    pub fn default_palette() -> Palette {
        Palette {
            ansi: FALLBACK_ANSI.map(u),
            foreground: u(0x509e31),
            background: u(0x0e101a),
            cursor: u(0xedf2c2),
            cursor_text: u(0x0e101a),
            selection: u(0xe39500),
            selection_text: u(0x0e101a),
        }
    }

    /// The theme laid over the fallback; `selection` and `selection_text`
    /// are the editor colours (the theme's yellow on its background, or the
    /// settings' overrides), so terminals and editors select alike.
    pub fn from_theme(
        theme: &Theme,
        selection: Option<[u8; 3]>,
        selection_text: Option<[u8; 3]>,
        cursor: Option<[u8; 3]>,
    ) -> Palette {
        let mut p = Palette::default_palette();
        let get = |k: &str| theme.get(k).and_then(|s| hex(s));
        for (i, name) in ANSI.iter().enumerate() {
            if let Some(c) = get(name) {
                p.ansi[i] = c;
            }
        }
        p.foreground = get("foreground").unwrap_or(p.foreground);
        p.background = get("background").unwrap_or(p.background);
        // The letter under a block cursor: the theme's cursor text when it
        // reads against the cursor, else black or white (#343). A cursor
        // colour from the settings brings no text colour of its own.
        p.cursor = cursor.or_else(|| get("cursor")).unwrap_or(p.foreground);
        let accent = if cursor.is_some() {
            None
        } else {
            theme.get("cursorAccent").map(String::as_str)
        };
        let text = infiniterm_core::label_colors::cursor_text_on(&to_hex(p.cursor), accent);
        p.cursor_text = hex(&text).unwrap_or(p.background);
        p.selection = selection.or_else(|| get("yellow")).unwrap_or(p.selection);
        p.selection_text = selection_text.unwrap_or(p.background);
        p
    }

    /// A cell's colour as RGB. Bold with a normal ANSI colour draws the bright
    /// variant, as xterm and iTerm2 do.
    pub fn resolve(&self, color: Color, bold: bool) -> [u8; 3] {
        match color {
            Color::Spec(c) => [c.r, c.g, c.b],
            Color::Indexed(i) => self.indexed(i as usize, bold),
            Color::Named(n) => match n {
                NamedColor::Foreground | NamedColor::BrightForeground => self.foreground,
                NamedColor::DimForeground => dim(self.foreground),
                NamedColor::Background => self.background,
                NamedColor::Cursor => self.cursor,
                n => {
                    let i = n as usize;
                    if i < 16 {
                        self.indexed(i, bold)
                    } else if i >= NamedColor::DimBlack as usize
                        && i <= NamedColor::DimWhite as usize
                    {
                        dim(self.ansi[i - NamedColor::DimBlack as usize])
                    } else {
                        self.foreground
                    }
                }
            },
        }
    }

    fn indexed(&self, i: usize, bold: bool) -> [u8; 3] {
        if i < 8 && bold {
            self.ansi[i + 8]
        } else if i < 16 {
            self.ansi[i]
        } else if i < 232 {
            // xterm's 6x6x6 colour cube: each axis is one of 6 levels, 0 or
            // BASE + level*STEP, so level 0 is pure black on every axis.
            const CUBE_SIDE: usize = 6;
            const CUBE_BASE: usize = 55;
            const CUBE_STEP: usize = 40;
            let i = i - 16;
            let (r, g, b) = (
                i / (CUBE_SIDE * CUBE_SIDE),
                (i / CUBE_SIDE) % CUBE_SIDE,
                i % CUBE_SIDE,
            );
            let c = |v: usize| {
                if v == 0 {
                    0
                } else {
                    (CUBE_BASE + v * CUBE_STEP) as u8
                }
            };
            [c(r), c(g), c(b)]
        } else {
            // xterm's 24-step greyscale ramp, indices 232-255.
            const GREY_BASE: usize = 8;
            const GREY_STEP: usize = 10;
            let v = (GREY_BASE + (i.min(255) - 232) * GREY_STEP) as u8;
            [v, v, v]
        }
    }

    /// The default answer to an OSC colour query, by ANSI index.
    pub fn default_rgb(index: usize) -> Option<Rgb> {
        let p = Palette::default_palette();
        let c = match index {
            0..=15 => p.ansi[index],
            256 => p.foreground,
            257 => p.background,
            258 => p.cursor,
            _ => return None,
        };
        Some(Rgb {
            r: c[0],
            g: c[1],
            b: c[2],
        })
    }
}

/// SGR faint: two-thirds brightness, xterm's own dim ratio.
const DIM_NUMERATOR: u16 = 2;
const DIM_DENOMINATOR: u16 = 3;

fn dim(c: [u8; 3]) -> [u8; 3] {
    [
        (c[0] as u16 * DIM_NUMERATOR / DIM_DENOMINATOR) as u8,
        (c[1] as u16 * DIM_NUMERATOR / DIM_DENOMINATOR) as u8,
        (c[2] as u16 * DIM_NUMERATOR / DIM_DENOMINATOR) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_overrides_the_fallback_and_bold_uses_the_bright_slot() {
        let theme: Theme = [
            ("red", "#ff0000"),
            ("brightRed", "#ff8080"),
            ("background", "#101010"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let p = Palette::from_theme(&theme, None, None, None);
        assert_eq!(p.resolve(Color::Named(NamedColor::Red), false), [255, 0, 0]);
        assert_eq!(
            p.resolve(Color::Named(NamedColor::Red), true),
            [255, 128, 128]
        );
        assert_eq!(p.resolve(Color::Indexed(1), true), [255, 128, 128]);
        assert_eq!(p.background, [16, 16, 16]);
        assert_eq!(
            p.resolve(Color::Named(NamedColor::Green), false),
            u(0x5dcd97),
            "fallback where the theme is silent"
        );
    }

    #[test]
    fn the_cube_and_the_greys_are_xterms() {
        let p = Palette::default_palette();
        assert_eq!(p.resolve(Color::Indexed(16), false), [0, 0, 0]);
        assert_eq!(p.resolve(Color::Indexed(231), false), [255, 255, 255]);
        assert_eq!(p.resolve(Color::Indexed(232), false), [8, 8, 8]);
        assert_eq!(p.resolve(Color::Indexed(255), false), [238, 238, 238]);
    }

    #[test]
    fn the_letter_under_the_cursor_reads_whatever_the_theme_says() {
        let mk = |pairs: &[(&str, &str)]| -> Theme {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        // Cursor text equal to the cursor: black or white instead.
        let same = mk(&[("cursor", "#e0e0e0"), ("cursorAccent", "#e0e0e0")]);
        let p = Palette::from_theme(&same, None, None, None);
        assert_eq!(p.cursor_text, [0x12, 0x15, 0x1a]);
        // A good theme choice stays.
        let good = mk(&[("cursor", "#e0e0e0"), ("cursorAccent", "#101010")]);
        assert_eq!(
            Palette::from_theme(&good, None, None, None).cursor_text,
            [16, 16, 16]
        );
        // The setting wins over the theme's cursor and brings its own text.
        let p = Palette::from_theme(&good, None, None, Some([0x20, 0x30, 0x80]));
        assert_eq!(p.cursor, [0x20, 0x30, 0x80]);
        assert_eq!(p.cursor_text, [0xf2, 0xf6, 0xfb]);
    }
}
