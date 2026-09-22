//! Spike: render a terminal card's text to a bitmap with CoreText, the way
//! the browser card already hands gpui a CEF texture, instead of painting
//! every glyph every frame.
//!
//! The question is cost and looks. Cost: painting a card's glyphs through
//! gpui is 1.7 us each and happens EVERY FRAME (twelve cards at fit-all
//! measured 85 to 100 ms), while a texture is rendered once per output
//! change and every pan and zoom after that is a blit. Looks: bars are what
//! a far card draws now, and Ekin asked whether a picture of the text would
//! read better.
//!
//! Headless on purpose: no window, no gpui. It writes PNGs at the size a
//! card really is at fit-all so the comparison can be looked at in an
//! editor card, and prints the milliseconds.
//!
//! What it does NOT answer: gpui's sprite atlas under a dozen live
//! textures (`window.paint_image` uploads a tile per image id and only
//! `window.drop_image` frees it), and colour fidelity against our palette.
use core_foundation::attributed_string::CFMutableAttributedString;
use core_foundation::base::{CFRange, TCFType};
use core_foundation::string::CFString;
use core_graphics::color::CGColor;
use core_graphics::color_space::CGColorSpace;
use core_graphics::context::CGContext;
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use core_text::font as ct_font;
use core_text::line::CTLine;
use core_text::string_attributes::{kCTFontAttributeName, kCTForegroundColorAttributeName};
use std::time::Instant;

/// Ekin's canvas: a full card is 1725 x 2000 world units at 19 px font and
/// 1.2 line height, which is this grid.
const COLS: usize = 151;
const ROWS: usize = 87;
/// What his fit-all lands at: 19 px font x 0.507 scale.
const FAR_FONT_PX: f64 = 9.63;
const LINE_HEIGHT: f64 = 1.2;

/// A span of one colour inside a row.
struct Span {
    text: String,
    rgb: (f64, f64, f64),
}

fn palette(i: usize) -> (f64, f64, f64) {
    // Roughly a dark theme's foreground, comment, string, keyword, warn.
    const P: [(f64, f64, f64); 5] = [
        (0.85, 0.87, 0.91),
        (0.45, 0.50, 0.58),
        (0.55, 0.78, 0.55),
        (0.60, 0.70, 0.95),
        (0.95, 0.70, 0.35),
    ];
    P[i % P.len()]
}

/// A card's worth of output shaped like a Claude session: a box, prose that
/// wraps, tool lines, indented code, blank rows.
fn sample_card() -> Vec<Vec<Span>> {
    let mut rows: Vec<Vec<Span>> = vec![];
    let sp = |t: &str, c: usize| Span {
        text: t.to_string(),
        rgb: palette(c),
    };
    rows.push(vec![sp(
        "╭──────────────────────────────────────────────────────────────────────────╮",
        3,
    )]);
    rows.push(vec![
        sp("│ ", 3),
        sp("infiniterm", 0),
        sp("  ~/Code/infiniterm  ", 1),
        sp("master", 4),
        sp(" *", 1),
    ]);
    rows.push(vec![sp(
        "╰──────────────────────────────────────────────────────────────────────────╯",
        3,
    )]);
    rows.push(vec![]);
    for i in 0..(ROWS - 4) {
        match i % 7 {
            0 => rows.push(vec![
                sp("● ", 4),
                sp("Read", 0),
                sp("(infiniterm-ui/src/terminal_body.rs)", 1),
            ]),
            1 => rows.push(vec![
                sp("  ⎿  ", 1),
                sp("Read 1,943 lines", 2),
            ]),
            2 => rows.push(vec![sp(
                "The cost is the total number of glyphs, not any one card's font size, because a card's grid",
                0,
            )]),
            3 => rows.push(vec![sp(
                "does not shrink when the canvas does. Twelve cards at fit-all is 157,000 cells.",
                0,
            )]),
            4 => rows.push(vec![
                sp("    let ", 3),
                sp("legible", 0),
                sp(" = font_size >= ", 0),
                sp("LEGIBLE_FONT_PX", 4),
                sp(";", 0),
            ]),
            5 => rows.push(vec![sp("", 0)]),
            _ => rows.push(vec![
                sp("  ", 0),
                sp("\"a string in the output\"", 2),
                sp("  // and a trailing comment", 1),
            ]),
        }
    }
    rows.truncate(ROWS);
    rows
}

/// A bitmap context the size gpui would paint into, BGRA like `RenderImage`.
fn context(w: usize, h: usize) -> CGContext {
    CGContext::create_bitmap_context(
        None,
        w,
        h,
        8,
        w * 4,
        &CGColorSpace::create_device_rgb(),
        // Premultiplied first + little endian = BGRA in memory.
        core_graphics::base::kCGImageAlphaPremultipliedFirst
            | core_graphics::base::kCGBitmapByteOrder32Little,
    )
}

fn ground(ctx: &CGContext, w: f64, h: f64) {
    ctx.set_rgb_fill_color(0.055, 0.063, 0.102, 1.);
    ctx.fill_rect(CGRect::new(&CGPoint::new(0., 0.), &CGSize::new(w, h)));
}

