//! The text of "Custom size…" (`card.size.custom`): "60x90", "60 x 90",
//! "60 90" or "60,90" as a card size. Pure: text in, a size or a plain
//! sentence out.
//!
//! The unit is the grid square (25 px), the same as `cards.width` and
//! `cards.height` (50 is 1250 px). Decimals are accepted and rounded to the
//! nearest square, because card sizes live on the grid everywhere else
//! (`grid::snap`). Below the minimum card the size is clamped, not refused
//! (`Parsed::clamped` says so); more than `MAX_SQUARES` a side is refused, a
//! typo like 600x900 is the likelier meaning than a card 15000 px wide.
//!
//! Called from `Model::answer` (`Pending::CustomSize`, `model/window_fit_cmd.rs`),
//! which feeds the size to `window_fit::reflow_to`. Related: `resize.rs` (the
//! minimum card), `config.rs` (the cards.width clamp).
use crate::grid::{snap, Size, GRID_SIZE};
use crate::resize::{MIN_CARD_H, MIN_CARD_W};

/// The most squares a side may have: 400 is 10000 px.
pub const MAX_SQUARES: f64 = 400.;

/// A parsed size, in pixels on the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parsed {
    pub size: Size,
    /// The text asked for less than the minimum card on a side.
    pub clamped: bool,
}

/// `text` as a card size, or the sentence that says what is wrong.
pub fn parse(text: &str) -> Result<Parsed, String> {
    // `x` (and the multiplication sign) and `,` are separators like a space.
    let spaced: String = text
        .chars()
        .map(|c| {
            if matches!(c, 'x' | 'X' | '×' | ',') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let words: Vec<&str> = spaced.split_whitespace().collect();
    let [w, h] = words[..] else {
        return Err("type two numbers: width and height in grid squares, like 60x90".into());
    };
    let number = |s: &str| {
        s.parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("\"{s}\" is not a number; type width and height, like 60x90"))
    };
    let (w, h) = (number(w)?, number(h)?);
    if w <= 0. || h <= 0. {
        return Err("width and height must be more than zero".into());
    }
    if w > MAX_SQUARES || h > MAX_SQUARES {
        return Err(format!(
            "at most {MAX_SQUARES} squares ({} px) on a side",
            MAX_SQUARES * GRID_SIZE
        ));
    }
    let (px_w, px_h) = (snap(w * GRID_SIZE), snap(h * GRID_SIZE));
    Ok(Parsed {
        size: Size {
            w: px_w.max(MIN_CARD_W),
            h: px_h.max(MIN_CARD_H),
        },
        clamped: px_w < MIN_CARD_W || px_h < MIN_CARD_H,
    })
}

/// A size as the prompt shows it: whole squares without a decimal point.
pub fn squares(size: Size) -> String {
    format!("{}x{}", size.w / GRID_SIZE, size.h / GRID_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> Size {
        parse(text).unwrap_or_else(|e| panic!("{text:?}: {e}")).size
    }

    #[test]
    fn every_separator_gives_the_same_size() {
        for t in [
            "60x90", "60 x 90", "60 90", "60,90", "60, 90", " 60X90 ", "60×90", "60  90",
        ] {
            assert_eq!(ok(t), Size { w: 1500., h: 2250. }, "{t}");
        }
    }

    #[test]
    fn decimals_round_to_the_nearest_square() {
        assert_eq!(ok("60.4x90.6"), Size { w: 1500., h: 2275. });
        assert_eq!(
            ok("50.5 80"),
            Size { w: 1275., h: 2000. },
            "a half rounds up"
        );
    }

    #[test]
    fn junk_says_what_to_type() {
        for t in [
            "", "   ", "60", "60x", "x90", "60x90x30", "60 90 30", "abc", "60xabc", "nan x 90",
            "inf 90",
        ] {
            let e = parse(t).unwrap_err();
            assert!(
                e.contains("60x90") || e.contains("not a number"),
                "{t}: {e}"
            );
        }
        assert!(parse("abc 90").unwrap_err().contains("\"abc\""));
    }

    #[test]
    fn zero_and_negative_are_refused() {
        for t in ["0x90", "60x0", "-5x90", "60 -1", "0 0"] {
            assert!(parse(t).unwrap_err().contains("more than zero"), "{t}");
        }
    }

    #[test]
    fn a_huge_side_is_refused_and_the_limit_itself_is_allowed() {
        assert!(parse("401x80").unwrap_err().contains("at most 400"));
        assert!(parse("60x1000").is_err());
        assert!(parse("1e9 80").is_err());
        assert_eq!(
            ok("400x400"),
            Size {
                w: 10000.,
                h: 10000.
            }
        );
    }

    #[test]
    fn under_the_minimum_card_is_clamped_and_says_so() {
        let p = parse("2x2").unwrap();
        assert_eq!(
            p.size,
            Size {
                w: MIN_CARD_W,
                h: MIN_CARD_H
            }
        );
        assert!(p.clamped);
        let p = parse("8x3").unwrap();
        assert_eq!(
            p.size,
            Size {
                w: MIN_CARD_W,
                h: MIN_CARD_H
            }
        );
        assert!(p.clamped, "one side is enough");
        assert!(
            !parse("8x4").unwrap().clamped,
            "exactly the minimum is not clamped"
        );
        assert!(!parse("60x90").unwrap().clamped);
    }

    #[test]
    fn squares_prints_what_the_parser_reads() {
        assert_eq!(squares(Size { w: 1250., h: 2000. }), "50x80");
        assert_eq!(squares(Size { w: 1275., h: 2000. }), "51x80");
        assert_eq!(
            ok(&squares(Size { w: 1500., h: 2250. })),
            Size { w: 1500., h: 2250. }
        );
    }
}
