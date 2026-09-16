//! Reading tmux control mode, line by line. Pure: a line of text in, a
//! `Notice` out, so the protocol can be tested without a tmux anywhere.
//!
//! tmux in control mode draws nothing. It says "pane %0 produced these
//! bytes" and our own emulator renders them, which is why scrollback,
//! selection and the mouse stay what they already are. See
//! spikes/tmux/NOTES.md for what was measured against tmux 3.7c.
//!
//! Called by `backend/tmux.rs`, which owns the process and the socket.
//! Related: backend/mod.rs for the `PaneEvent` this ends up as.

/// One line of control mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// A pane's output, already unescaped.
    Output {
        pane: String,
        bytes: Vec<u8>,
    },
    /// A command's reply begins; the number is tmux's command number.
    Begin(u64),
    /// A command's reply ends. `ok` is false for `%error`.
    End {
        command: u64,
        ok: bool,
    },
    WindowAdd(String),
    WindowClose(String),
    WindowRenamed {
        window: String,
        name: String,
    },
    /// tmux stopped sending this pane's output, because we asked.
    Paused(String),
    Continued(String),
    /// The pane's program exited.
    PaneDead(String),
    /// tmux is going away. The reason is whatever it gave, if any.
    Exit(Option<String>),
    /// A line inside a `%begin`/`%end` block: a command's answer.
    Reply(String),
    /// A notice we do not act on. Kept as a variant rather than dropped so
    /// a log can show what arrived.
    Other(String),
}

/// Whether the parser is inside a command's reply block, which changes what
/// a plain line means.
#[derive(Debug, Default)]
pub struct Reader {
    in_block: bool,
}

impl Reader {
    pub fn new() -> Reader {
        Reader::default()
    }

    /// One line, without its newline.
    pub fn line(&mut self, line: &str) -> Notice {
        if let Some(rest) = line.strip_prefix('%') {
            let (word, args) = split_first(rest);
            match word {
                "output" => {
                    let (pane, data) = split_first(args);
                    return Notice::Output {
                        pane: pane.to_string(),
                        bytes: unescape(data),
                    };
                }
                "begin" => {
                    self.in_block = true;
                    return Notice::Begin(field(args, 1));
                }
                "end" | "error" => {
                    self.in_block = false;
                    return Notice::End {
                        command: field(args, 1),
                        ok: word == "end",
                    };
                }
                "window-add" | "unlinked-window-add" => return Notice::WindowAdd(args.to_string()),
                "window-close" | "unlinked-window-close" => {
                    return Notice::WindowClose(args.to_string())
                }
                "window-renamed" => {
                    let (window, name) = split_first(args);
                    return Notice::WindowRenamed {
                        window: window.to_string(),
                        name: name.to_string(),
                    };
                }
                "pause" => return Notice::Paused(args.to_string()),
                "continue" => return Notice::Continued(args.to_string()),
                // The pane's program finished. tmux keeps the pane only if
                // remain-on-exit is set; either way the card is done.
                "pane-mode-changed" => return Notice::Other(line.to_string()),
                "exit" => {
                    return Notice::Exit(Some(args.to_string()).filter(|a| !a.is_empty()));
                }
                _ => return Notice::Other(line.to_string()),
            }
        }
        if self.in_block {
            Notice::Reply(line.to_string())
        } else {
            Notice::Other(line.to_string())
        }
    }

    pub fn in_block(&self) -> bool {
        self.in_block
    }
}

fn split_first(s: &str) -> (&str, &str) {
    match s.split_once(' ') {
        Some((head, rest)) => (head, rest),
        None => (s, ""),
    }
}

/// The nth space-separated field as a number, 0 when it is not one.
fn field(s: &str, n: usize) -> u64 {
    s.split(' ')
        .nth(n)
        .and_then(|f| f.parse().ok())
        .unwrap_or(0)
}

