//! Mouse reporting for programs that ask for it (a TUI): the click, the
//! release, the wheel, in xterm's SGR encoding when the program enabled it
//! and the old X10 bytes otherwise. Cells are 0-based in, 1-based out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

fn code(button: MouseButton, mods: Mods, motion: bool) -> u8 {
    let mut c = match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    };
    if mods.shift {
        c += 4;
    }
    if mods.alt {
        c += 8;
    }
    if mods.ctrl {
        c += 16;
    }
    if motion {
        c += 32;
    }
    c
}

/// A press (or a wheel notch, which has no release).
pub fn press(button: MouseButton, col: usize, row: usize, mods: Mods, sgr: bool) -> Vec<u8> {
    report(code(button, mods, false), col, row, sgr, false)
}

pub fn release(button: MouseButton, col: usize, row: usize, mods: Mods, sgr: bool) -> Vec<u8> {
    if sgr {
        report(code(button, mods, false), col, row, true, true)
    } else {
        // X10 has no button on release: 3 means "released".
        report(3, col, row, false, false)
    }
}

/// A drag: the button held while moving.
pub fn motion(button: MouseButton, col: usize, row: usize, mods: Mods, sgr: bool) -> Vec<u8> {
    report(code(button, mods, true), col, row, sgr, false)
}

fn report(code: u8, col: usize, row: usize, sgr: bool, released: bool) -> Vec<u8> {
    if sgr {
        format!(
            "\x1b[<{code};{};{}{}",
            col + 1,
            row + 1,
            if released { 'm' } else { 'M' }
        )
        .into_bytes()
    } else {
        // X10 caps at 223 columns; a bigger terminal simply reports the edge.
        let clamp = |v: usize| (v.min(222) + 33) as u8;
        vec![0x1b, b'[', b'M', 32 + code, clamp(col), clamp(row)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_reports_press_release_and_wheel() {
        let m = Mods::default();
        assert_eq!(
            press(MouseButton::Left, 0, 0, m, true),
            b"\x1b[<0;1;1M".to_vec()
        );
        assert_eq!(
            release(MouseButton::Left, 4, 2, m, true),
            b"\x1b[<0;5;3m".to_vec()
        );
        assert_eq!(
            press(MouseButton::WheelUp, 4, 2, m, true),
            b"\x1b[<64;5;3M".to_vec()
        );
        assert_eq!(
            press(MouseButton::Right, 0, 0, Mods { ctrl: true, ..m }, true),
            b"\x1b[<18;1;1M".to_vec()
        );
    }

    #[test]
    fn x10_is_the_old_bytes() {
        let m = Mods::default();
        assert_eq!(
            press(MouseButton::Left, 0, 0, m, false),
            vec![0x1b, b'[', b'M', 32, 33, 33]
        );
        assert_eq!(
            release(MouseButton::Left, 0, 0, m, false),
            vec![0x1b, b'[', b'M', 35, 33, 33]
        );
    }
}
