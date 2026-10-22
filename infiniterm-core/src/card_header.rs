//! The card header (#544): a row inside the card, above its body, with the
//! program's icon, the card's name and a few buttons. Off unless a ui theme
//! sets `card.header`; off, a card is exactly as it was (the name is a chip in
//! the corner, `paint_labels`).
//!
//! Where it fits: the ui subtracts `inset` from the card's height wherever it
//! sizes a body (`fit_size`, `resized`, the painted bounds) and from a mouse
//! position's `y` wherever it hands one to a body (`input.rs`), so a body
//! never knows the header is there. `button_at` answers a click in the header.
//! Every button runs a registered command, so the palette and the keymap reach
//! the same things (nothing is mouse-only).
//! Related: `ui_theme.rs` (`card.header*` keys), `tab_icon.rs` (the icon),
//! infiniterm-ui/src/paint.rs (`paint_header`).
//!
//! All lengths are WORLD units, so the header scales with the card and a
//! terminal's rows come out the same at any zoom.
use crate::grid::Rect;
use crate::saved_layout::CardKind;

/// The header's height in world units.
pub const HEADER_H: f64 = 30.;
/// One button's width in world units.
pub const BUTTON_W: f64 = 28.;
/// Space between the last button and the card's right edge.
pub const BUTTON_MARGIN: f64 = 6.;

/// The buttons a theme may list, in `card.headerButtons`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderButton {
    SplitRight,
    SplitDown,
    Maximise,
    Close,
}

impl HeaderButton {
    pub const NAMES: [&'static str; 4] = ["split-right", "split-down", "maximise", "close"];

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "split-right" => Some(Self::SplitRight),
            "split-down" => Some(Self::SplitDown),
            "maximise" => Some(Self::Maximise),
            "close" => Some(Self::Close),
            _ => None,
        }
    }

    /// The registered command the button runs.
    pub fn command(self) -> &'static str {
        match self {
            Self::SplitRight => "card.split.right",
            Self::SplitDown => "card.split.down",
            Self::Maximise => "card.maximize.toggle",
            Self::Close => "card.close",
        }
    }

    /// A Nerd Font glyph: columns, rows, expand, close.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::SplitRight => "\u{f0db}",
            Self::SplitDown => "\u{f0c9}",
            Self::Maximise => "\u{f065}",
            Self::Close => "\u{f00d}",
        }
    }
}

/// The buttons a `card.headerButtons` list names, in order, without repeats.
/// `None` for an unknown name, so a theme with a typo is refused.
pub fn parse_buttons(names: &[String]) -> Option<Vec<HeaderButton>> {
    let mut out: Vec<HeaderButton> = vec![];
    for n in names {
        let b = HeaderButton::parse(n)?;
        if !out.contains(&b) {
            out.push(b);
        }
    }
    Some(out)
}

/// Whether a card of this kind can wear the header. An editor, a diff and a
/// browser already have their own strip with the title, the tabs and their
/// buttons; a second row over it would say everything twice.
pub fn applies(kind: CardKind) -> bool {
    matches!(
        kind,
        CardKind::Terminal | CardKind::Transcript | CardKind::Page
    )
}

/// World units the header takes from the top of a card: 0 when `card.header`
/// is off or the kind has its own strip.
pub fn inset(on: bool, kind: CardKind) -> f64 {
    if on && applies(kind) {
        HEADER_H
    } else {
        0.
    }
}

/// Where each button sits, in card-local world units, right-aligned in a
/// header `width` wide. A button that would not fit is left out, from the
/// left, so a narrow card keeps its close button.
pub fn button_rects(width: f64, buttons: &[HeaderButton]) -> Vec<(HeaderButton, Rect)> {
    let mut out = vec![];
    let mut right = width - BUTTON_MARGIN;
    for b in buttons.iter().rev() {
        let x = right - BUTTON_W;
        if x < 0. {
            break;
        }
        out.push((
            *b,
            Rect {
                x,
                y: 0.,
                w: BUTTON_W,
                h: HEADER_H,
            },
        ));
        right = x;
    }
    out.reverse();
    out
}

/// The button under a card-local point, if any.
pub fn button_at(width: f64, buttons: &[HeaderButton], x: f64, y: f64) -> Option<HeaderButton> {
    button_rects(width, buttons)
        .into_iter()
        .find(|(_, r)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
        .map(|(b, _)| b)
}

/// Whether a card-local `y` is inside the header.
pub fn in_header(inset: f64, y: f64) -> bool {
    y >= 0. && y < inset
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [HeaderButton; 4] = [
        HeaderButton::SplitRight,
        HeaderButton::SplitDown,
        HeaderButton::Maximise,
        HeaderButton::Close,
    ];

    #[test]
    fn the_header_takes_space_only_when_on_and_only_from_kinds_without_a_strip() {
        assert_eq!(inset(false, CardKind::Terminal), 0.);
        assert_eq!(inset(true, CardKind::Terminal), HEADER_H);
        assert_eq!(inset(true, CardKind::Transcript), HEADER_H);
        assert_eq!(inset(true, CardKind::Page), HEADER_H);
        for kind in [CardKind::Editor, CardKind::Diff, CardKind::Browser] {
            assert_eq!(inset(true, kind), 0., "{kind:?} has its own strip");
        }
    }

    #[test]
    fn button_names_parse_and_each_runs_a_command() {
        for (name, b) in HeaderButton::NAMES.iter().zip(ALL) {
            assert_eq!(HeaderButton::parse(name), Some(b));
            assert!(!b.command().is_empty() && !b.glyph().is_empty());
        }
        assert_eq!(HeaderButton::parse("minimise"), None);
        let list = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse_buttons(&list(&["close", "split-right", "close"])),
            Some(vec![HeaderButton::Close, HeaderButton::SplitRight])
        );
        assert_eq!(parse_buttons(&list(&["close", "nope"])), None);
        assert_eq!(parse_buttons(&[]), Some(vec![]));
    }

    #[test]
    fn buttons_sit_at_the_right_in_the_order_listed() {
        let rects = button_rects(300., &ALL);
        assert_eq!(rects.len(), 4);
        let xs: Vec<f64> = rects.iter().map(|(_, r)| r.x).collect();
        assert!(xs.windows(2).all(|w| w[0] + BUTTON_W == w[1]), "{xs:?}");
        let (last, r) = rects.last().unwrap();
        assert_eq!(*last, HeaderButton::Close);
        assert_eq!(r.x + r.w, 300. - BUTTON_MARGIN);
    }

    #[test]
    fn a_narrow_card_keeps_the_rightmost_buttons() {
        let rects = button_rects(70., &ALL);
        let kept: Vec<HeaderButton> = rects.iter().map(|(b, _)| *b).collect();
        assert_eq!(kept, vec![HeaderButton::Maximise, HeaderButton::Close]);
        assert!(button_rects(10., &ALL).is_empty());
    }

    #[test]
    fn a_click_finds_its_button_and_nothing_outside_the_header() {
        let w = 300.;
        let close = button_rects(w, &ALL).last().unwrap().1;
        assert_eq!(
            button_at(w, &ALL, close.x + 1., 5.),
            Some(HeaderButton::Close)
        );
        assert_eq!(button_at(w, &ALL, 5., 5.), None);
        assert_eq!(button_at(w, &ALL, close.x + 1., HEADER_H + 1.), None);
        assert!(in_header(HEADER_H, 0.) && in_header(HEADER_H, HEADER_H - 0.1));
        assert!(!in_header(HEADER_H, HEADER_H) && !in_header(0., 0.));
    }
}