/// tmux escapes anything unprintable as a THREE-DIGIT OCTAL `\ooo`, and a
/// backslash as `\\`. Everything else is itself.
///
/// Bytes, not characters: the pane's output is arbitrary and goes to a VT
/// parser, so a partial UTF-8 sequence split across two `%output` lines has
/// to survive as the bytes it is.
pub fn unescape(text: &str) -> Vec<u8> {
    let src = text.as_bytes();
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        if src[i] != b'\\' {
            out.push(src[i]);
            i += 1;
            continue;
        }
        // A trailing backslash is not an escape; keep it rather than lose it.
        let Some(&next) = src.get(i + 1) else {
            out.push(b'\\');
            break;
        };
        if next == b'\\' {
            out.push(b'\\');
            i += 2;
            continue;
        }
        let octal = src
            .get(i + 1..i + 4)
            .filter(|d| d.iter().all(|c| (b'0'..=b'7').contains(c)));
        match octal {
            Some(digits) => {
                let value = digits
                    .iter()
                    .fold(0u32, |acc, d| acc * 8 + u32::from(d - b'0'));
                out.push(value as u8);
                i += 4;
            }
            // Not an escape tmux wrote; pass it through unharmed.
            None => {
                out.push(b'\\');
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(lines: &[&str]) -> Vec<Notice> {
        let mut r = Reader::new();
        lines.iter().map(|l| r.line(l)).collect()
    }

    #[test]
    fn output_carries_the_pane_and_the_unescaped_bytes() {
        let got = read(&[r"%output %0 hello\015\012"]);
        assert_eq!(
            got[0],
            Notice::Output {
                pane: "%0".into(),
                bytes: b"hello\r\n".to_vec(),
            }
        );
    }

    // The escape is three-digit octal, which is how an ESC arrives.
    #[test]
    fn octal_escapes_become_the_bytes_they_name() {
        assert_eq!(unescape(r"\033[m"), b"\x1b[m".to_vec());
        assert_eq!(unescape(r"a\\b"), b"a\\b".to_vec());
        assert_eq!(unescape("plain"), b"plain".to_vec());
        assert_eq!(unescape(""), Vec::<u8>::new());
    }

    // Arbitrary bytes, not characters: a UTF-8 sequence split across two
    // lines must survive as bytes for the VT parser to join.
    #[test]
    fn a_split_utf8_sequence_survives_as_bytes() {
        let first = unescape(r"\342\234"); // the first two bytes of ✔
        let second = unescape(r"\224");
        let mut joined = first.clone();
        joined.extend(second);
        assert_eq!(String::from_utf8(joined).unwrap(), "✔");
        assert_eq!(first.len(), 2, "half a character is still two bytes");
    }

    // Anything that is not an escape tmux wrote passes through: a filename
    // with a backslash in it must not be mangled.
    #[test]
    fn a_backslash_that_is_not_an_escape_is_kept() {
        assert_eq!(unescape(r"\9"), b"\\9".to_vec());
        assert_eq!(unescape(r"end\"), b"end\\".to_vec());
        assert_eq!(unescape(r"\01"), b"\\01".to_vec(), "too short to be octal");
    }

    #[test]
    fn a_command_reply_is_framed_and_its_lines_are_replies() {
        let got = read(&[
            "%begin 1789596465 339 1",
            "@0 80x24",
            "%end 1789596465 339 1",
            "@1 60x20",
        ]);
        assert_eq!(got[0], Notice::Begin(339));
        assert_eq!(got[1], Notice::Reply("@0 80x24".into()));
        assert_eq!(
            got[2],
            Notice::End {
                command: 339,
                ok: true
            }
        );
        // Outside a block the same text is not an answer to anything.
        assert_eq!(got[3], Notice::Other("@1 60x20".into()));
    }

    #[test]
    fn an_error_closes_the_block_and_says_so() {
        let got = read(&["%begin 1 7 1", "no such window", "%error 1 7 1"]);
        assert_eq!(
            got[2],
            Notice::End {
                command: 7,
                ok: false
            }
        );
    }

    #[test]
    fn the_notices_a_card_cares_about() {
        let got = read(&[
            "%window-add @3",
            "%window-close @3",
            "%window-renamed @0 zsh",
            "%pause %2",
            "%continue %2",
            "%exit server exited",
            "%exit",
        ]);
        assert_eq!(got[0], Notice::WindowAdd("@3".into()));
        assert_eq!(got[1], Notice::WindowClose("@3".into()));
        assert_eq!(
            got[2],
            Notice::WindowRenamed {
                window: "@0".into(),
                name: "zsh".into()
            }
        );
        assert_eq!(got[3], Notice::Paused("%2".into()));
        assert_eq!(got[4], Notice::Continued("%2".into()));
        assert_eq!(got[5], Notice::Exit(Some("server exited".into())));
        assert_eq!(got[6], Notice::Exit(None));
    }

    // Recorded from tmux 3.7c during the spike: the shape a real session
    // starts with, which is the one that has to parse.
    #[test]
    fn a_real_sessions_opening_lines_parse() {
        let mut r = Reader::new();
        let opening = [
            "%begin 1789596464 333 0",
            "%end 1789596464 333 0",
            "%window-add @0",
            "%sessions-changed",
            "%session-changed $0 spike",
            r"%output %0 \033[1m\033[32mekinertac\033[m",
        ];
        let got: Vec<Notice> = opening.iter().map(|l| r.line(l)).collect();
        assert!(!r.in_block(), "the block closed");
        assert_eq!(got[2], Notice::WindowAdd("@0".into()));
        assert!(matches!(got[3], Notice::Other(_)));
        assert!(matches!(&got[5], Notice::Output { pane, .. } if pane == "%0"));
    }
}
