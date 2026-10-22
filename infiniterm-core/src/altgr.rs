//! When Ctrl+Alt is AltGr and not a chord: the pure rule.
//!
//! Responsibility: decide whether a key-down that looks like Ctrl+Alt is the
//! user typing with AltGr, so the keyboard code can drop that Ctrl and Alt
//! before a chord is looked up or a key is sent to a terminal. Nothing here
//! talks to Windows: the caller says how a layout types characters and which
//! keys are down.
//!
//! Why it exists: on a layout with an AltGr key (Turkish Q, German, French,
//! Spanish, US-International and others) Windows reports AltGr as Right Alt
//! plus a SYNTHETIC Left Ctrl. Turkish Q types `{ [ ] } \ @ € ~` that way, so
//! without this every such character would look like a Ctrl+Alt chord and
//! could fire a binding, or reach a terminal as control codes. A real
//! Ctrl+Alt chord must still count, and on a layout without an AltGr (US)
//! Right Alt is a plain Alt and injects no Ctrl, so Left Ctrl plus Right Alt
//! there is a chord the user meant.
//!
//! Where it fits: `infiniterm-ui/src/keycode.rs` (the Windows keyboard hook
//! calls `altgr_in_effect` on every key-down and keeps the answer),
//! `input.rs::platform_keystroke_in` (drops Ctrl and Alt when it was AltGr).
//! Written for #537's command-key design (docs/agent/command-key.md), whose
//! Ctrl+Alt rows are safe on Windows only with this.
//!
//! Constraint: the layout test is a probe of how the layout types characters
//! (`VkKeyScanExW` on Windows), which misses an AltGr layout that gives AltGr
//! only to characters outside `probe_chars`; those layouts keep today's
//! behaviour (Ctrl+Alt counts) rather than losing real chords.

/// The shift state a layout reports for a character when it needs Ctrl and
/// Alt together: `VkKeyScanExW`'s high byte, where Shift is 1, Ctrl 2, Alt 4.
pub const CTRL_ALT: u8 = 2 | 4;

/// The characters to ask a layout about: printable ASCII, the Latin-1
/// letters and symbols (where AltGr puts the accented ones on US-International
/// and the Nordic layouts), and the euro sign. An AltGr layout types at least
/// one of them with AltGr; a layout without an AltGr key types none of them
/// with Ctrl+Alt.
pub fn probe_chars() -> impl Iterator<Item = char> {
    (' '..='~')
        .chain('\u{00A1}'..='\u{00FF}')
        .chain(std::iter::once('\u{20AC}'))
}

/// Whether a layout has an AltGr, given how it types each character:
/// `shift_state_of(c)` is the shift state it reports for `c` (`None` when it
/// cannot type `c`). True when some probe character needs Ctrl and Alt both.
pub fn layout_has_altgr(shift_state_of: impl Fn(char) -> Option<u8>) -> bool {
    probe_chars().any(|c| shift_state_of(c).is_some_and(|s| s & CTRL_ALT == CTRL_ALT))
}

/// Whether the keys down right now are AltGr: the layout has one, Right Alt is
/// down, and the Left Ctrl that goes with it is the injected one. A Right Ctrl
/// down as well means the user is holding a real Ctrl, so it is a chord.
pub fn altgr_in_effect(
    layout_has_altgr: bool,
    right_alt: bool,
    left_ctrl: bool,
    right_ctrl: bool,
) -> bool {
    layout_has_altgr && right_alt && left_ctrl && !right_ctrl
}

#[cfg(test)]
mod tests {
    use super::*;

    // Turkish Q types @ with AltGr+Q; shift state 6 is Ctrl+Alt.
    fn turkish_q(c: char) -> Option<u8> {
        match c {
            '@' | '{' | '[' | ']' | '}' | '\\' | '~' | '\u{20AC}' => Some(CTRL_ALT),
            'a'..='z' | '0'..='9' => Some(0),
            'A'..='Z' => Some(1),
            _ => None,
        }
    }

    // US: shifted symbols use Shift (1) and nothing needs Ctrl+Alt.
    fn us(c: char) -> Option<u8> {
        match c {
            '@' | '{' | '}' | '~' | 'A'..='Z' => Some(1),
            ' '..='~' => Some(0),
            _ => None,
        }
    }

    // US-International: AltGr gives accented letters, not ASCII symbols.
    fn us_international(c: char) -> Option<u8> {
        match c {
            '\u{00E1}' | '\u{00E9}' | '\u{00F6}' | '\u{00F1}' => Some(CTRL_ALT),
            _ => us(c),
        }
    }

    #[test]
    fn a_layout_with_an_altgr_is_told_from_one_without() {
        assert!(layout_has_altgr(turkish_q));
        assert!(layout_has_altgr(us_international));
        assert!(!layout_has_altgr(us));
        assert!(
            !layout_has_altgr(|_| None),
            "a layout that types nothing has none"
        );
    }

    #[test]
    fn a_layout_is_judged_on_ctrl_and_alt_together_not_either_alone() {
        // Alt alone (4) or Ctrl alone (2) is not AltGr.
        assert!(!layout_has_altgr(|c| (c == '@').then_some(4)));
        assert!(!layout_has_altgr(|c| (c == '@').then_some(2)));
        assert!(
            layout_has_altgr(|c| (c == '@').then_some(2 | 4 | 1)),
            "AltGr+Shift too"
        );
    }

    #[test]
    fn the_probe_covers_ascii_latin_one_and_the_euro() {
        let chars: Vec<char> = probe_chars().collect();
        for c in ['a', '@', '~', '\u{00E9}', '\u{00FF}', '\u{20AC}'] {
            assert!(chars.contains(&c), "{c:?}");
        }
        assert!(!chars.contains(&'\u{0100}'));
    }

    #[test]
    fn altgr_is_right_alt_with_the_injected_left_ctrl_on_a_layout_that_has_one() {
        assert!(altgr_in_effect(true, true, true, false));
        // No AltGr on the layout: Right Alt is a plain Alt, the Ctrl is real.
        assert!(!altgr_in_effect(false, true, true, false));
        // Left Alt is never AltGr.
        assert!(!altgr_in_effect(true, false, true, false));
        // Ctrl with no Alt at all.
        assert!(!altgr_in_effect(true, true, false, false));
        // A real Right Ctrl is held too: a chord.
        assert!(!altgr_in_effect(true, true, true, true));
    }
}
