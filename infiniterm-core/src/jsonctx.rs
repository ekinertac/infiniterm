//! Where the caret is in a JSON or JSONC text: the key path to it, whether
//! it sits in a key or a value, and what has been typed so far. The input
//! of schema completion (`schema.rs`, `complete.rs`).
//!
//! A tolerant scanner, not a parser: the text is invalid most of the time
//! it is asked about (the caret is in the middle of a half-typed key), so it
//! only tracks brackets, strings, comments, `:` and `,`. Strings end at a
//! newline; comments (`//`, `/* */`) answer `None`.
//!
//! Positions are char indices, the buffer's unit.

/// One step of a path: an object key or an array position.
#[derive(Clone, Debug, PartialEq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

/// What the caret is about to write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Key,
    Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    /// For a key: the path to the object the key goes in. For a value: the
    /// path to the value itself (the object's key, or the array position).
    pub path: Vec<Seg>,
    pub slot: Slot,
    /// The caret is inside a string's quotes.
    pub in_string: bool,
    /// What is typed so far: the string's text up to the caret, or the bare
    /// word (`tr` of `true`) when there is no quote.
    pub prefix: String,
    /// How many chars before the caret a completion replaces.
    pub typed: usize,
    /// The keys already in the object a key is being written in, so they
    /// are not offered twice. The key being typed is not among them.
    pub siblings: Vec<String>,
}

struct Frame {
    obj: bool,
    keys: Vec<String>,
    cur_key: Option<String>,
    expect_key: bool,
    index: usize,
}

enum Mode {
    Normal,
    Str {
        is_key: bool,
        text: String,
        esc: bool,
    },
    Line,
    Block(usize),
}

struct Snap {
    path: Vec<Seg>,
    slot: Slot,
    in_string: bool,
    prefix: String,
    frame: usize,
    /// Index the key being typed will take in its frame's key list.
    typing_key_at: Option<usize>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '-' | '+')
}

/// The path to the frame at `stack[k]`: one step per ancestor.
fn path_to(stack: &[usize], arena: &[Frame], k: usize) -> Option<Vec<Seg>> {
    stack[..k]
        .iter()
        .map(|&f| {
            let f = &arena[f];
            if f.obj {
                f.cur_key.clone().map(Seg::Key)
            } else {
                Some(Seg::Index(f.index))
            }
        })
        .collect()
}

fn snapshot(
    chars: &[char],
    at: usize,
    arena: &[Frame],
    stack: &[usize],
    mode: &Mode,
) -> Option<Snap> {
    let &top = stack.last()?;
    let f = &arena[top];
    let (in_string, slot, prefix, typing) = match mode {
        Mode::Line | Mode::Block(_) => return None,
        Mode::Str { is_key, text, .. } => (
            true,
            if *is_key { Slot::Key } else { Slot::Value },
            text.clone(),
            is_key.then_some(f.keys.len()),
        ),
        Mode::Normal => {
            let mut start = at;
            while start > 0 && is_word(chars[start - 1]) {
                start -= 1;
            }
            let prefix: String = chars[start..at].iter().collect();
            let slot = if f.obj && f.expect_key {
                Slot::Key
            } else {
                Slot::Value
            };
            (false, slot, prefix, None)
        }
    };
    let mut path = path_to(stack, arena, stack.len() - 1)?;
    if slot == Slot::Value {
        path.push(if f.obj {
            Seg::Key(f.cur_key.clone()?)
        } else {
            Seg::Index(f.index)
        });
    }
    Some(Snap {
        path,
        slot,
        in_string,
        prefix,
        frame: top,
        typing_key_at: typing,
    })
}

