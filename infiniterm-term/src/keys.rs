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
pub fn encode(k: &Key, app_cursor: bool) -> Option<Vec<u8>> {
    let m = modifier(k);
    let plain = m == 1;
    // The reference's text-field arrows.
    if !k.ctrl && !k.shift {
        match (k.alt, k.cmd, k.name) {
            (true, false, "left") => return Some(b"\x1bb".to_vec()),
            (true, false, "right") => return Some(b"\x1bf".to_vec()),
            (false, true, "left") => return Some(vec![0x01]),
            (false, true, "right") => return Some(vec![0x05]),
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
                let mut out = Vec::with_capacity(text.len() + 1);
                if k.alt {
                    // Meta as ESC prefix, what bash and zsh expect by default;
                    // the character is the unmodified one on macOS since gpui
                    // gives it to us for the physical key.
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
