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
//! A tab dragged along the strip (#682) is `TabDrag`: the body keeps one
//! from the press to the release, `paint_strip` draws the others making
//! room, and the drop is handed to the model as `Model::move_tab`.
//!
//! A tab cuts a long name with an ellipsis, so the pointer over a tab
//! (#742) shows `paint_hover_label`: the full name, and under it the file's
//! path or the page's address when `cardTab.hoverDetail` is true. The
//! body keeps which tab is hovered from `strip_hit` in its `mouse_move`.
//! Hand painted like the rest of the strip, so not gpui's element tooltip.
//!
//! Called by `editor_tabs.rs`. `body::TabClick` is the shared answer.
use crate::body::TabClick;
use crate::chrome::{Chrome, TabStyle};
use gpui::{fill, point, px, quad, size, App, BorderStyle, Bounds, Font, Hsla, Pixels, Window};
use infiniterm_core::grid::Point;
use infiniterm_core::model::tabs_cmd::index_after_move;
use infiniterm_core::ui_theme::TabTheme;

/// gpui's default line height (`phi`), which a workspace tab's label has:
/// the tab's height is its line plus `TAB_PAD_Y_PX` above and below. Spelled
/// as gpui spells it, not as `std::f64::consts::GOLDEN_RATIO`, which needs a
/// newer Rust than some machines building this have.
#[allow(clippy::approx_constant)]
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

/// How far a press on a tab travels, in font sizes, before it is a drag:
/// under that it is the click that switched to the tab, and a hand that
/// shakes a little does not reorder anything.
pub const TAB_DRAG_SLOP_RATIO: f64 = 0.5;

/// A tab held by the pointer (#682), in world units like `strip_hit`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabDrag {
    pub from: usize,
    start_x: f64,
    /// How far the pointer has gone since the press.
    pub dx: f64,
    /// Past the slop: the strip draws the drag and the release drops it.
    pub moving: bool,
}

impl TabDrag {
    pub fn new(from: usize, x: f64) -> TabDrag {
        TabDrag {
            from,
            start_x: x,
            dx: 0.,
            moving: false,
        }
    }

    /// The pointer at `x`; true when the strip has to be painted again.
    pub fn moved(&mut self, x: f64, font_px: f64, ui_scale: f32) -> bool {
        self.dx = x - self.start_x;
        let slop = TAB_DRAG_SLOP_RATIO * font_px * ui_scale as f64;
        if !self.moving && self.dx.abs() >= slop {
            self.moving = true;
        }
        self.moving
    }

    /// Where the tab would land if dropped now.
    pub fn to(&self, font_px: f64, ui_scale: f32, tab_count: usize) -> usize {
        let tab_w = TAB_STRIP_TAB_WIDTH_RATIO * font_px * ui_scale as f64;
        drop_index(self.from, self.dx, tab_w, tab_count)
    }

    /// The release: the move to make, if the tab went anywhere.
    pub fn drop(&self, font_px: f64, ui_scale: f32, tab_count: usize) -> Option<(usize, usize)> {
        let to = self.to(font_px, ui_scale, tab_count);
        (self.moving && to != self.from).then_some((self.from, to))
    }
}

/// `dx` held to the strip: the dragged tab stops at the first and last
/// places rather than sliding off over the "+" or the folder header.
fn clamped_dx(from: usize, dx: f64, tab_w: f64, tab_count: usize) -> f64 {
    let last = tab_count.saturating_sub(1) as f64;
    dx.clamp(-(from as f64) * tab_w, (last - from as f64) * tab_w)
}

/// The place whose slot holds the dragged tab's middle: it takes that
/// place, the way a dragged workspace tab takes the one it is over.
pub fn drop_index(from: usize, dx: f64, tab_w: f64, tab_count: usize) -> usize {
    if tab_count == 0 || tab_w <= 0. {
        return from;
    }
    let mid = (from as f64 + 0.5) * tab_w + clamped_dx(from, dx, tab_w, tab_count);
    ((mid / tab_w).floor().max(0.) as usize).min(tab_count - 1)
}

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
    /// `cardTab.hoverDetail`: the hover label's path or address line.
    pub hover_detail: bool,
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
        hover_detail: theme.hover_detail.unwrap_or(false),
    }
}

/// The hover label's lines (#742): the tab's full name, then `detail` (the
/// file's path, the page's address) when `show_detail` allows it. A detail
/// that is empty or only repeats the name (an untitled page is labelled with
/// its address) adds no line.
pub fn hover_lines(name: &str, detail: Option<&str>, show_detail: bool) -> Vec<String> {
    let mut lines = vec![name.to_string()];
    if let Some(d) = detail.filter(|d| show_detail && !d.is_empty() && *d != name) {
        lines.push(d.to_string());
    }
    lines
}