/// The context at char index `caret` of `text`; `None` inside a comment, or
/// outside every object and array.
pub fn context_at(text: &str, caret: usize) -> Option<Context> {
    let chars: Vec<char> = text.chars().collect();
    let mut arena: Vec<Frame> = vec![];
    let mut stack: Vec<usize> = vec![];
    let mut mode = Mode::Normal;
    let mut snap: Option<Option<Snap>> = None;
    let mut i = 0;
    loop {
        if i == caret && snap.is_none() {
            snap = Some(snapshot(&chars, i, &arena, &stack, &mode));
        }
        let Some(&c) = chars.get(i) else { break };
        match &mut mode {
            Mode::Str { is_key, text, esc } => {
                if *esc {
                    *esc = false;
                    text.push(c);
                } else if c == '\\' {
                    *esc = true;
                    text.push(c);
                } else if c == '"' || c == '\n' {
                    if *is_key {
                        if let Some(&top) = stack.last() {
                            arena[top].keys.push(text.clone());
                            arena[top].cur_key = Some(text.clone());
                        }
                    }
                    mode = Mode::Normal;
                } else {
                    text.push(c);
                }
            }
            Mode::Line => {
                if c == '\n' {
                    mode = Mode::Normal;
                }
            }
            Mode::Block(start) => {
                if c == '/' && i > *start + 1 && chars[i - 1] == '*' {
                    mode = Mode::Normal;
                }
            }
            Mode::Normal => match c {
                '"' => {
                    let is_key = stack
                        .last()
                        .is_some_and(|&t| arena[t].obj && arena[t].expect_key);
                    mode = Mode::Str {
                        is_key,
                        text: String::new(),
                        esc: false,
                    };
                }
                '/' if chars.get(i + 1) == Some(&'/') => mode = Mode::Line,
                '/' if chars.get(i + 1) == Some(&'*') => mode = Mode::Block(i),
                '{' | '[' => {
                    arena.push(Frame {
                        obj: c == '{',
                        keys: vec![],
                        cur_key: None,
                        expect_key: c == '{',
                        index: 0,
                    });
                    stack.push(arena.len() - 1);
                }
                '}' | ']' => {
                    stack.pop();
                }
                ':' => {
                    if let Some(&t) = stack.last() {
                        arena[t].expect_key = false;
                    }
                }
                ',' => {
                    if let Some(&t) = stack.last() {
                        let f = &mut arena[t];
                        if f.obj {
                            f.expect_key = true;
                            f.cur_key = None;
                        } else {
                            f.index += 1;
                        }
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    let snap = snap.flatten()?;
    let siblings = arena[snap.frame]
        .keys
        .iter()
        .enumerate()
        .filter(|(k, _)| Some(*k) != snap.typing_key_at)
        .map(|(_, key)| key.clone())
        .collect();
    let typed = snap.prefix.chars().count();
    Some(Context {
        path: snap.path,
        slot: snap.slot,
        in_string: snap.in_string,
        prefix: snap.prefix,
        typed,
        siblings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_end(text: &str) -> Option<Context> {
        context_at(text, text.chars().count())
    }

    fn key(s: &str) -> Seg {
        Seg::Key(s.into())
    }

    #[test]
    fn a_key_being_typed_knows_its_prefix_and_its_siblings() {
        let c = at_end("{ \"a\": 1, \"ui.fi").unwrap();
        assert_eq!(c.slot, Slot::Key);
        assert!(c.in_string);
        assert_eq!(c.prefix, "ui.fi");
        assert_eq!(c.typed, 5);
        assert!(c.path.is_empty());
        assert_eq!(c.siblings, vec!["a".to_string()]);
    }

    #[test]
    fn siblings_after_the_caret_count_and_the_key_being_typed_does_not() {
        let text = "{ \"ui.fi\", \"x\": 1 }";
        let c = context_at(text, 8).unwrap();
        assert_eq!(c.prefix, "ui.fi");
        assert_eq!(c.siblings, vec!["x".to_string()]);
    }

    #[test]
    fn a_value_knows_the_key_it_belongs_to() {
        let c = at_end("{\"theme\": \"Du").unwrap();
        assert_eq!((c.slot, c.in_string), (Slot::Value, true));
        assert_eq!(c.path, vec![key("theme")]);
        assert_eq!(c.prefix, "Du");
        // Nested, with nothing typed yet.
        let c = at_end("{\"editor\": {\"wrap\": ").unwrap();
        assert_eq!((c.slot, c.in_string), (Slot::Value, false));
        assert_eq!(c.path, vec![key("editor"), key("wrap")]);
        assert_eq!((c.prefix.as_str(), c.typed), ("", 0));
        // A bare word.
        let c = at_end("{\"a\": tr").unwrap();
        assert_eq!((c.prefix.as_str(), c.typed), ("tr", 2));
    }

    #[test]
    fn arrays_count_their_elements_and_objects_inside_them_have_paths() {
        let c = at_end("{\"a\": [1, ").unwrap();
        assert_eq!(c.path, vec![key("a"), Seg::Index(1)]);
        assert_eq!(c.slot, Slot::Value);
        let c = at_end("{\"a\": [{}, {\"k").unwrap();
        assert_eq!(c.path, vec![key("a"), Seg::Index(1)]);
        assert_eq!(c.slot, Slot::Key);
    }

    #[test]
    fn closed_brackets_give_their_path_back() {
        let c = at_end("{\"a\": {\"b\": 1}, \"c").unwrap();
        assert!(c.path.is_empty());
        assert_eq!(c.siblings, vec!["a".to_string()]);
    }

    #[test]
    fn comments_and_the_top_level_have_no_context() {
        assert_eq!(at_end("{ // \"ui."), None);
        assert_eq!(at_end("{ /* \"ui."), None);
        assert_eq!(at_end("\"ui."), None);
        // A line comment ends at the newline.
        assert!(at_end("{ // note\n \"ui.").is_some());
    }

    #[test]
    fn an_escaped_quote_does_not_end_the_string() {
        let c = at_end("{\"a\": \"x\\\"y").unwrap();
        assert_eq!(c.prefix, "x\\\"y");
    }
}