/// One CTLine per row, with a colour run per span: the fast path, and what
/// a terminal row already is.
fn draw_rows(ctx: &CGContext, rows: &[Vec<Span>], font: &ct_font::CTFont, cell_w: f64, line_h: f64) {
    let ascent = font.ascent();
    for (r, spans) in rows.iter().enumerate() {
        if spans.is_empty() {
            continue;
        }
        let mut attr = CFMutableAttributedString::new();
        let mut at = 0_isize;
        for span in spans {
            let s = CFString::new(&span.text);
            let len = span.text.encode_utf16().count() as isize;
            attr.replace_str(&s, CFRange::init(at, 0));
            let range = CFRange::init(at, len);
            unsafe {
                attr.set_attribute(range, kCTFontAttributeName, font);
                attr.set_attribute(
                    range,
                    kCTForegroundColorAttributeName,
                    &CGColor::rgb(span.rgb.0, span.rgb.1, span.rgb.2, 1.),
                );
            }
            at += len;
        }
        let line = CTLine::new_with_attributed_string(attr.as_concrete_TypeRef());
        // The grid's y, from the top, as our painter does it.
        let baseline = (rows.len() as f64 - r as f64) * line_h - line_h + (line_h - ascent) / 2.;
        ctx.set_text_position(0., baseline);
        line.draw(ctx);
    }
    let _ = cell_w;
}

/// What a far card draws today: a bar per word, at the run's own width.
fn draw_bars(ctx: &CGContext, rows: &[Vec<Span>], cell_w: f64, line_h: f64) {
    ctx.set_rgb_fill_color(0.85, 0.87, 0.91, 0.45);
    let bar_h = (line_h * 0.55).max(1.);
    for (r, spans) in rows.iter().enumerate() {
        let mut col = 0_usize;
        for span in spans {
            for word in span.text.split_inclusive(' ') {
                let trimmed = word.trim_end();
                let n = trimmed.chars().count();
                if n > 0 {
                    let x = col as f64 * cell_w;
                    let y = (rows.len() as f64 - r as f64) * line_h - line_h + (line_h - bar_h) / 2.;
                    ctx.fill_rect(CGRect::new(
                        &CGPoint::new(x, y),
                        &CGSize::new(n as f64 * cell_w, bar_h),
                    ));
                }
                col += word.chars().count();
            }
        }
    }
}

fn save(ctx: &mut CGContext, w: usize, h: usize, path: &str) {
    // BGRA premultiplied out, RGBA in: swap and drop the premultiply.
    let data = ctx.data().to_vec();
    let mut out = image::RgbaImage::new(w as u32, h as u32);
    for y in 0..h {
        for x in 0..w {
            let i = y * w * 4 + x * 4;
            let (b, g, r, a) = (data[i], data[i + 1], data[i + 2], data[i + 3]);
            out.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, a]));
        }
    }
    out.save(path).unwrap();
    println!("wrote {path}");
}

fn main() {
    let family = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "IosevkaTerm Nerd Font Mono".to_string());
    let rows = sample_card();
    let font_px = FAR_FONT_PX;
    let font = ct_font::new_from_name(&family, font_px)
        .unwrap_or_else(|_| ct_font::new_from_name("Menlo", font_px).unwrap());
    // The cell is the advance of M, as the app's metrics are.
    let cell_w = {
        let s = CFString::new("M");
        let mut a = CFMutableAttributedString::new();
        a.replace_str(&s, CFRange::init(0, 0));
        unsafe { a.set_attribute(CFRange::init(0, 1), kCTFontAttributeName, &font) };
        CTLine::new_with_attributed_string(a.as_concrete_TypeRef())
            .get_typographic_bounds()
            .width
    };
    let line_h = font_px * LINE_HEIGHT;
    let w = (cell_w * COLS as f64).ceil() as usize;
    let h = (line_h * ROWS as f64).ceil() as usize;
    println!(
        "card {COLS}x{ROWS} cells at {font_px:.2} px -> texture {w}x{h} px, {:.1} MB",
        (w * h * 4) as f64 / 1e6
    );

    // Cold, then warm: the first render pays for the font's glyph cache.
    let mut times = vec![];
    for i in 0..12 {
        let mut ctx = context(w, h);
        let t = Instant::now();
        ground(&ctx, w as f64, h as f64);
        draw_rows(&ctx, &rows, &font, cell_w, line_h);
        let ms = t.elapsed().as_secs_f64() * 1000.;
        times.push(ms);
        if i == 0 {
            save(&mut ctx, w, h, "/tmp/card-texture-text.png");
        }
    }
    println!(
        "text: first {:.1} ms, then {:.1} ms mean over {} (twelve cards: {:.0} ms)",
        times[0],
        times[1..].iter().sum::<f64>() / (times.len() - 1) as f64,
        times.len() - 1,
        times[1..].iter().sum::<f64>() / (times.len() - 1) as f64 * 12.
    );

    let mut ctx = context(w, h);
    let t = Instant::now();
    ground(&ctx, w as f64, h as f64);
    draw_bars(&ctx, &rows, cell_w, line_h);
    println!("bars: {:.1} ms", t.elapsed().as_secs_f64() * 1000.);
    save(&mut ctx, w, h, "/tmp/card-texture-bars.png");

    // And at the size a 4x2 fit-all gives, to see how far this reads.
    for (name, px) in [("fitall", FAR_FONT_PX), ("tiny", 6.0), ("half", 14.0)] {
        let font = ct_font::new_from_name(&family, px)
            .unwrap_or_else(|_| ct_font::new_from_name("Menlo", px).unwrap());
        let cw = cell_w * px / FAR_FONT_PX;
        let lh = px * LINE_HEIGHT;
        let (w, h) = (
            (cw * COLS as f64).ceil() as usize,
            (lh * ROWS as f64).ceil() as usize,
        );
        let mut ctx = context(w, h);
        let t = Instant::now();
        ground(&ctx, w as f64, h as f64);
        draw_rows(&ctx, &rows, &font, cw, lh);
        println!(
            "{name} ({px:.1} px, {w}x{h}): {:.1} ms, {:.1} MB",
            t.elapsed().as_secs_f64() * 1000.,
            (w * h * 4) as f64 / 1e6
        );
        save(&mut ctx, w, h, &format!("/tmp/card-texture-{name}.png"));
    }
}
