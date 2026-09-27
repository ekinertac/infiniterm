//! A key as the bytes a shell expects. The full table the spike's subset
//! stood in for, with the reference's two rules on top of xterm's: Alt+Arrow
//! is a word (`ESC b` / `ESC f`) and Cmd+Arrow is the line (Ctrl+A / Ctrl+E),
//! since xterm's own modifier sequences are not bound by any shell's line
//! editor and the keys did nothing. Cmd+anything else never reaches here:
//! the app owns Cmd.
//!
//! Toolkit-free: the ui turns its keystroke into a `Key`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Key<'a> {
    /// gpui's name: a character, or `enter`, `escape`, `left`, `f1`, ...
    pub name: &'a str,
    /// The text the key produces, when it does.
    pub text: Option<&'a str>,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
}

/// xterm's modifier parameter: 1 + (shift 1, alt 2, ctrl 4).
fn modifier(k: &Key) -> u8 {
    1 + (k.shift as u8) + (k.alt as u8) * 2 + (k.ctrl as u8) * 4
}

/// `app_cursor` is DECCKM: arrows and Home/End send SS3 instead of CSI.
/// Legacy keys only; `encode_with` takes the kitty flag as well.
pub fn encode(k: &Key, app_cursor: bool) -> Option<Vec<u8>> {
    encode_with(k, app_cursor, false)
}

