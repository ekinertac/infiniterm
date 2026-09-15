//! The one-line text field behind the name prompt and the palette's query.
//! Port of what an `<input>` gave the reference for free.
//!
//! Selection is all-or-nothing: a prompt opens with its suggested value
//! selected, so typing replaces it and Enter keeps it (the reference's
//! `<input>` did the same by selecting on focus); Cmd+A selects all; a
//! typed character, Backspace or a paste replaces the selection; Left,
//! Right, Home and End clear it. No caret movement inside the text, which
//! neither field needed. The editing of the model's strings happens here so
//! the model stays a plain struct.
use gpui::Keystroke;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Field {
    pub text: String,
    pub selected: bool,
}

pub enum Edit {
    /// The text changed.
    Changed,
    /// The key was a field key (a selection change) but the text is the same.
    Handled,
    /// Not a field key; the caller decides.
    Ignored,
}

impl Field {
    pub fn open(text: &str, selected: bool) -> Field {
        Field {
            text: text.into(),
            selected,
        }
    }

    /// `paste` is the clipboard's text when the key was Cmd+V.
    pub fn key(&mut self, k: &Keystroke, paste: Option<&str>) -> Edit {
        let m = &k.modifiers;
        if m.platform {
            return match k.key.as_str() {
                "a" => {
                    self.selected = !self.text.is_empty();
                    Edit::Handled
                }
                "v" => match paste {
                    Some(p) => {
                        self.replace(p.lines().next().unwrap_or(""));
                        Edit::Changed
                    }
                    None => Edit::Handled,
                },
                "backspace" => {
                    self.text.clear();
                    self.selected = false;
                    Edit::Changed
                }
                _ => Edit::Ignored,
            };
        }
        match k.key.as_str() {
            "backspace" => {
                if self.selected || m.alt {
                    self.text.clear();
                    self.selected = false;
                } else {
                    self.text.pop();
                }
                Edit::Changed
            }
            "left" | "right" | "home" | "end" => {
                self.selected = false;
                Edit::Handled
            }
            _ => match k.key_char.as_deref() {
                Some(ch) if !ch.chars().any(char::is_control) => {
                    self.replace(ch);
                    Edit::Changed
                }
                _ => Edit::Ignored,
            },
        }
    }

    fn replace(&mut self, with: &str) {
        if self.selected {
            self.text.clear();
            self.selected = false;
        }
        self.text.push_str(with);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;

    fn key(k: &str, ch: Option<&str>, platform: bool) -> Keystroke {
        Keystroke {
            modifiers: Modifiers {
                platform,
                ..Default::default()
            },
            key: k.into(),
            key_char: ch.map(Into::into),
        }
    }

    // A prompt opens with its suggestion selected: typing replaces it.
    #[test]
    fn typing_over_a_selection_replaces_it() {
        let mut f = Field::open("api", true);
        f.key(&key("b", Some("b"), false), None);
        assert_eq!(f.text, "b");
        assert!(!f.selected);
        f.key(&key("x", Some("x"), false), None);
        assert_eq!(f.text, "bx");
    }

    #[test]
    fn cmd_a_selects_all_and_backspace_clears_the_selection() {
        let mut f = Field::open("hello", false);
        f.key(&key("a", None, true), None);
        assert!(f.selected);
        f.key(&key("backspace", None, false), None);
        assert_eq!(f.text, "");
        // Nothing to select in an empty field.
        f.key(&key("a", None, true), None);
        assert!(!f.selected);
    }

    #[test]
    fn arrows_keep_the_text_and_drop_the_selection() {
        let mut f = Field::open("api", true);
        f.key(&key("right", None, false), None);
        assert_eq!((f.text.as_str(), f.selected), ("api", false));
        f.key(&key("backspace", None, false), None);
        assert_eq!(f.text, "ap");
    }

    #[test]
    fn paste_replaces_the_selection_with_the_first_line() {
        let mut f = Field::open("api", true);
        f.key(&key("v", None, true), Some("web\nsecond"));
        assert_eq!(f.text, "web");
        assert!(matches!(f.key(&key("t", None, true), None), Edit::Ignored));
    }
}
