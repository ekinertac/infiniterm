//! The tab strip an editor card wears: the band at the top listing its
//! tabs, the `×` on each, the `+` past the last, the card's number at the
//! right, and the hit test that says which of those a click landed on.
//!
//! A second copy of what `browser_body.rs` draws for a browser card, on
//! purpose: that file is the browser session's, and the two strips must
//! look and click the same, so the constants are the same numbers. When a
//! third card kind wants tabs the browser's strip should come here too.
//! Sized in screen pixels, then `ui_scale`, then the zoom, like every
//! other piece of chrome.
//!
//! A tab is drawn like a workspace tab in the title bar (#693): the same
//! label size, padding, height and corner radius, and the `cardTab.*` keys
//! of the ui theme are the `tab.*` keys under another prefix, resolved by
//! the same `Chrome::tab_style`. The same values in both make the two alike.
//!
//! Called by `editor_tabs.rs`. `body::TabClick` is the shared answer.
use crate::body::TabClick;
use crate::chrome::{Chrome, TabStyle};
use gpui::{fill, point, px, quad, size, App, BorderStyle, Bounds, Font, Hsla, Pixels, Window};
use infiniterm_core::grid::Point;
use infiniterm_core::ui_theme::TabTheme;

/// gpui's default line height (`phi`), which a workspace tab's label has:
/// the tab's height is its line plus `TAB_PAD_Y_PX` above and below.
const TAB_LINE_HEIGHT_RATIO: f64 = 1.618_034;
/// A workspace tab's `py_1`, in pixels.
const TAB_PAD_Y_PX: f64 = 4.;
/// A workspace tab's `rounded_sm`, in pixels, before `cardTab.radius`.
const TAB_RADIUS_PX: f64 = 4.;
/// A workspace tab's `gap_1` to its neighbour: half of it on each side of a
/// card tab's slot, so the hit test still counts whole slots.
const TAB_GAP_PX: f64 = 4.;
/// The strip's ground above and below a tab, so a rounded tab sits in the
/// band the way a workspace tab sits in the title bar.
const STRIP_INSET_Y_PX: f64 = 3.;
/// How much of a tab's height a `cardTab.separator` takes, centred: the
/// workspace tabs' 14 px line beside their 12 px label.
const SEPARATOR_HEIGHT_RATIO: f64 = 0.5;
/// A tab's width in font sizes. Fixed rather than proportional to the
/// card: a card with many tabs would otherwise shrink every tab to a
/// sliver.
pub const TAB_STRIP_TAB_WIDTH_RATIO: f64 = 9.;
/// The folder header's and the card number's inset, in font sizes. A tab's
/// own label inset is `cardTab.paddingX`.
pub const TAB_STRIP_LABEL_PAD_RATIO: f64 = 0.6;
/// The close button's band at a tab's right edge.
pub const TAB_STRIP_CLOSE_WIDTH_RATIO: f64 = 1.5;
pub const TAB_STRIP_BORDER_PX: f64 = 1.;
/// The "+" button's slot after the last tab, in font sizes: wide enough to
/// click, narrow enough that the glyph sits beside the tabs. It took a whole
/// tab's width (9) and floated about four sizes away from them (#375).
pub const TAB_STRIP_PLUS_WIDTH_RATIO: f64 = 2.5;

/// The strip's colours, mirrored from the chrome each frame.
#[derive(Clone, Debug, PartialEq)]
pub struct StripStyle {
    pub bg: Hsla,
    pub border: Hsla,
    /// The tabs once the theme's `cardTab.*` keys have had their say.
    pub tabs: TabStyle,
    pub text_bright: Hsla,
    pub text_muted: Hsla,
    pub regular: Font,
    pub bold: Font,
    /// The workspace tabs' label size: the strip's font, in screen pixels.
    pub font_px: f64,
}

/// The strip's look for this frame: the chrome's colours and the ui
/// theme's card tab keys. Unset keys are what a workspace tab wears unset:
/// no fill on an idle tab, the control colour on the chosen one.
pub fn strip_style(chrome: &Chrome, theme: &TabTheme) -> StripStyle {
    StripStyle {
        bg: chrome.bar_bg,
        border: chrome.bar_border,
        tabs: chrome.tab_style(theme, chrome.control_bg),
        text_bright: chrome.text_bright,
        text_muted: chrome.text_muted,
        regular: chrome.typography.regular.clone(),
        bold: chrome.typography.bold.clone(),
        font_px: chrome
            .typography
            .size(crate::overlays::TAB_LABEL_FONT_PX)
            .into(),
    }
}

