//! The app's own colours and fonts: the canvas ground, card borders, labels,
//! the status bar. The tokens of the reference's `app.css`, as a struct
//! (`tokens.test.ts` guarded that every chrome colour was a token; here the
//! type system does), with the terminal theme's colours laid over the ones a
//! theme drives (`theme.svelte.ts`'s TOKENS map).
//!
//! Colours are `Hsla` because that is what gpui paints with; the theme
//! arrives as hex strings from `itermcolors.rs`. Sizes that must hold on
//! screen at every zoom are in SCREEN pixels here and divided by the scale
//! at paint time (`chrome.rs` in core has the maths).
use gpui::{font, rgb, Font, Hsla, Rgba};
use infiniterm_core::itermcolors::Theme;

/// A one-device-pixel line: the thinnest gpui draws crisply. Every card
/// body's hairline dividers and 1px outlines share this so they read as one
/// weight across the app.
pub const HAIRLINE_PX: f64 = 1.;
/// The alpha a gutter or panel hairline is drawn at against its card's
/// background: visible as a seam, not a border.
pub const HAIRLINE_ALPHA: f32 = 0.4;
/// Below this font size a glyph costs more to shape than it is worth
/// looking at (about 1 microsecond each in gpui); card bodies paint texture
/// bars instead. Shared by every body that draws text.
pub const LEGIBLE_FONT_PX: f64 = 3.;
/// Lines scrolled per wheel tick, shared by every card body with a text
/// buffer, so the terminal, editor, diff and transcript all feel the same
/// under the mouse wheel.
pub const WHEEL_LINES_PER_TICK: f64 = 3.;
/// Below `LEGIBLE_FONT_PX` a run of text is drawn as a texture bar this
/// fraction of the line height, shared by the terminal and editor bodies so
/// an unreadable zoom looks the same in both.
pub const TEXTURE_BAR_HEIGHT_RATIO: f32 = 0.55;
/// A texture bar's alpha: a hint of ink, not a glyph.
pub const TEXTURE_BAR_ALPHA: f32 = 0.45;
/// `ui.inactiveDim`'s default, before a settings read overwrites it: how
/// much an unfocused card's scrim dims it. Every body with one starts here.
pub const INACTIVE_DIM_DEFAULT: f64 = 0.45;
/// A list's cursor row (a file tree, or the transcript's turn list) tints
/// this faint when its card isn't focused, so it marks a position without
/// competing with the focused highlight.
pub const TREE_CURSOR_UNFOCUSED_ALPHA: f32 = 0.12;

#[derive(Clone, Debug)]
pub struct Chrome {
    pub canvas_bg: Hsla,
    pub grid_line: Hsla,
    pub card_bg: Hsla,
    pub card_border: Hsla,
    pub card_fg: Hsla,
    pub card_label_fg: Hsla,
    pub agent_working: Hsla,
    pub agent_idle: Hsla,
    pub remote_bg: Hsla,
    pub remote_fg: Hsla,
    pub group_border: Hsla,
    pub group_label_fg: Hsla,
    pub focus_ring: Hsla,
    pub text_bright: Hsla,
    pub text: Hsla,
    pub text_mid: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,
    pub bar_bg: Hsla,
    pub bar_border: Hsla,
    pub control_bg: Hsla,
    pub control_border: Hsla,
    pub sel_bg: Hsla,
    pub sel_fg: Hsla,
    pub row_highlight: Hsla,
    pub row_selected: Hsla,
    pub overlay_backdrop: Hsla,
    pub badge_bg: Hsla,
    pub phantom: Hsla,
    /// The loaded theme, for the label colours chosen by hashing an id.
    pub theme: Option<Theme>,
    pub ui_font: Font,
    pub mono_font: Font,
}

/// `#rrggbb` or `#rgb` to a colour; `None` for anything else.
pub fn hex(s: &str) -> Option<Hsla> {
    let h = s.strip_prefix('#')?;
    let v = match h.len() {
        6 => u32::from_str_radix(h, 16).ok()?,
        3 => {
            let d = |i: usize| u32::from_str_radix(&h[i..i + 1].repeat(2), 16).ok();
            (d(0)? << 16) | (d(1)? << 8) | d(2)?
        }
        _ => return None,
    };
    Some(rgb(v).into())
}

pub fn with_alpha(c: Hsla, a: f32) -> Hsla {
    Hsla { a, ..c }
}

impl Chrome {
    /// The first paint's palette, before any theme is read: the literals the
    /// reference paints before its stylesheet exists.
    pub fn default_chrome() -> Chrome {
        let c = |v: u32| -> Hsla { rgb(v).into() };
        Chrome {
            canvas_bg: c(0x14161a),
            grid_line: c(0x1c212b),
            card_bg: c(0x0e101a),
            card_border: c(0x2a2f38),
            card_fg: c(0xb9c4d2),
            card_label_fg: c(0x7b8794),
            agent_working: c(0xd97757),
            agent_idle: c(0xff9d5c),
            remote_bg: c(0xc0392b),
            remote_fg: c(0xfff2ef),
            group_border: c(0x39414f),
            group_label_fg: c(0xaab4c2),
            focus_ring: c(0xdbe4f0),
            text_bright: c(0xe6ebf2),
            text: c(0xb9c4d2),
            text_mid: c(0x94a3b8),
            text_muted: c(0x7b8794),
            text_faint: c(0x5a6472),
            bar_bg: c(0x0f1115),
            bar_border: c(0x1e2430),
            control_bg: c(0x171b22),
            control_border: c(0x232a35),
            sel_bg: c(0xe39500),
            sel_fg: c(0x0e101a),
            row_highlight: c(0x1e242e),
            row_selected: c(0x2b3442),
            overlay_backdrop: Hsla {
                a: 0.53,
                ..c(0x000000)
            },
            badge_bg: Hsla {
                a: 0.8,
                ..c(0x0e1014)
            },
            phantom: c(0x4a5568),
            theme: None,
            ui_font: font("Menlo"),
            mono_font: font("Menlo"),
        }
    }

    /// A theme drives BOTH the terminal and the chrome: card ground, label
    /// colours, the canvas. They are one surface, and a card whose frame
    /// does not match the terminal inside it looks broken.
    pub fn apply_theme(&mut self, theme: &Theme) {
        if let Some(bg) = theme.get("background").and_then(|s| hex(s)) {
            self.card_bg = bg;
        }
        if let Some(fg) = theme.get("foreground").and_then(|s| hex(s)) {
            self.card_fg = fg;
        }
        self.theme = Some(theme.clone());
    }

    pub fn rgba(c: Hsla) -> Rgba {
        c.into()
    }
}