/// Where the hover label's box starts, from the strip's left edge: under the
/// tab's left edge, moved left as far as it must to stay inside a card
/// `card_w` wide, and never past the card's left edge.
pub fn hover_x(tab_x: f32, box_w: f32, card_w: f32) -> f32 {
    tab_x.min(card_w - box_w).max(0.)
}

/// The tab a point is over, its `×` included: the one whose label to show.
pub fn hovered(hit: Option<TabClick>) -> Option<usize> {
    match hit {
        Some(TabClick::Switch(i) | TabClick::Close(i)) => Some(i),
        _ => None,
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
/// `drag` is a tab being dragged along the strip, if one is.
#[allow(clippy::too_many_arguments)]
pub fn paint_strip(
    bounds: Bounds<Pixels>,
    scale: f64,
    ui_scale: f32,
    labels: &[String],
    active: usize,
    drag: Option<&TabDrag>,
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
        // A drag (#682): the others sit where the drop would leave them,
        // the held tab follows the pointer and is painted last, over them.
        let world_tab_w = TAB_STRIP_TAB_WIDTH_RATIO * style.font_px * ui_scale as f64;
        let drag = drag.filter(|d| d.moving && d.from < labels.len());
        let to = drag.map(|d| drop_index(d.from, d.dx, world_tab_w, labels.len()));
        let mut order: Vec<usize> = (0..labels.len())
            .filter(|&i| Some(i) != drag.map(|d| d.from))
            .collect();
        order.extend(drag.map(|d| d.from));
        for i in order {
            let label = &labels[i];
            let x = match (drag, to) {
                (Some(d), _) if d.from == i => {
                    let dx = clamped_dx(d.from, d.dx, world_tab_w, labels.len());
                    tab_w * (i as f32) + px((dx * scale) as f32)
                }
                (Some(d), Some(to)) => tab_w * (index_after_move(i, d.from, to) as f32),
                _ => tab_w * (i as f32),
            };
            let tab_bounds = Bounds::new(
                point(strip.origin.x + left + x, strip.origin.y),
                size(tab_w, strip_h),
            );
            let pill = pill_in(tab_bounds);
            let is_active = i == active;
            let held = drag.is_some_and(|d| d.from == i);
            // An idle tab may have no fill of its own: a held one gets the
            // strip's ground so it hides the tabs it passes over.
            if held && !is_active {
                window.paint_quad(fill(pill, style.bg).corner_radii(radius));
            }
            paint_tab_box(window, pill, radius, border, &style.tabs, is_active);
            // Between two idle neighbours only: no line touches the chosen tab.
            if let Some(c) = style.tabs.separator {
                if i > 0 && !is_active && i - 1 != active && !held {
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

/// Draws the hover label of tab `index` (#742) under the strip, inside the
/// card's `bounds`: a box in the strip's colours holding `lines`, the first
/// bright, the rest muted. `left` is the folder header's width (world
/// units, 0 without one), as `paint_strip` takes it. Call it after the
/// card's content so the label sits over it. Nothing while the strip's
/// text is too small to draw, as the labels themselves.
#[allow(clippy::too_many_arguments)]
pub fn paint_hover_label(
    bounds: Bounds<Pixels>,
    scale: f64,
    ui_scale: f32,
    index: usize,
    left: f64,
    lines: &[String],
    style: &StripStyle,
    window: &mut Window,
    cx: &mut App,
) {
    let unit = style.font_px * ui_scale as f64 * scale;
    let px_scale = ui_scale as f64 * scale;
    let strip_font = px(unit as f32);
    if lines.is_empty() || strip_font < crate::chrome::legible_font_px(window.scale_factor()) {
        return;
    }
    let line_h = px((unit * TAB_LINE_HEIGHT_RATIO) as f32);
    let pad_x = px((style.pad_x() * px_scale) as f32);
    let pad_y = px((TAB_PAD_Y_PX * px_scale) as f32);
    // A line wider than the card is cut with an ellipsis: the box never
    // leaves the card it belongs to.
    let room = f32::from(bounds.size.width - pad_x * 2.);
    let shaped: Vec<_> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let color = if i == 0 {
                style.text_bright
            } else {
                style.text_muted
            };
            let shown = crate::text::elide(l, room, |t| {
                f32::from(crate::text::shape(window, t, strip_font, &style.regular, color).width)
            });
            crate::text::shape(window, &shown, strip_font, &style.regular, color)
        })
        .collect();
    let widest = shaped.iter().map(|l| l.width).fold(px(0.), |a, b| a.max(b));
    let box_w = (widest + pad_x * 2.).min(bounds.size.width);
    let box_h = line_h * shaped.len() as f32 + pad_y * 2.;
    let tab_w = TAB_STRIP_TAB_WIDTH_RATIO * unit;
    let tab_x = (left * scale + index as f64 * tab_w) as f32;
    let x = hover_x(tab_x, f32::from(box_w), f32::from(bounds.size.width));
    let strip_h = px((strip_world_h(style.font_px, ui_scale) * scale) as f32);
    let gap = px((STRIP_INSET_Y_PX * px_scale) as f32);
    let rect = Bounds::new(
        point(bounds.origin.x + px(x), bounds.origin.y + strip_h + gap),
        size(box_w, box_h),
    );
    let radius = px((style.tabs.radius.map_or(TAB_RADIUS_PX, f64::from) * px_scale) as f32);
    let border = px((TAB_STRIP_BORDER_PX * px_scale) as f32);
    window.paint_quad(quad(
        rect,
        radius,
        style.bg,
        border,
        style.border,
        BorderStyle::Solid,
    ));
    window.with_content_mask(Some(gpui::ContentMask { bounds: rect }), |window| {
        for (i, line) in shaped.iter().enumerate() {
            let row = Bounds::new(
                point(rect.origin.x, rect.origin.y + pad_y + line_h * i as f32),
                size(box_w, line_h),
            );
            crate::text::paint_in(window, cx, line, row, pad_x);
        }
    });
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

    #[test]
    fn a_hovered_tab_shows_its_full_name_then_its_path_or_address() {
        let path = "~/.config/infiniterm/settings.json";
        assert_eq!(
            hover_lines("settings.json", Some(path), true),
            ["settings.json", path]
        );
        // `cardTab.hoverDetail` false: the name alone.
        assert_eq!(
            hover_lines("settings.json", Some(path), false),
            ["settings.json"]
        );
        // An untitled buffer has no path; an untitled page is its address.
        assert_eq!(hover_lines("untitled", None, true), ["untitled"]);
        let url = "https://example.com/";
        assert_eq!(hover_lines(url, Some(url), true), [url]);
        assert_eq!(hover_lines("Docs", Some(""), true), ["Docs"]);
    }

    #[test]
    fn the_hover_label_stays_inside_its_card() {
        // Room under the tab: it starts at the tab's left edge.
        assert_eq!(hover_x(90., 200., 600.), 90.);
        // The last tab of a narrow card: moved left to end at the card's edge.
        assert_eq!(hover_x(450., 200., 600.), 400.);
        // Wider than the card: from the card's left edge.
        assert_eq!(hover_x(90., 800., 600.), 0.);
    }

    #[test]
    fn the_pointer_over_a_tab_or_its_close_hovers_that_tab() {
        assert_eq!(hovered(hit(10., 10., 1., 2, 0.)), Some(0));
        assert_eq!(hovered(hit(98., 10., 1., 2, 0.)), Some(1));
        assert_eq!(hovered(hit(160., 10., 1., 2, 0.)), Some(1));
        // The "+", past the tabs, and below the strip hover nothing.
        assert_eq!(hovered(hit(190., 10., 1., 2, 0.)), None);
        assert_eq!(hovered(hit(10., 200., 1., 2, 0.)), None);
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
    #[allow(clippy::approx_constant)] // gpui's phi, as TAB_LINE_HEIGHT_RATIO
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

    // A dragged tab (#682) takes the place its middle is over, held to the
    // strip's ends. At a 10 px font a tab is 90 wide; three tabs here.
    #[test]
    fn a_dragged_tab_lands_where_its_middle_is() {
        assert_eq!(drop_index(0, 0., 90., 3), 0);
        assert_eq!(
            drop_index(0, 44., 90., 3),
            0,
            "not yet past the next tab's edge"
        );
        assert_eq!(drop_index(0, 46., 90., 3), 1);
        assert_eq!(
            drop_index(0, 500., 90., 3),
            2,
            "the last place, not past it"
        );
        assert_eq!(drop_index(2, -136., 90., 3), 0);
        assert_eq!(drop_index(1, -500., 90., 3), 0);
        assert_eq!(drop_index(0, 10., 90., 0), 0, "no tabs, nothing moves");
    }

    // Under the slop a press is only the click that switched tabs.
    #[test]
    fn a_drag_starts_past_the_slop_and_drops_only_when_it_moved() {
        let mut d = TabDrag::new(0, 20.);
        assert!(!d.moved(23., 10., 1.), "3 is under the 5 slop");
        assert_eq!(d.drop(10., 1., 3), None);
        assert!(d.moved(120., 10., 1.));
        assert_eq!(d.drop(10., 1., 3), Some((0, 1)));
        // Back where it started: a drag, but no move to make.
        d.moved(22., 10., 1.);
        assert!(d.moving);
        assert_eq!(d.drop(10., 1., 3), None);
    }

    // The lock is `card.protect`'s, not this strip's own `locked`: it
    // shows or not by the card's saved flag alone.
    #[test]
    fn a_protected_card_wears_the_lock_glyph_beside_its_number() {
        assert_eq!(number_label(7, false), "#7");
        assert_eq!(number_label(7, true), "\u{1f512} #7");
    }
}