/// `kitty` is whether the program in the pane speaks the kitty keyboard
/// protocol, per `Grid::kitty_keys`. While it does, Enter with Shift or
/// Ctrl is `CSI 13 ; modifier u`, which is the only spelling that differs
/// from Enter at all: legacy xterm sends CR for both, so a program cannot
/// tell "send" from "new line". This is exactly what kitty itself sends in
/// its legacy mode, for the keys that have no legacy encoding, and it is
/// what Claude Code means by Shift+Enter being "native" in kitty, Ghostty,
/// WezTerm and iTerm2. Nothing else changes: Alt+Enter keeps `ESC CR`,
/// plain Enter keeps CR, Esc keeps ESC. Measured against Claude Code
/// 2.1.274: it accepts CSI 13;2u, rejects CSI 27 u for Esc, and never
/// pushes a flag, so this cannot be gated on a push.
pub fn encode_with(k: &Key, app_cursor: bool, kitty: bool) -> Option<Vec<u8>> {
    let m = modifier(k);
    let plain = m == 1;
    if kitty && !k.cmd && !k.alt && k.name == "enter" && (k.shift || k.ctrl) {
        return Some(format!("\x1b[13;{m}u").into_bytes());
    }
    // The reference's text-field arrows.
    if !k.ctrl && !k.shift {
        match (k.alt, k.cmd, k.name) {
            (true, false, "left") => return Some(b"\x1bb".to_vec()),
            (true, false, "right") => return Some(b"\x1bf".to_vec()),
            // The forward twin of Alt+Backspace's `ESC DEL`. xterm's own
            // `CSI 3;3~` is bound by no line editor: zsh, Claude Code and
            // Pi all showed it landing on the line as the text `3~`.
            // `ESC d` is readline's delete-word-forward and all three bind
            // it.
            (true, false, "delete") => return Some(b"\x1bd".to_vec()),
            (false, true, "left") => return Some(vec![0x01]),
            (false, true, "right") => return Some(vec![0x05]),
            // Cmd+Backspace / Cmd+Delete delete to the line's start / end,
            // a Mac field's keys, as iTerm2 and Ghostty send them: Ctrl+U
            // and Ctrl+K. Our zsh integration binds Ctrl+U to readline's
            // meaning (to the start), not zsh's (the whole line).
            (false, true, "backspace") => return Some(vec![0x15]),
            (false, true, "delete") => return Some(vec![0x0b]),
            _ => {}
        }
    }
    if k.cmd {
        return None;
    }
    let csi = |code: &str, tilde: bool| -> Vec<u8> {
        if plain {
            if tilde {
                format!("\x1b[{code}~").into_bytes()
            } else if app_cursor {
                format!("\x1bO{code}").into_bytes()
            } else {
                format!("\x1b[{code}").into_bytes()
            }
        } else if tilde {
            format!("\x1b[{code};{m}~").into_bytes()
        } else {
            format!("\x1b[1;{m}{code}").into_bytes()
        }
    };
    let seq = match k.name {
        "up" => csi("A", false),
        "down" => csi("B", false),
        "right" => csi("C", false),
        "left" => csi("D", false),
        "home" => csi("H", false),
        "end" => csi("F", false),
        "insert" => csi("2", true),
        "delete" => csi("3", true),
        "pageup" => csi("5", true),
        "pagedown" => csi("6", true),
        "f1" => {
            if plain {
                b"\x1bOP".to_vec()
            } else {
                csi("P", false)
            }
        }
        "f2" => {
            if plain {
                b"\x1bOQ".to_vec()
            } else {
                csi("Q", false)
            }
        }
        "f3" => {
            if plain {
                b"\x1bOR".to_vec()
            } else {
                csi("R", false)
            }
        }
        "f4" => {
            if plain {
                b"\x1bOS".to_vec()
            } else {
                csi("S", false)
            }
        }
        "f5" => csi("15", true),
        "f6" => csi("17", true),
        "f7" => csi("18", true),
        "f8" => csi("19", true),
        "f9" => csi("20", true),
        "f10" => csi("21", true),
        "f11" => csi("23", true),
        "f12" => csi("24", true),
        "enter" => {
            if k.alt {
                b"\x1b\r".to_vec()
            } else {
                b"\r".to_vec()
            }
        }
        "tab" => {
            if k.shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            }
        }
        "escape" => b"\x1b".to_vec(),
        "backspace" => {
            if k.alt {
                b"\x1b\x7f".to_vec()
            } else if k.ctrl {
                vec![0x08]
            } else {
                vec![0x7f]
            }
        }
        "space" => {
            if k.ctrl {
                vec![0]
            } else if k.alt {
                b"\x1b ".to_vec()
            } else {
                b" ".to_vec()
            }
        }
        _ => {
            if k.ctrl {
                // Ctrl+letter is the control byte; the punctuation ones too.
                let c = k.name.chars().next()?;
                let byte = match c.to_ascii_lowercase() {
                    c @ 'a'..='z' => c as u8 - b'a' + 1,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '-' | '7' => 0x1f,
                    '2' | '@' => 0,
                    '8' => 0x7f,
                    _ => return None,
                };
                if k.alt {
                    vec![0x1b, byte]
                } else {
                    vec![byte]
                }
            } else {
                let text = k.text?;
                if text.is_empty() || text.chars().any(char::is_control) {
                    return None;
                }
                // Alt is Meta, an ESC prefix, which is what bash and zsh
                // expect. EXCEPT where the layout makes Option a character
                // key: on Turkish Q, Option+S is `ş` and Option+I is `ı`,
                // and macOS hands us that character rather than `s`. An ESC
                // in front of it is a sequence nothing binds, and it
                // reached the line as `<ffffffff>`. The character IS the
                // key there, so it goes alone. A layout where Option
                // changes nothing (US: Option+S is still `s`) keeps Meta.
                let composed = k.alt && text != k.name;
                let mut out = Vec::with_capacity(text.len() + 1);
                if k.alt && !composed {
                    out.push(0x1b);
                }
                out.extend_from_slice(text.as_bytes());
                out
            }
        }
    };
    Some(seq)
}

