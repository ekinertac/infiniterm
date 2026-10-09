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
use crate::fonts::UiTypography;
use gpui::{rgb, Hsla, Rgba};
use infiniterm_core::config::Ui;
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
/// The physical floor under `LEGIBLE_FONT_PX`, in logical pixels. That
/// constant counts DEVICE pixels, measured on Ekin's 1x 4K: a glyph needs
/// about seven to be drawn at all. On a 2x Retina screen seven device
/// pixels are 3.5 logical ones, sharp but physically too small to read, so
/// there the limit is size, not pixels. First measured by eye on the
/// 15-inch MacBook Air, 2026-09-24, where the fixed 7 px line turned a
/// single card to bars at 48% although its 6.7 px text was 13 device
/// pixels and read fine.
pub const LEGIBLE_FLOOR_PX: f32 = 4.5;

/// The font size, in the logical pixels gpui paints in, below which text
/// is drawn as bars: enough device pixels to draw a glyph, and big enough
/// to read. 7 on a 1x screen, as it always was; 4.5 on Retina.
pub fn legible_font_px(scale_factor: f32) -> gpui::Pixels {
    let pixels = LEGIBLE_FONT_PX as f32 / scale_factor.max(1.);
    gpui::px(pixels.max(LEGIBLE_FLOOR_PX))
}
/// While the frame is over its glyph budget (`AppView::crowded`), a frame
/// asked for by output or the cursor blink is painted at most this often:
/// the cards are bars then, and nobody reads a wall of bars frame by frame.
///
/// There used to be a `FAR_FONT_PX` (12 px) under which text drew bars
/// while the viewport moved, counted toward the budget, and repainted at
/// four frames a second, by font size alone. It stood in for the budget
/// before the budget existed, and on 2026-09-24 it turned a single card at
/// 57% (10.8 px, perfectly readable, in a smaller window on Ekin's 4K)
/// into bars at 4 fps. The budget decides now, counting what is on screen.
pub const FAR_REFRESH_MS: f64 = 250.;
/// How many terminal cells may be on screen, across every visible card,
/// before a frame stops painting glyphs at all and every card draws bars
/// (`AppView::crowded`). ON SCREEN: a card half out of the window counts
/// half its cells, because the painter skips the rows outside it.
///
/// The per-card size threshold above cannot say this: the cost is the
/// TOTAL number of glyphs, and a glyph is 1.7 microseconds whatever the
/// zoom, because a card's grid does not shrink when the canvas does. On
/// Ekin's 4K canvas one full card is about 13,000 cells and fit-all with
/// twelve of them is 157,000, which measured 85 to 100 ms a frame at a
/// font size (9.6 px) the old 7 px threshold called legible. This is about
/// two full cards: a pair side by side still reads, a wall of them does
/// not and now costs nothing. Counted from the grids, not from what is in
/// them, so the flag does not flip as output scrolls.
#[cfg(test)]
pub const GLYPH_BUDGET_CELLS: usize = infiniterm_core::config::DEFAULT_GLYPH_BUDGET;

/// The budget also grows with the WINDOW: it is at least this many
/// windowfuls of text at 100%. A fixed 30,000 was less than one windowful
/// on the 4K (three large cards covering the screen at 98% drew bars,
/// 2026-09-24), and no terminal draws bars at its own font size. 1.5 means
/// a window covered edge to edge in cards reads down to about 82% zoom
/// (the square root of 1/1.5), and a half-covered one to about 58%.
pub const GLYPH_BUDGET_WINDOWS: f32 = 1.5;

/// Whether a frame showing `cells` of text across its visible cards must
/// drop to bars, when the window would hold `window_cells` at 100%. The
/// rule, so it can be argued with in one place. `budget` is the floor, the
/// user's `ui.glyphBudget` (default `GLYPH_BUDGET_CELLS`): raising it trades
/// frame time for text at a smaller zoom (Ekin, 2026-10-09: twelve 50 by 80
/// cards at 49% read fine and were bars).
pub fn over_glyph_budget(cells: usize, window_cells: usize, budget: usize) -> bool {
    let relative = (window_cells as f32 * GLYPH_BUDGET_WINDOWS) as usize;
    cells > budget.max(relative)
}
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
    /// Mid-turn, or a long command running. The common state, so a quiet
    /// violet. It was Claude's clay (#d97757) until 2026-09-25, when failed
    /// got red: clay and red are one colour at fit-all zoom, and blue was
    /// taken by the selection ring.
    pub agent_working: Hsla,
    /// Blocked on you. The loudest thing on the canvas, and the only one
    /// that earns being loud.
    pub agent_waiting: Hsla,
    /// Finished. Calm, and a different HUE from the other two: at 40% zoom
    /// two oranges differing only in brightness are one colour.
    pub agent_done: Hsla,
    /// Something failed: a non-zero exit, a turn that ended in error.
    pub agent_failed: Hsla,
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
    pub typography: UiTypography,
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

