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

/// The ellipsis the label rules cut with.
pub const ELLIPSIS: char = '\u{2026}';

/// Shortens `text` from the END until `measure` says it fits `room`, with an
/// ellipsis where it was cut. Returns `text` unchanged when it already fits.
///
/// The card labels ellipsise the HEAD by preference, because the last path
/// segment is what tells two cards apart. This is the fallback for a label
/// that has no head to give: an agent's session name is one long phrase with
/// no slash in it, and before this it simply ran off the card and over the
/// neighbours.
///
/// Binary search over the character boundaries, so a long label costs a
/// handful of shaping calls rather than one per character.
pub fn elide(text: &str, room: f32, measure: impl Fn(&str) -> f32) -> String {
    if measure(text) <= room || text.is_empty() {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let cut = |n: usize| -> String {
        let mut s: String = chars[..n].iter().collect();
        s.push(ELLIPSIS);
        s
    };
    // Nothing fits, not even one character and the ellipsis.
    if measure(&cut(0)) > room {
        return String::new();
    }
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if measure(&cut(mid)) <= room {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    cut(lo)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A fake measure: every character is ten wide, so the arithmetic in the
    // test is the arithmetic a reader can do.
    fn measure(s: &str) -> f32 {
        s.chars().count() as f32 * 10.
    }

    #[test]
    fn what_fits_is_left_alone() {
        assert_eq!(elide("scan", 100., measure), "scan");
        assert_eq!(elide("scan", 40., measure), "scan");
        assert_eq!(elide("", 0., measure), "");
    }

    #[test]
    fn what_does_not_fit_is_cut_with_an_ellipsis() {
        // Room for five characters, one of which is the ellipsis.
        assert_eq!(elide("scan network", 50., measure), "scan…");
        assert_eq!(elide("scan network", 20., measure), "s…");
    }

    // A width that cannot hold even one character plus the ellipsis gets
    // nothing rather than a lone ellipsis pretending to be a label.
    #[test]
    fn a_hopeless_width_gets_nothing() {
        assert_eq!(elide("scan", 5., measure), "");
    }

    // Characters, never bytes: a cut inside a multi-byte character would
    // panic, and every one of these labels can hold one.
    #[test]
    fn it_cuts_on_characters() {
        assert_eq!(elide("◐ ağ taraması", 50., measure), "◐ ağ…");
    }
}