/// A tab's height in screen pixels at ui_scale 1, as a workspace tab's.
fn tab_h(font_px: f64) -> f64 {
    font_px * TAB_LINE_HEIGHT_RATIO + 2. * TAB_PAD_Y_PX
}

/// The strip's height in WORLD units: `local`'s space in every mouse
/// handler, the zoom not yet applied.
pub fn strip_world_h(font_px: f64, ui_scale: f32) -> f64 {
    (tab_h(font_px) + 2. * STRIP_INSET_Y_PX) * ui_scale as f64
}

/// Where a tab's `×` ends, from the tab's left edge, in WORLD units: the
/// tab's padding (`pad_x`, pixels) in from the tab's right edge, the way
/// the label sits that far in from its left edge.
fn close_right(font_px: f64, ui_scale: f32, pad_x: f64) -> f64 {
    let ui = ui_scale as f64;
    TAB_STRIP_TAB_WIDTH_RATIO * font_px * ui - (TAB_GAP_PX / 2. + pad_x) * ui
}

/// Where a tab's close band starts, from the tab's left edge, in WORLD
/// units: one close width ahead of the `×`'s right edge. The band runs on
/// to the tab's edge, so the padding past the `×` closes too.
fn close_band_left(font_px: f64, ui_scale: f32, pad_x: f64) -> f64 {
    close_right(font_px, ui_scale, pad_x) - TAB_STRIP_CLOSE_WIDTH_RATIO * font_px * ui_scale as f64
}

impl StripStyle {
    /// A tab's padding in pixels: `cardTab.paddingX`, else a workspace tab's.
    pub fn pad_x(&self) -> f64 {
        f64::from(self.tabs.padding_x.unwrap_or(crate::overlays::TAB_PAD_X_PX))
    }
}

/// The strip's own "#N" badge, with `card.protect`'s own lock ahead of it
/// when set — the same glyph and wording as the corner label's
/// (`Model::numbered_label`). A different lock from this strip's `locked`
/// (the keyboard's), so it is spelled out rather than reusing that word.
fn number_label(card_number: u32, protected: bool) -> String {
    if protected {
        format!("\u{1f512} #{card_number}")
    } else {
        format!("#{card_number}")
    }
}

/// Which of the strip's affordances a click at `local` (world units) lands
/// on, or `None` below the strip.
///
/// `left` is the width of the folder header ahead of the tabs (world units,
/// 0 without one): a click on the header is no tab's. `pad_x` is the tab's
/// padding in pixels (`StripStyle::pad_x`), which places the `×`.
pub fn strip_hit(
    local: Point,
    font_px: f64,
    ui_scale: f32,
    tab_count: usize,
    left: f64,
    pad_x: f64,
) -> Option<TabClick> {
    let strip_h = strip_world_h(font_px, ui_scale);
    if local.y < 0. || local.y >= strip_h || local.x < left {
        return None;
    }
    let local = Point {
        x: local.x - left,
        y: local.y,
    };
    let unit = font_px * ui_scale as f64;
    let tab_w = TAB_STRIP_TAB_WIDTH_RATIO * unit;
    let index = (local.x / tab_w).floor().max(0.) as usize;
    if index >= tab_count {
        let from_plus = local.x - tab_count as f64 * tab_w;
        return (from_plus >= 0. && from_plus < TAB_STRIP_PLUS_WIDTH_RATIO * unit)
            .then_some(TabClick::New);
    }
    let x_in_tab = local.x - (index as f64) * tab_w;
    if x_in_tab >= close_band_left(font_px, ui_scale, pad_x) {
        Some(TabClick::Close(index))
    } else {
        Some(TabClick::Switch(index))
    }
}