/// `ui.cardOpacity` for the frame being painted, as a gpui global so every
/// body's `paint` can reach it without a new parameter on `CardBody`
/// (#98). Set once a frame in `paint_world`; absent means opaque.
#[derive(Clone, Copy, PartialEq)]
pub struct CardOpacity(pub f32);
impl gpui::Global for CardOpacity {}

/// The card corner radius in SCREEN pixels this frame (`ui.cardRadius` times
/// the zoom), set once in `paint_world` like `CardOpacity` so a body's
/// `paint` can reach it without a new parameter on `CardBody` (#235).
#[derive(Clone, Copy, PartialEq)]
pub struct CardRadius(pub f32);
impl gpui::Global for CardRadius {}

/// The radius a card body rounds its own quads with; 0 when none is set.
pub fn card_radius(cx: &gpui::App) -> gpui::Pixels {
    gpui::px(cx.try_global::<CardRadius>().map_or(0., |r| r.0))
}

/// The fill under the pointer on a clickable row: the selected row's colour,
/// fainter, so the row you would pick by clicking is visible before you click.
pub fn hover_fill(selected: Hsla) -> Hsla {
    with_alpha(selected, selected.a * 0.6)
}

/// A card body's whole-body fill: the card fill, rounded to the card.
pub fn card_body_quad(
    cx: &gpui::App,
    bounds: gpui::Bounds<gpui::Pixels>,
    c: Hsla,
) -> gpui::PaintQuad {
    gpui::fill(bounds, card_fill(cx, c)).corner_radii(card_radius(cx))
}

/// A wash over a whole body (the inactive dim), rounded like the body so its
/// corners do not poke out past the card's.
pub fn card_wash(cx: &gpui::App, bounds: gpui::Bounds<gpui::Pixels>, c: Hsla) -> gpui::PaintQuad {
    gpui::fill(bounds, c).corner_radii(card_radius(cx))
}

/// A card's background fill at the card opacity. Only the fill across the
/// whole body goes through this: text, selections and cells a program
/// coloured keep their own alpha.
pub fn card_fill(cx: &gpui::App, c: Hsla) -> Hsla {
    scaled_alpha(c, cx.try_global::<CardOpacity>().map_or(1., |o| o.0))
}

/// `c` with its own alpha multiplied by `factor`: a fill that was already
/// translucent stays proportionally so.
pub fn scaled_alpha(c: Hsla, factor: f32) -> Hsla {
    with_alpha(c, c.a * factor)
}

/// `base` pulled `amount` (0 to 1) of the way toward `rgb`, in RGB space, alpha
/// kept: the title bar of a remote instance in its host's colour.
pub fn tint(base: Hsla, rgb: (u8, u8, u8), amount: f32) -> Hsla {
    let b: gpui::Rgba = base.into();
    let mix = |from: f32, to: u8| from + (to as f32 / 255. - from) * amount;
    gpui::Rgba {
        r: mix(b.r, rgb.0),
        g: mix(b.g, rgb.1),
        b: mix(b.b, rgb.2),
        a: b.a,
    }
    .into()
}

/// The title bar's colours when the window wears a colour (`window.color`,
/// #173): the bar leans toward it, the chosen tab's fill leans the same way,
/// and the other tab names are drawn brighter, since the muted grey was made
/// for the plain bar and fades on a tint.
pub struct TabColors {
    pub bar: Hsla,
    pub idle_text: Hsla,
    pub idle_faint: Hsla,
    pub chosen_fill: Hsla,
}

