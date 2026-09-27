//! Visual mode's keys (Cmd+Shift+C on a terminal card, 2026-09-27): what a
//! key does to the visual cursor, from the same toolkit-free `Key` the
//! encoder takes. Mac keys and vim keys side by side, as Ekin chose: arrows,
//! Alt+Arrow words, Cmd+Arrow line ends and scrollback ends, Home/End, Page
//! Up/Down, Shift on any of them to select; and h j k l, w b e, 0 $, g G,
//! Ctrl+U / Ctrl+D, v / V / Ctrl+V.
//!
//! Called from terminal_body.rs `key` while `Grid::visual` is on; the grid
//! does the moving (grid.rs `visual_move`, `visual_select`). Copying (Cmd+C,
//! `y`) does NOT leave the mode and Escape is the only way out (Ekin: leave
//! deliberately). Every other key is swallowed: nothing reaches the program.
use crate::grid::{CursorMove, VisualSelect};
use crate::keys::Key;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualKey {
    /// Move, selecting when the bool is set (Shift held).
    Move(CursorMove, bool),
    Select(VisualSelect),
    Copy,
    Leave,
    /// Taken and dropped, so it cannot reach the shell.
    Swallow,
}

pub fn visual_key(k: &Key) -> VisualKey {
    use CursorMove::*;
    use VisualKey::*;
    let shift = k.shift;
    // Named keys first, with their modifiers: the Mac half.
    let named = match (k.cmd, k.alt, k.name) {
        (true, _, "c") => Some(Copy),
        (true, _, "left") => Some(Move(LineStart, shift)),
        (true, _, "right") => Some(Move(LineEnd, shift)),
        (true, _, "up") => Some(Move(Top, shift)),
        (true, _, "down") => Some(Move(Bottom, shift)),
        (_, true, "left") => Some(Move(WordLeft, shift)),
        (_, true, "right") => Some(Move(WordRight, shift)),
        (_, _, "left") => Some(Move(Left, shift)),
        (_, _, "right") => Some(Move(Right, shift)),
        (_, _, "up") => Some(Move(Up, shift)),
        (_, _, "down") => Some(Move(Down, shift)),
        (_, _, "home") => Some(Move(LineStart, shift)),
        (_, _, "end") => Some(Move(LineEnd, shift)),
        (_, _, "pageup") => Some(Move(HalfPageUp, shift)),
        (_, _, "pagedown") => Some(Move(HalfPageDown, shift)),
        (_, _, "escape") => Some(Leave),
        _ => None,
    };
    if let Some(action) = named {
        return action;
    }
    if k.cmd || k.alt {
        return Swallow;
    }
    if k.ctrl {
        return match k.name {
            "u" => Move(HalfPageUp, false),
            "d" => Move(HalfPageDown, false),
            "v" => Select(VisualSelect::Block),
            _ => Swallow,
        };
    }
    // The vim half, by the character typed, so `$` and `G` work on any
    // layout that types them.
    match k.text.unwrap_or("") {
        "h" => Move(Left, false),
        "j" => Move(Down, false),
        "k" => Move(Up, false),
        "l" => Move(Right, false),
        "w" => Move(WordRight, false),
        "b" => Move(WordLeft, false),
        "e" => Move(WordEnd, false),
        "0" => Move(LineStart, false),
        "$" => Move(LineEnd, false),
        "g" => Move(Top, false),
        "G" => Move(Bottom, false),
        "v" => Select(VisualSelect::Cells),
        "V" => Select(VisualSelect::Lines),
        "y" => Copy,
        _ => Swallow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CursorMove::*;
    use VisualKey::*;

    fn key<'a>(name: &'a str, text: Option<&'a str>) -> Key<'a> {
        Key {
            name,
            text,
            ..Key::default()
        }
    }

    #[test]
    fn mac_keys_move_and_shift_selects() {
        assert_eq!(visual_key(&key("left", None)), Move(Left, false));
        let shifted = Key {
            shift: true,
            ..key("left", None)
        };
        assert_eq!(visual_key(&shifted), Move(Left, true));
        let word = Key {
            alt: true,
            shift: true,
            ..key("right", None)
        };
        assert_eq!(visual_key(&word), Move(WordRight, true));
        let top = Key {
            cmd: true,
            ..key("up", None)
        };
        assert_eq!(visual_key(&top), Move(Top, false));
        assert_eq!(
            visual_key(&key("pagedown", None)),
            Move(HalfPageDown, false)
        );
    }

    #[test]
    fn vim_keys_move_select_and_copy() {
        assert_eq!(visual_key(&key("j", Some("j"))), Move(Down, false));
        assert_eq!(visual_key(&key("4", Some("$"))), Move(LineEnd, false));
        let big_g = Key {
            shift: true,
            ..key("g", Some("G"))
        };
        assert_eq!(visual_key(&big_g), Move(Bottom, false));
        assert_eq!(
            visual_key(&key("v", Some("V"))),
            Select(VisualSelect::Lines)
        );
        let block = Key {
            ctrl: true,
            ..key("v", None)
        };
        assert_eq!(visual_key(&block), Select(VisualSelect::Block));
        assert_eq!(visual_key(&key("y", Some("y"))), Copy);
    }

    // Copying stays in the mode; only Escape leaves; the rest is dropped.
    #[test]
    fn only_escape_leaves_and_nothing_else_gets_through() {
        let copy = Key {
            cmd: true,
            ..key("c", None)
        };
        assert_eq!(visual_key(&copy), Copy);
        assert_eq!(visual_key(&key("escape", None)), Leave);
        assert_eq!(visual_key(&key("q", Some("q"))), Swallow);
        assert_eq!(visual_key(&key("enter", None)), Swallow);
    }
}