/// One tab's fill and outline in `pill`, the chosen tab's or an idle one's.
/// The folder header wears an idle tab's.
fn paint_tab_box(
    window: &mut Window,
    pill: Bounds<Pixels>,
    radius: Pixels,
    border: Pixels,
    tabs: &TabStyle,
    active: bool,
) {
    let (tab_fill, outline) = if active {
        (Some(tabs.active_fill), tabs.active_border)
    } else {
        (tabs.idle_fill, tabs.idle_border)
    };
    if let Some(f) = tab_fill {
        window.paint_quad(fill(pill, f.background()).corner_radii(radius));
    }
    if let Some(c) = outline {
        window.paint_quad(quad(
            pill,
            radius,
            gpui::transparent_black(),
            border,
            c,
            BorderStyle::Solid,
        ));
    }
}

/// Draws the strip across the top of `bounds` (screen pixels, at `scale`)
/// and returns its height, so the caller paints its content below.
/// `header` is a folder editor's name and the width of its tree (world
/// units): a box over the tree's column, with the tabs starting after it.
#[allow(clippy::too_many_arguments)]
pub fn paint_strip(
    bounds: Bounds<Pixels>,
    scale: f64,
    ui_scale: f32,
    labels: &[String],
    active: usize,
    card_number: u32,
    protected: bool,
    header: Option<(&str, f64)>,
    style: &StripStyle,
    window: &mut Window,
    cx: &mut App,
) -> Pixels {
    // One unit is the font size on screen; widths are a ratio of it. The
    // workspace tab's own measures are pixels, so they scale by `px_scale`.
    let unit = style.font_px * ui_scale as f64 * scale;
    let px_scale = ui_scale as f64 * scale;
    let strip_h = px((strip_world_h(style.font_px, ui_scale) * scale) as f32);
    let inset_y = px((STRIP_INSET_Y_PX * px_scale) as f32);
    let tab_h = px((tab_h(style.font_px) * px_scale) as f32);
    let half_gap = px((TAB_GAP_PX / 2. * px_scale) as f32);
    let radius = px((style.tabs.radius.map_or(TAB_RADIUS_PX, f64::from) * px_scale) as f32);
    let strip = Bounds::new(bounds.origin, size(bounds.size.width, strip_h));
    // The strip is the card's top edge: with rounded cards its top corners follow.
    let r = crate::chrome::card_radius(cx);
    window.paint_quad(fill(strip, style.bg).corner_radii(gpui::Corners {
        top_left: r,
        top_right: r,
        bottom_left: px(0.),
        bottom_right: px(0.),
    }));
    let border = px((TAB_STRIP_BORDER_PX * ui_scale as f64 * scale) as f32);
    let tab_w = px((TAB_STRIP_TAB_WIDTH_RATIO * unit) as f32);
    let strip_font = px(unit as f32);
    // The `×` ends `cardTab.paddingX` in from the tab's right edge.
    let close_right = px((close_right(style.font_px, ui_scale, style.pad_x()) * scale) as f32);
    let close_left = px((close_band_left(style.font_px, ui_scale, style.pad_x()) * scale) as f32);
    if strip_font >= crate::chrome::legible_font_px(window.scale_factor()) {
        let pad = px((TAB_STRIP_LABEL_PAD_RATIO * unit) as f32);
        let left = px(header.map_or(0., |(_, w)| w * scale) as f32);
        // A tab's label inset from its own edge (`cardTab.paddingX`).
        let tab_pad = px((style.pad_x() * px_scale) as f32) + half_gap;
        // A slot less half the gap on each side and the strip's inset above
        // and below: the box a tab is drawn in, like a workspace tab.
        let pill_in = |slot: Bounds<Pixels>| {
            Bounds::new(
                point(slot.origin.x + half_gap, slot.origin.y + inset_y),
                size(slot.size.width - half_gap * 2., tab_h),
            )
        };
        if let Some((name, _)) = header {
            let head = Bounds::new(strip.origin, size(left, strip_h));
            // The folder header is a box of the tabs' height, filled and
            // outlined like an idle tab, so the row reads as one (#693).
            paint_tab_box(window, pill_in(head), radius, border, &style.tabs, false);
            let room = f32::from(left) - f32::from(tab_pad) * 2.;
            let shown = crate::text::elide(name, room, |t| {
                f32::from(
                    crate::text::shape(window, t, strip_font, &style.bold, style.text_bright).width,
                )
            });
            let line =
                crate::text::shape(window, &shown, strip_font, &style.bold, style.text_bright);
            crate::text::paint_in(window, cx, &line, head, tab_pad);
        }
        for (i, label) in labels.iter().enumerate() {
            let tab_bounds = Bounds::new(
                point(strip.origin.x + left + tab_w * (i as f32), strip.origin.y),
                size(tab_w, strip_h),
            );
            let pill = pill_in(tab_bounds);
            let is_active = i == active;
            paint_tab_box(window, pill, radius, border, &style.tabs, is_active);
            // Between two idle neighbours only: no line touches the chosen tab.
            if let Some(c) = style.tabs.separator {
                if i > 0 && !is_active && i - 1 != active {
                    let line_h = tab_h * SEPARATOR_HEIGHT_RATIO as f32;
                    window.paint_quad(fill(
                        Bounds::new(
                            point(
                                tab_bounds.origin.x - border / 2.,
                                pill.origin.y + (tab_h - line_h) / 2.,
                            ),
                            size(border, line_h),
                        ),
                        c,
                    ));
                }
            }
            let (color, font) = if is_active {
                (style.text_bright, &style.bold)
            } else {
                (style.text_muted, &style.regular)
            };
            // The label runs from its padding to where the close band starts.
            let room = f32::from(close_left) - f32::from(tab_pad);
            let shown = crate::text::elide(label, room, |t| {
                f32::from(crate::text::shape(window, t, strip_font, font, color).width)
            });
            let line = crate::text::shape(window, &shown, strip_font, font, color);
            crate::text::paint_in(window, cx, &line, tab_bounds, tab_pad);
            let close_line = crate::text::shape(window, "×", strip_font, font, color);
            let close_bounds = Bounds::new(
                point(
                    tab_bounds.origin.x + close_right - close_line.width,
                    tab_bounds.origin.y,
                ),
                size(close_line.width, strip_h),
            );
            crate::text::paint_in(window, cx, &close_line, close_bounds, px(0.));
        }
        let plus = crate::text::shape(window, "+", strip_font, &style.regular, style.text_muted);
        let plus_w = px((TAB_STRIP_PLUS_WIDTH_RATIO * unit) as f32);
        let plus_bounds = Bounds::new(
            point(
                strip.origin.x + left + tab_w * (labels.len() as f32) + plus_w / 2.
                    - plus.width / 2.,
                strip.origin.y,
            ),
            size(plus.width, strip_h),
        );
        crate::text::paint_in(window, cx, &plus, plus_bounds, px(0.));
        if card_number > 0 {
            let number = number_label(card_number, protected);
            let line = crate::text::shape(
                window,
                &number,
                strip_font,
                &style.regular,
                style.text_muted,
            );
            let number_bounds = Bounds::new(
                point(
                    strip.origin.x + strip.size.width - line.width - pad,
                    strip.origin.y,
                ),
                size(line.width + pad, strip_h),
            );
            crate::text::paint_in(window, cx, &line, number_bounds, px(0.));
        }
    }
    // The hairline under the strip.
    window.paint_quad(fill(
        Bounds::new(
            point(strip.origin.x, strip.origin.y + strip_h - border),
            size(strip.size.width, border),
        ),
        style.border,
    ));
    strip_h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A click at (x, y) on a strip at a 10 px font with the default 6 px
    /// tab padding: `scale` is ui_scale, `n` the tab count, `left` the
    /// folder header's width.
    fn hit(x: f64, y: f64, scale: f32, n: usize, left: f64) -> Option<TabClick> {
        strip_hit(Point { x, y }, 10., scale, n, left, 6.)
    }

    // Hit-test geometry in world units at a 10 px font and ui_scale 1: a
    // tab is 90 wide, the `×` ends 8 in from its edge (half the 4 gap plus
    // the 6 padding) and its band starts 15 before that, at 67; the strip
    // is about 30 tall (`strip_world_h`), the 25 after the last tab is "+".
    #[test]
    fn a_click_lands_on_a_tab_its_close_or_the_plus() {
        assert_eq!(hit(10., 10., 1., 2, 0.), Some(TabClick::Switch(0)));
        assert_eq!(hit(66., 10., 1., 2, 0.), Some(TabClick::Switch(0)));
        assert_eq!(hit(68., 10., 1., 2, 0.), Some(TabClick::Close(0)));
        assert_eq!(
            hit(88., 10., 1., 2, 0.),
            Some(TabClick::Close(0)),
            "the padding past the × closes too"
        );
        assert_eq!(hit(100., 10., 1., 2, 0.), Some(TabClick::Switch(1)));
        assert_eq!(hit(190., 10., 1., 2, 0.), Some(TabClick::New));
        assert_eq!(
            hit(210., 10., 1., 2, 0.),
            None,
            "past the plus's own slot, 25 wide after the last tab (#375)"
        );
        assert_eq!(hit(400., 10., 1., 2, 0.), None, "past the plus");
        assert_eq!(hit(10., 35., 1., 2, 0.), None, "below the strip");
        // Twice the interface scale: everything twice as big.
        assert_eq!(hit(100., 25., 2., 2, 0.), Some(TabClick::Switch(0)));
        assert_eq!(hit(140., 25., 2., 2, 0.), Some(TabClick::Close(0)));
    }

    // `cardTab.paddingX` moves the `×` in from the tab's edge, and its
    // click band with it (#693).
    #[test]
    fn the_close_band_follows_the_tab_padding() {
        let wide = |x: f64| strip_hit(Point { x, y: 10. }, 10., 1., 1, 0., 20.);
        // 90 - (2 + 20) = 68 is the ×'s right edge, 53 its band's left.
        assert_eq!(wide(52.), Some(TabClick::Switch(0)));
        assert_eq!(wide(54.), Some(TabClick::Close(0)));
    }

    // A folder editor's header takes the tree's width ahead of the tabs:
    // a click on it is nobody's, and the tabs count from its right edge.
    #[test]
    fn the_folder_header_shifts_the_tabs_and_is_no_tab() {
        assert_eq!(hit(100., 10., 1., 2, 200.), None);
        assert_eq!(hit(210., 10., 1., 2, 200.), Some(TabClick::Switch(0)));
        assert_eq!(hit(280., 10., 1., 2, 200.), Some(TabClick::Close(0)));
        assert_eq!(
            hit(390., 10., 1., 0, 200.),
            None,
            "no tab and past the plus"
        );
        assert_eq!(hit(205., 10., 1., 0, 200.), Some(TabClick::New));
    }

    // A card tab is as tall as a workspace tab at the same font (#693): its
    // line at gpui's default height plus `py_1` above and below, and the
    // strip adds its inset around it. Both grow with the interface size.
    #[test]
    fn a_card_tab_is_as_tall_as_a_workspace_tab() {
        let workspace_tab = 12. * 1.618_034 + 2. * 4.;
        assert!((tab_h(12.) - workspace_tab).abs() < 1e-6);
        assert!((strip_world_h(12., 1.) - (workspace_tab + 6.)).abs() < 1e-6);
        assert!((strip_world_h(12., 2.) - 2. * strip_world_h(12., 1.)).abs() < 1e-6);
    }

    // Unset `cardTab.*` keys draw a card tab the way unset `tab.*` keys
    // draw a workspace tab: no fill on an idle tab, the control colour on
    // the chosen one, no outline or separator, the workspace tab's padding.
    #[test]
    fn no_card_tab_keys_draw_a_workspace_tab() {
        let chrome = Chrome::default_chrome();
        let s = strip_style(&chrome, &TabTheme::default());
        assert_eq!(
            s.tabs,
            chrome.tab_style(&TabTheme::default(), chrome.control_bg)
        );
        assert_eq!(s.tabs.idle_fill, None);
        assert_eq!(
            s.tabs.active_fill,
            crate::chrome::TabFill::Flat(chrome.control_bg)
        );
        assert_eq!((s.tabs.radius, s.tabs.padding_x), (None, None));
        let themed = strip_style(
            &chrome,
            &TabTheme {
                padding_x: Some(14.),
                ..TabTheme::default()
            },
        );
        assert_eq!(themed.tabs.padding_x, Some(14.));
    }

    // The lock is `card.protect`'s, not this strip's own `locked`: it
    // shows or not by the card's saved flag alone.
    #[test]
    fn a_protected_card_wears_the_lock_glyph_beside_its_number() {
        assert_eq!(number_label(7, false), "#7");
        assert_eq!(number_label(7, true), "\u{1f512} #7");
    }
}