pub fn tab_colors(chrome: &Chrome, wear: Option<(u8, u8, u8)>, amount: f32) -> TabColors {
    match wear {
        Some(c) => TabColors {
            bar: tint(chrome.bar_bg, c, amount),
            idle_text: chrome.text,
            idle_faint: chrome.text_muted,
            chosen_fill: tint(chrome.control_bg, c, amount),
        },
        None => TabColors {
            bar: chrome.bar_bg,
            idle_text: chrome.text_muted,
            idle_faint: chrome.text_faint,
            chosen_fill: chrome.control_bg,
        },
    }
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
            agent_working: c(0x9d7cd8),
            agent_waiting: c(0xffc400),
            agent_done: c(0x4fb477),
            agent_failed: c(0xe5484d),
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
            typography: UiTypography::from_config(&infiniterm_core::config::default_config().ui),
        }
    }

    pub fn apply_ui_typography(&mut self, ui: &Ui) {
        self.typography = UiTypography::from_config(ui);
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

#[cfg(test)]
mod tests {
    use super::*;

    // #98: a card's fill takes the card opacity times its own alpha, and
    // nothing else about the colour changes.
    #[test]
    fn a_fill_scales_its_alpha_and_keeps_its_colour() {
        let c = Hsla {
            h: 0.5,
            s: 0.4,
            l: 0.3,
            a: 1.,
        };
        let half = scaled_alpha(c, 0.5);
        assert_eq!((half.h, half.s, half.l, half.a), (0.5, 0.4, 0.3, 0.5));
        assert_eq!(
            scaled_alpha(half, 0.5).a,
            0.25,
            "already translucent stays so"
        );
        assert_eq!(scaled_alpha(c, 1.), c);
    }

    #[test]
    fn a_tint_moves_toward_the_colour_and_keeps_the_alpha() {
        let base = gpui::rgba(0x101010ff);
        let none: gpui::Rgba = tint(base.into(), (255, 0, 0), 0.).into();
        let full: gpui::Rgba = tint(base.into(), (255, 0, 0), 1.).into();
        let half: gpui::Rgba = tint(gpui::rgba(0x10101080).into(), (255, 0, 0), 0.5).into();
        assert!((none.r - base.r).abs() < 0.01);
        assert!((full.r - 1.).abs() < 0.01 && full.g < 0.01 && full.b < 0.01);
        assert!(half.r > base.r && half.r < 1.);
        assert!((half.a - 128. / 255.).abs() < 0.01, "alpha is kept");
    }

    #[test]
    fn the_legible_line_is_pixels_on_1x_and_size_on_retina() {
        assert_eq!(legible_font_px(1.), gpui::px(7.), "the 4K is unchanged");
        assert_eq!(legible_font_px(2.), gpui::px(LEGIBLE_FLOOR_PX));
        // 6.7 px on the Air (48% of a 14 px font) is text there, bars on 1x.
        assert!(gpui::px(6.7) >= legible_font_px(2.));
        assert!(gpui::px(6.7) < legible_font_px(1.));
    }

    /// One full card on Ekin's canvas is about 151 by 87 cells. A pair
    /// still reads; a wall of eight cost 85 to 100 ms a frame in glyphs.
    #[test]
    fn the_glyph_budget_is_about_two_full_cards() {
        let card = 151 * 87;
        assert!(
            !over_glyph_budget(card, 0, GLYPH_BUDGET_CELLS),
            "one card paints glyphs"
        );
        assert!(
            !over_glyph_budget(card * 2, 0, GLYPH_BUDGET_CELLS),
            "a pair side by side still does"
        );
        assert!(
            over_glyph_budget(card * 3, 0, GLYPH_BUDGET_CELLS),
            "three is past it"
        );
        assert!(
            over_glyph_budget(card * 8, 0, GLYPH_BUDGET_CELLS),
            "a 4x2 fit-all is bars"
        );
    }

    /// A window that holds 60,000 cells at 100% (the 4K) and is covered
    /// edge to edge in cards shows 60,000 / zoom^2 of them. At 98% that is
    /// text; at 70% it is bars.
    #[test]
    fn a_window_full_of_cards_is_text_near_100_percent() {
        let window = 60_000;
        let shown = |zoom: f32| (window as f32 / (zoom * zoom)) as usize;
        assert!(
            !over_glyph_budget(shown(0.98), window, GLYPH_BUDGET_CELLS),
            "98% reads"
        );
        assert!(
            !over_glyph_budget(shown(0.85), window, GLYPH_BUDGET_CELLS),
            "85% reads"
        );
        assert!(
            over_glyph_budget(shown(0.70), window, GLYPH_BUDGET_CELLS),
            "70% is bars"
        );
    }

    /// `ui.glyphBudget` raises the floor: twelve full cards that are bars by
    /// default read once the budget covers them.
    #[test]
    fn a_larger_budget_keeps_text_on_a_wall_of_cards() {
        let twelve = 12 * 50 * 80;
        assert!(over_glyph_budget(twelve, 12_000, GLYPH_BUDGET_CELLS));
        assert!(!over_glyph_budget(twelve, 12_000, 120_000));
        // The window-relative rule still applies when it is larger.
        assert!(!over_glyph_budget(100_000, 100_000, 1_000));
    }
}

#[cfg(test)]
mod tab_color_tests {
    use super::*;

    #[test]
    fn a_window_colour_tints_the_chosen_tab_and_brightens_the_others() {
        let chrome = Chrome::default_chrome();
        let plain = tab_colors(&chrome, None, 0.32);
        assert_eq!(plain.chosen_fill, chrome.control_bg);
        assert_eq!(plain.idle_text, chrome.text_muted);
        let green = tab_colors(&chrome, Some((0x22, 0xaa, 0x44)), 0.32);
        assert_ne!(green.chosen_fill, chrome.control_bg);
        assert_ne!(green.bar, chrome.bar_bg);
        assert_eq!(green.idle_text, chrome.text);
        assert_eq!(green.idle_faint, chrome.text_muted);
    }
}
