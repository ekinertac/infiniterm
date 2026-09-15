//! Shaped text for the chrome: one call that shapes a string in one font
//! and colour and returns the line, since every label, badge and status
//! item is that. gpui shapes per call; the strings here are short.
use gpui::{point, App, Bounds, Font, Hsla, Pixels, ShapedLine, SharedString, TextRun, Window};

pub fn shape(window: &Window, text: &str, size: Pixels, font: &Font, color: Hsla) -> ShapedLine {
    window.text_system().shape_line(
        SharedString::from(text.to_string()),
        size,
        &[TextRun {
            len: text.len(),
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    )
}

/// Paints `text` inside `bounds`, vertically centred, `pad` from the left.
pub fn paint_in(
    window: &mut Window,
    cx: &mut App,
    line: &ShapedLine,
    bounds: Bounds<Pixels>,
    pad: Pixels,
) {
    let height = bounds.size.height;
    let _ = line.paint(
        point(bounds.origin.x + pad, bounds.origin.y),
        height,
        window,
        cx,
    );
}
