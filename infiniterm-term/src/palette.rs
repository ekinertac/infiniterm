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
        p.cursor = get("cursor").unwrap_or(p.foreground);
        p.cursor_text = get("cursorAccent").unwrap_or(p.background);
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
            let i = i - 16;
            let (r, g, b) = (i / 36, (i / 6) % 6, i % 6);
            let c = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
            [c(r), c(g), c(b)]
        } else {
            let v = (8 + (i.min(255) - 232) * 10) as u8;
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

fn dim(c: [u8; 3]) -> [u8; 3] {
    [
        (c[0] as u16 * 2 / 3) as u8,
        (c[1] as u16 * 2 / 3) as u8,
        (c[2] as u16 * 2 / 3) as u8,
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
        let p = Palette::from_theme(&theme, None, None);
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
}
