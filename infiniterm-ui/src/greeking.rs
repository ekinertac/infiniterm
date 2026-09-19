//! Text too small to read, drawn as its silhouette: what page-layout
//! programs call greeking. Below `chrome::LEGIBLE_FONT_PX` a glyph costs
//! more than it shows (1.7 µs of CPU each in gpui, tens of thousands a
//! frame at fit-all), and a flat bar per word showed only that text was
//! there. A word's SHAPE survives at 5 px where its letters do not: the
//! x-height letters sit low, the ascenders and capitals reach up, the
//! descenders hang under the baseline. So each character is classed by
//! where it reaches, neighbours of one class merge into one quad, and a
//! row of prose is a handful of quads that still reads as words.
//!
//! Called by `terminal_body.rs`'s bars path. Pure; the tests are the
//! contract. `chrome.rs` owns the heights.
use infiniterm_term::grid::SPACER;

/// Where a character reaches, from the baseline. `Low` is the x-height
/// body; `Tall` reaches the ascender line (a capital, a digit, `b d f h k
/// l t`, a bracket); `Deep` is x-height with a descender (`g j p q y`);
/// `Mark` is punctuation on the baseline, short.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    Low,
    Tall,
    Deep,
    Mark,
}

/// One quad: the column it starts at, how many cells, and its reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stroke {
    pub col: usize,
    pub len: usize,
    pub reach: Reach,
}

pub fn reach_of(c: char) -> Option<Reach> {
    if c.is_whitespace() || c == SPACER {
        return None;
    }
    Some(match c {
        'b' | 'd' | 'f' | 'h' | 'k' | 'l' | 't' | 'i' => Reach::Tall,
        'g' | 'j' | 'p' | 'q' | 'y' => Reach::Deep,
        c if c.is_lowercase() => Reach::Low,
        c if c.is_uppercase() || c.is_numeric() => Reach::Tall,
        '(' | ')' | '[' | ']' | '{' | '}' | '|' | '/' | '\\' | '#' | '$' | '@' | '&' | '%'
        | '!' | '?' => Reach::Tall,
        '.' | ',' | ':' | ';' | '\'' | '`' | '-' | '_' | '=' | '~' | '*' | '+' | '<' | '>'
        | '"' | '^' => Reach::Mark,
        // Box drawing, icons, anything else: a full-height mark.
        _ => Reach::Tall,
    })
}

/// The strokes of one row's characters, `col` counting every cell
/// (a wide character's `SPACER` takes a column and joins no stroke).
pub fn strokes(chars: impl Iterator<Item = char>) -> Vec<Stroke> {
    let mut out: Vec<Stroke> = Vec::new();
    for (col, c) in chars.enumerate() {
        let Some(reach) = reach_of(c) else { continue };
        match out.last_mut() {
            Some(last) if last.reach == reach && last.col + last.len == col => last.len += 1,
            _ => out.push(Stroke { col, len: 1, reach }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_is_its_silhouette() {
        // "hello": h tall, e l l o -> e low, ll tall, o low.
        let s = strokes("hello".chars());
        assert_eq!(
            s,
            vec![
                Stroke {
                    col: 0,
                    len: 1,
                    reach: Reach::Tall
                },
                Stroke {
                    col: 1,
                    len: 1,
                    reach: Reach::Low
                },
                Stroke {
                    col: 2,
                    len: 2,
                    reach: Reach::Tall
                },
                Stroke {
                    col: 4,
                    len: 1,
                    reach: Reach::Low
                },
            ]
        );
    }

    #[test]
    fn spaces_break_strokes_and_take_columns() {
        let s = strokes("no go".chars());
        assert_eq!(
            s[0],
            Stroke {
                col: 0,
                len: 2,
                reach: Reach::Low
            }
        );
        assert_eq!(
            s[1],
            Stroke {
                col: 3,
                len: 1,
                reach: Reach::Deep
            }
        );
        assert_eq!(
            s[2],
            Stroke {
                col: 4,
                len: 1,
                reach: Reach::Low
            }
        );
    }

    #[test]
    fn classes() {
        assert_eq!(reach_of('A'), Some(Reach::Tall));
        assert_eq!(reach_of('7'), Some(Reach::Tall));
        assert_eq!(reach_of('j'), Some(Reach::Deep));
        assert_eq!(reach_of('i'), Some(Reach::Tall));
        assert_eq!(reach_of('.'), Some(Reach::Mark));
        assert_eq!(reach_of(' '), None);
        assert_eq!(reach_of(SPACER), None);
        assert_eq!(reach_of('─'), Some(Reach::Tall));
    }
}
