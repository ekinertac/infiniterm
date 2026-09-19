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
/// Below this font size a glyph costs more to paint than it is worth
/// looking at (1.7 microseconds each in gpui, measured 2026-09-19: eleven
/// cards at fit-all, about 4 px, cost 85 ms a frame in glyphs alone); card
/// bodies paint texture bars instead. Shared by every body that draws text.
/// Was 3 px, then 5: at 4 px nothing was readable and everything was
/// paid for, and Ekin's fit-all is 6.6 px, where a frame still cost 100
/// ms and the text still did not read.
pub const LEGIBLE_FONT_PX: f64 = 7.;
/// Below this, text is "far": while the viewport MOVES the terminal draws
/// bars so the animation stays smooth (a zoom from fit-all passes through
/// this band with every card still on screen), and at rest, in the band
/// between `LEGIBLE_FONT_PX` and this where a frame is expensive but the
/// text just reads, a frame asked for by output or the cursor blink is
/// painted at most every `FAR_REFRESH_MS`. Zoomed in, nothing changes.
pub const FAR_FONT_PX: f64 = 12.;
pub const FAR_REFRESH_MS: f64 = 250.;
/// Lines scrolled per wheel tick, shared by every card body with a text
/// buffer, so the terminal, editor, diff and transcript all feel the same
/// under the mouse wheel.
pub const WHEEL_LINES_PER_TICK: f64 = 3.;

/// Turns wheel deltas into whole lines without losing the fraction between
/// events.
///
/// A trackpad sends pixels and a mouse wheel sends "lines", which macOS
/// accelerates: a slow, deliberate notch arrives as 0.1 of a line, which is
/// two pixels, which is a third of a text row, which rounded to zero and
/// was dropped. Ten slow notches were ten nothings; only a flick moved the
/// text, so a mouse could not scroll a terminal. The remainder is carried
/// to the next event instead, so three slow notches make one line. A change
/// of direction cancels what was carried, which is what a wheel does.
#[derive(Debug, Default)]
pub struct WheelCarry(f64);

impl WheelCarry {
    /// `dy` in world pixels, `row_px` the height of one text row. Positive
    /// is up, as gpui delivers it.
    pub fn lines(&mut self, dy: f64, row_px: f64) -> i64 {
        let exact = dy / row_px * WHEEL_LINES_PER_TICK + self.0;
        let whole = exact.trunc();
        self.0 = exact - whole;
        whole as i64
    }
}

#[cfg(test)]
mod wheel_tests {
    use super::*;

    // One slow mouse notch at the default font: 0.1 line * 20 px = 2 px
    // against a 16.8 px row. Rounded, that was 0 every time.
    const SLOW_NOTCH_PX: f64 = 2.;
    const ROW_PX: f64 = 16.8;

    #[test]
    fn slow_notches_add_up_instead_of_vanishing() {
        let mut w = WheelCarry::default();
        let mut moved = 0;
        for _ in 0..3 {
            moved += w.lines(SLOW_NOTCH_PX, ROW_PX);
        }
        assert_eq!(
            moved, 1,
            "three slow notches are one line, not three nothings"
        );
        let mut none = WheelCarry::default();
        assert_eq!(
            none.lines(SLOW_NOTCH_PX, ROW_PX),
            0,
            "the first one alone is still under a line"
        );
    }

    #[test]
    fn a_full_notch_and_a_flick_still_move_at_once() {
        let mut w = WheelCarry::default();
        assert_eq!(w.lines(20., ROW_PX), 3, "one full notch is the tick");
        assert!(w.lines(400., ROW_PX) > 50, "a flick is many");
    }

    #[test]
    fn reversing_cancels_what_was_carried() {
        let mut w = WheelCarry::default();
        w.lines(SLOW_NOTCH_PX, ROW_PX);
        w.lines(SLOW_NOTCH_PX, ROW_PX); // 0.71 carried
        assert_eq!(w.lines(-SLOW_NOTCH_PX, ROW_PX), 0);
        assert_eq!(w.lines(-SLOW_NOTCH_PX, ROW_PX), 0);
        assert_eq!(
            w.lines(-SLOW_NOTCH_PX, ROW_PX),
            0,
            "back where it started, nothing moved"
        );
    }
}
/// Below `LEGIBLE_FONT_PX` a run of text is drawn as a texture bar this
/// fraction of the line height, shared by the terminal and editor bodies so
/// an unreadable zoom looks the same in both.
pub const TEXTURE_BAR_HEIGHT_RATIO: f32 = 0.55;
/// A texture bar's alpha: a hint of ink, not a glyph.
pub const TEXTURE_BAR_ALPHA: f32 = 0.45;
/// Greeking (greeking.rs): where the baseline sits in the line, and how
/// far each reach goes from it, as fractions of the line height. A real
/// Menlo line at 1.2 line height has its baseline about three quarters
/// down, x-height about a third of the line, ascenders about half, and
/// descenders a fifth below.
pub const GREEK_BASELINE_RATIO: f32 = 0.78;
pub const GREEK_X_HEIGHT: f32 = 0.34;
pub const GREEK_ASCENDER: f32 = 0.52;
pub const GREEK_DESCENDER: f32 = 0.18;
pub const GREEK_MARK: f32 = 0.12;
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
    /// Mid-turn. The common state, so it stays the quiet branded clay.
    pub agent_working: Hsla,
    /// Blocked on you. The loudest thing on the canvas, and the only one
    /// that earns being loud.
    pub agent_waiting: Hsla,
    /// Finished. Calm, and a different HUE from the other two: at 40% zoom
    /// two oranges differing only in brightness are one colour.
    pub agent_done: Hsla,
    /// Something the app wants to point out that has nothing to do with an
    /// agent: a frame rate that has fallen over, a search with no hits. Its
    /// own colour, because borrowing the agent's is how a signal colour
    /// stops signalling anything.
    pub warn: Hsla,
    pub remote_bg: Hsla,
    pub remote_fg: Hsla,
    pub group_border: Hsla,
    pub group_label_fg: Hsla,
    pub focus_ring: Hsla,
    /// The ring on the OTHER cards of a multiple selection, at the same
    /// strength as the active card's white one. A selection is a set, so a
    /// card is in it or it is not and there is nothing in between to draw;
    /// the active card stays white only to say where you are.
    pub selection_ring: Hsla,
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
    /// An overlay's own ground: LIGHTER than the canvas, because a raised
    /// surface catches more light. The sheets used `bar_bg`, which is
    /// darker than the canvas they float over, so nothing about them said
    /// "on top".
    pub overlay_bg: Hsla,
    /// An overlay's hairline, brighter than a control's: it is the edge of
    /// a surface rather than of a widget.
    pub overlay_border: Hsla,
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
            agent_waiting: c(0xffc400),
            agent_done: c(0x4fb477),
            warn: c(0xe3a008),
            remote_bg: c(0xc0392b),
            remote_fg: c(0xfff2ef),
            group_border: c(0x39414f),
            group_label_fg: c(0xaab4c2),
            focus_ring: c(0xdbe4f0),
            // Blue: the one hue on this canvas that means neither agent
            // state nor a card's identity, so it cannot be misread as either.
            selection_ring: c(0x4a9eff),
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
            overlay_bg: c(0x1b2029),
            overlay_border: c(0x39414f),
            // Deep enough that the canvas recedes. At 0.53 over a near-black
            // canvas there was almost nothing left to darken, so the sheet
            // had to do all the separating by itself.
            overlay_backdrop: Hsla {
                a: 0.72,
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