/// A paste, bracketed when the program asked for it so a multi-line paste
/// is not run line by line.
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.as_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Turkish Q: Option+S is `ş`, a character key, not Meta+s. macOS
    // resolves it through the layout and hands us the composed character;
    // an ESC in front of it is a sequence no shell binds, and it showed on
    // the line as `<ffffffff>`.
    #[test]
    fn option_types_the_character_a_layout_composes() {
        let mut k = key("s", Some("ş"));
        k.alt = true;
        assert_eq!(encode(&k, false), Some("ş".as_bytes().to_vec()));
        let mut dotless = key("i", Some("ı"));
        dotless.alt = true;
        assert_eq!(encode(&dotless, false), Some("ı".as_bytes().to_vec()));
    }

    // US layout: Option+S is still `s`, so Alt keeps meaning Meta and the
    // ESC prefix stays. Losing this would break Alt+letter in every shell.
    #[test]
    fn option_is_still_meta_where_the_layout_composes_nothing() {
        let mut k = key("s", Some("s"));
        k.alt = true;
        assert_eq!(encode(&k, false), Some(b"\x1bs".to_vec()));
    }

    // A Mac field's delete-to-the-ends, and the Shift moves our zsh
    // integration selects with (shell/infiniterm.zsh binds these bytes).
    #[test]
    fn line_editing_keys_send_what_the_zsh_integration_binds() {
        let cmd = |name| Key {
            cmd: true,
            ..key(name, None)
        };
        assert_eq!(encode(&cmd("backspace"), false), Some(vec![0x15]));
        assert_eq!(encode(&cmd("delete"), false), Some(vec![0x0b]));
        let shift = |name| Key {
            shift: true,
            ..key(name, None)
        };
        assert_eq!(encode(&shift("left"), false), Some(b"\x1b[1;2D".to_vec()));
        assert_eq!(encode(&shift("home"), false), Some(b"\x1b[1;2H".to_vec()));
        assert_eq!(encode(&shift("end"), false), Some(b"\x1b[1;2F".to_vec()));
        let word = Key {
            shift: true,
            alt: true,
            ..key("right", None)
        };
        assert_eq!(encode(&word, false), Some(b"\x1b[1;4C".to_vec()));
    }

    // Alt+Backspace deletes the word behind; Alt+Delete must delete the
    // word ahead. xterm's CSI 3;3~ is bound by nothing and arrived as the
    // text `3~`, measured in zsh, Claude Code and Pi.
    #[test]
    fn alt_delete_is_delete_word_forward() {
        let mut k = key("delete", None);
        k.alt = true;
        assert_eq!(encode(&k, false), Some(b"\x1bd".to_vec()));
        let mut back = key("backspace", None);
        back.alt = true;
        assert_eq!(encode(&back, false), Some(b"\x1b\x7f".to_vec()));
    }

    // The one key the kitty protocol changes for us. Shift+Enter is the
    // whole reason: legacy xterm sends CR for it and for Enter, and Claude
    // Code then sent the prompt where a new line was wanted.
    #[test]
    fn a_kitty_program_gets_csi_u_for_shift_or_ctrl_enter_and_legacy_for_the_rest() {
        let mut shift = key("enter", Some("\n"));
        shift.shift = true;
        assert_eq!(
            encode_with(&shift, false, true),
            Some(b"\x1b[13;2u".to_vec())
        );
        let mut ctrl = key("enter", Some("\n"));
        ctrl.ctrl = true;
        assert_eq!(
            encode_with(&ctrl, false, true),
            Some(b"\x1b[13;5u".to_vec())
        );
        // These have legacy spellings and kitty keeps them; so do we.
        let mut alt = key("enter", Some("\n"));
        alt.alt = true;
        assert_eq!(encode_with(&alt, false, true), Some(b"\x1b\r".to_vec()));
        assert_eq!(
            encode_with(&key("enter", Some("\n")), false, true),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            encode_with(&key("escape", None), false, true),
            Some(b"\x1b".to_vec())
        );
        let mut stab = key("tab", Some("\t"));
        stab.shift = true;
        assert_eq!(encode_with(&stab, false, true), Some(b"\x1b[Z".to_vec()));
    }

    // A shell never asks, and must keep the bytes it has always had. With
    // CSI u forced on, zsh was measured turning Shift+Enter into the text
    // `;2u` on the command line instead of running it.
    #[test]
    fn a_shell_still_gets_cr_for_shift_enter() {
        let mut shift = key("enter", Some("\n"));
        shift.shift = true;
        assert_eq!(encode_with(&shift, false, false), Some(b"\r".to_vec()));
        assert_eq!(encode(&shift, false), Some(b"\r".to_vec()));
    }

    fn key<'a>(name: &'a str, text: Option<&'a str>) -> Key<'a> {
        Key {
            name,
            text,
            ..Default::default()
        }
    }

    #[test]
    fn printable_keys_are_their_text_and_enter_is_cr() {
        assert_eq!(encode(&key("a", Some("a")), false), Some(b"a".to_vec()));
        assert_eq!(
            encode(&key("enter", Some("\n")), false),
            Some(b"\r".to_vec())
        );
        assert_eq!(encode(&key("backspace", None), false), Some(vec![0x7f]));
    }

    #[test]
    fn control_letters_are_control_bytes() {
        let ctrl = |n| Key {
            ctrl: true,
            ..key(n, None)
        };
        assert_eq!(encode(&ctrl("c"), false), Some(vec![3]));
        assert_eq!(encode(&ctrl("d"), false), Some(vec![4]));
        assert_eq!(encode(&ctrl("["), false), Some(vec![0x1b]));
        assert_eq!(
            encode(
                &Key {
                    ctrl: true,
                    ..key("space", Some(" "))
                },
                false
            ),
            Some(vec![0])
        );
    }

    // The reference's rule: word and line movement as the line editor
    // already understands it, and Cmd never otherwise reaches the shell.
    #[test]
    fn alt_and_cmd_arrows_are_word_and_line_movement() {
        assert_eq!(
            encode(
                &Key {
                    alt: true,
                    ..key("left", None)
                },
                false
            ),
            Some(b"\x1bb".to_vec())
        );
        assert_eq!(
            encode(
                &Key {
                    alt: true,
                    ..key("right", None)
                },
                false
            ),
            Some(b"\x1bf".to_vec())
        );
        assert_eq!(
            encode(
                &Key {
                    cmd: true,
                    ..key("left", None)
                },
                false
            ),
            Some(vec![1])
        );
        assert_eq!(
            encode(
                &Key {
                    cmd: true,
                    ..key("right", None)
                },
                false
            ),
            Some(vec![5])
        );
        assert_eq!(
            encode(
                &Key {
                    cmd: true,
                    ..key("k", None)
                },
                false
            ),
            None
        );
    }

    #[test]
    fn arrows_follow_deckm_and_carry_modifiers() {
        assert_eq!(encode(&key("up", None), false), Some(b"\x1b[A".to_vec()));
        assert_eq!(encode(&key("up", None), true), Some(b"\x1bOA".to_vec()));
        assert_eq!(
            encode(
                &Key {
                    shift: true,
                    ..key("up", None)
                },
                false
            ),
            Some(b"\x1b[1;2A".to_vec())
        );
        assert_eq!(
            encode(
                &Key {
                    ctrl: true,
                    ..key("right", None)
                },
                true
            ),
            Some(b"\x1b[1;5C".to_vec())
        );
        assert_eq!(
            encode(&key("delete", None), false),
            Some(b"\x1b[3~".to_vec())
        );
        assert_eq!(
            encode(
                &Key {
                    shift: true,
                    ..key("tab", None)
                },
                false
            ),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(encode(&key("f5", None), false), Some(b"\x1b[15~".to_vec()));
        assert_eq!(encode(&key("f1", None), false), Some(b"\x1bOP".to_vec()));
    }

    #[test]
    fn alt_prefixes_a_character_with_escape() {
        assert_eq!(
            encode(
                &Key {
                    alt: true,
                    ..key("x", Some("x"))
                },
                false
            ),
            Some(b"\x1bx".to_vec())
        );
        assert_eq!(
            encode(
                &Key {
                    alt: true,
                    ..key("backspace", None)
                },
                false
            ),
            Some(b"\x1b\x7f".to_vec())
        );
    }

    #[test]
    fn a_paste_is_bracketed_only_when_asked() {
        assert_eq!(paste("ls\n", false), b"ls\n".to_vec());
        assert_eq!(paste("ls\n", true), b"\x1b[200~ls\n\x1b[201~".to_vec());
    }
}
