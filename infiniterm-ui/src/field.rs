//! The one-line text field behind every prompt, the palette's query, the
//! omnibox, the find bars and the shortcuts filter. Port of what an
//! `<input>` gave the reference for free, and then some: the reference's
//! field had no caret, so Left and Right only dropped the selection and
//! Backspace could only eat the tail, which is not a text field anyone has
//! used since 1984.
//!
//! A caret (a char index) and an optional anchor: the selection is the span
//! between them, in either order, so Shift+Left extends leftwards and the
//! caret is the end that moves. The chord set is macOS's for a single-line
//! field, plus the Emacs handful the system fields also take (Ctrl+A, E, K,
//! D, H, B, F), because the person typing into these lives in a shell.
//!
//! Pure: no gpui element here, only the keystroke. The renderers draw from
//! `parts()`, three strings, so the caret and the selection land where they
//! are rather than at the end. The editing of the model's strings happens
//! here so the model stays a plain struct.
use gpui::Keystroke;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Field {
    pub text: String,
    /// Where typing goes, as a char index.
    caret: usize,
    /// The other end of the selection, when there is one.
    anchor: Option<usize>,
}

pub enum Edit {
    /// The text changed.
    Changed,
    /// The key was a field key (a caret or selection move) but the text is
    /// the same.
    Handled,
    /// Cmd+C: this goes on the clipboard, the text is unchanged.
    Copy(String),
    /// Cmd+X: this goes on the clipboard AND the text changed.
    Cut(String),
    /// Not a field key; the caller decides.
    Ignored,
}

impl Edit {
    /// Whether the text is different afterwards. A cut changes it too.
    pub fn changed(&self) -> bool {
        matches!(self, Edit::Changed | Edit::Cut(_))
    }

    /// What goes on the clipboard, for a copy or a cut.
    pub fn clipboard(&self) -> Option<&str> {
        match self {
            Edit::Copy(s) | Edit::Cut(s) => Some(s),
            _ => None,
        }
    }
}

impl Field {
    /// The field as an inline element: the text before the selection, the
    /// selection on its own ground, the caret where it is, the rest. Every
    /// div-drawn field (the palette, the omnibox, the find bar, the prompt,
    /// the shortcuts filter) draws through this, so a caret in the middle
    /// of a word looks the same in all of them. The caller draws the
    /// placeholder itself when the text is empty, because a caret against
    /// placeholder words suggests they were typed.
    pub fn inline(&self, sel_bg: gpui::Hsla, sel_fg: gpui::Hsla) -> gpui::Div {
        use gpui::{div, ParentElement, Styled};
        let (before, selected, after) = self.parts();
        let caret = selected.is_empty();
        let mut d = div().flex().flex_row().items_center();
        if !before.is_empty() {
            d = d.child(before);
        }
        if !selected.is_empty() {
            d = d.child(div().bg(sel_bg).text_color(sel_fg).child(selected));
        }
        if caret {
            d = d.child("▏");
        }
        if !after.is_empty() {
            d = d.child(after);
        }
        d
    }

    /// `selected` opens with the whole text selected, which is how a prompt
    /// offers a suggestion: typing replaces it and Enter keeps it.
    pub fn open(text: &str, selected: bool) -> Field {
        let len = text.chars().count();
        Field {
            text: text.into(),
            caret: len,
            anchor: selected.then_some(0).filter(|_| len > 0),
        }
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selection as `lo..hi` in chars, `None` when empty.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        if a == self.caret {
            return None;
        }
        Some((a.min(self.caret), a.max(self.caret)))
    }

    pub fn selected(&self) -> bool {
        self.selection().is_some()
    }

    pub fn selected_text(&self) -> String {
        match self.selection() {
            Some((lo, hi)) => self.text.chars().skip(lo).take(hi - lo).collect(),
            None => String::new(),
        }
    }

    /// What a renderer draws: the text before the selection (or the caret),
    /// the selected text, and the rest. With no selection the middle is
    /// empty and the caret sits between the first and the last.
    pub fn parts(&self) -> (String, String, String) {
        let (lo, hi) = self.selection().unwrap_or((self.caret, self.caret));
        let chars: Vec<char> = self.text.chars().collect();
        (
            chars[..lo].iter().collect(),
            chars[lo..hi].iter().collect(),
            chars[hi..].iter().collect(),
        )
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    /// `paste` is the clipboard's text when the key was Cmd+V.
    pub fn key(&mut self, k: &Keystroke, paste: Option<&str>) -> Edit {
        let m = &k.modifiers;
        let key = k.key.as_str();
        if m.platform {
            return match key {
                "a" => {
                    if self.len() > 0 {
                        self.anchor = Some(0);
                        self.caret = self.len();
                    }
                    Edit::Handled
                }
                "c" => Edit::Copy(self.selected_text()),
                "x" => {
                    let s = self.selected_text();
                    if s.is_empty() {
                        return Edit::Handled;
                    }
                    self.delete_selection();
                    Edit::Cut(s)
                }
                "v" => match paste {
                    Some(p) => {
                        self.insert(p.lines().next().unwrap_or(""));
                        Edit::Changed
                    }
                    None => Edit::Handled,
                },
                "backspace" => self.delete_back(Reach::Line),
                "delete" => self.delete_forward(Reach::Line),
                "left" | "up" => self.move_to(0, m.shift),
                "right" | "down" => self.move_to(self.len(), m.shift),
                _ => Edit::Ignored,
            };
        }
        if m.control {
            // The Emacs handful macOS text fields take natively.
            return match key {
                "a" => self.move_to(0, m.shift),
                "e" => self.move_to(self.len(), m.shift),
                "b" => self.step(-1, m.shift),
                "f" => self.step(1, m.shift),
                "h" => self.delete_back(Reach::Char),
                "d" => self.delete_forward(Reach::Char),
                "k" => {
                    let n = self.len();
                    self.anchor = None;
                    if self.caret < n {
                        self.text = self.text.chars().take(self.caret).collect();
                        Edit::Changed
                    } else {
                        Edit::Handled
                    }
                }
                _ => Edit::Ignored,
            };
        }
        let reach = if m.alt { Reach::Word } else { Reach::Char };
        match key {
            "backspace" => self.delete_back(reach),
            "delete" => self.delete_forward(reach),
            "left" => {
                if m.alt {
                    let to = self.word_left();
                    self.move_to(to, m.shift)
                } else {
                    self.step(-1, m.shift)
                }
            }
            "right" => {
                if m.alt {
                    let to = self.word_right();
                    self.move_to(to, m.shift)
                } else {
                    self.step(1, m.shift)
                }
            }
            "home" => self.move_to(0, m.shift),
            "end" => self.move_to(self.len(), m.shift),
            _ => match k.key_char.as_deref() {
                Some(ch) if !ch.is_empty() && !ch.chars().any(char::is_control) => {
                    self.insert(ch);
                    Edit::Changed
                }
                _ => Edit::Ignored,
            },
        }
    }

    // --- moves ---

    /// One char left or right. With a selection and no Shift, a plain arrow
    /// collapses it to that edge rather than stepping past it, which is
    /// what every text field does.
    fn step(&mut self, dir: isize, extend: bool) -> Edit {
        if !extend {
            if let Some((lo, hi)) = self.selection() {
                self.anchor = None;
                self.caret = if dir < 0 { lo } else { hi };
                return Edit::Handled;
            }
        }
        let to = if dir < 0 {
            self.caret.saturating_sub(1)
        } else {
            (self.caret + 1).min(self.len())
        };
        self.move_to(to, extend)
    }

    fn move_to(&mut self, to: usize, extend: bool) -> Edit {
        let to = to.min(self.len());
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.caret);
            }
        } else {
            self.anchor = None;
        }
        self.caret = to;
        Edit::Handled
    }

    /// The start of the word before the caret: back over spaces, then back
    /// over the word.
    fn word_left(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = self.caret;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }

    /// The end of the word after the caret: over spaces, then over the word.
    fn word_right(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let n = chars.len();
        let mut i = self.caret;
        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        while i < n && !chars[i].is_whitespace() {
            i += 1;
        }
        i
    }

    // --- edits ---

    /// Text with no key behind it (the emoji panel, a finished composition,
    /// an input method's commit), placed as typing would place it.
    pub fn insert_text(&mut self, with: &str) {
        self.insert(with);
    }

    fn insert(&mut self, with: &str) {
        self.delete_selection();
        let mut chars: Vec<char> = self.text.chars().collect();
        let at = self.caret.min(chars.len());
        let added: Vec<char> = with.chars().collect();
        let n = added.len();
        chars.splice(at..at, added);
        self.text = chars.into_iter().collect();
        self.caret = at + n;
    }

    /// Removes the selection if there is one. Returns whether it did.
    fn delete_selection(&mut self) -> bool {
        let Some((lo, hi)) = self.selection() else {
            self.anchor = None;
            return false;
        };
        self.remove(lo, hi);
        true
    }

    fn remove(&mut self, lo: usize, hi: usize) {
        let mut chars: Vec<char> = self.text.chars().collect();
        chars.drain(lo..hi.min(chars.len()));
        self.text = chars.into_iter().collect();
        self.caret = lo;
        self.anchor = None;
    }

    fn delete_back(&mut self, reach: Reach) -> Edit {
        if self.delete_selection() {
            return Edit::Changed;
        }
        if self.caret == 0 {
            return Edit::Handled;
        }
        let lo = match reach {
            Reach::Char => self.caret - 1,
            Reach::Word => self.word_left(),
            Reach::Line => 0,
        };
        self.remove(lo, self.caret);
        Edit::Changed
    }

    fn delete_forward(&mut self, reach: Reach) -> Edit {
        if self.delete_selection() {
            return Edit::Changed;
        }
        if self.caret >= self.len() {
            return Edit::Handled;
        }
        let hi = match reach {
            Reach::Char => self.caret + 1,
            Reach::Word => self.word_right(),
            Reach::Line => self.len(),
        };
        self.remove(self.caret, hi);
        Edit::Changed
    }
}

/// How far a delete reaches: the char, the word, or the whole line to that
/// side. Plain, Option and Cmd, on a Mac.
#[derive(Clone, Copy)]
enum Reach {
    Char,
    Word,
    Line,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;

    fn ks(k: &str, ch: Option<&str>, mods: Modifiers) -> Keystroke {
        Keystroke {
            modifiers: mods,
            key: k.into(),
            key_char: ch.map(Into::into),
        }
    }
    fn plain(k: &str) -> Keystroke {
        ks(k, None, Modifiers::default())
    }
    fn typed(c: &str) -> Keystroke {
        ks(c, Some(c), Modifiers::default())
    }
    fn cmd(k: &str) -> Keystroke {
        ks(
            k,
            None,
            Modifiers {
                platform: true,
                ..Default::default()
            },
        )
    }
    fn alt(k: &str) -> Keystroke {
        ks(
            k,
            None,
            Modifiers {
                alt: true,
                ..Default::default()
            },
        )
    }
    fn shift(k: &str) -> Keystroke {
        ks(
            k,
            None,
            Modifiers {
                shift: true,
                ..Default::default()
            },
        )
    }
    fn ctrl(k: &str) -> Keystroke {
        ks(
            k,
            None,
            Modifiers {
                control: true,
                ..Default::default()
            },
        )
    }
    fn alt_shift(k: &str) -> Keystroke {
        ks(
            k,
            None,
            Modifiers {
                alt: true,
                shift: true,
                ..Default::default()
            },
        )
    }

    // A prompt opens with its suggestion selected: typing replaces it.
    #[test]
    fn typing_over_a_selection_replaces_it() {
        let mut f = Field::open("api", true);
        f.key(&typed("b"), None);
        assert_eq!(f.text, "b");
        assert!(!f.selected());
        f.key(&typed("x"), None);
        assert_eq!(f.text, "bx");
    }

    #[test]
    fn cmd_a_selects_all_and_backspace_clears_the_selection() {
        let mut f = Field::open("hello", false);
        f.key(&cmd("a"), None);
        assert!(f.selected());
        f.key(&plain("backspace"), None);
        assert_eq!(f.text, "");
        // Nothing to select in an empty field.
        f.key(&cmd("a"), None);
        assert!(!f.selected());
    }

    // The bug this rewrite is for: the old field had no caret, so an arrow
    // could only drop the selection and Backspace could only eat the tail.
    #[test]
    fn the_caret_moves_and_edits_happen_where_it_is() {
        let mut f = Field::open("hello world", false);
        for _ in 0..6 {
            f.key(&plain("left"), None);
        }
        assert_eq!(f.caret(), 5);
        f.key(&typed(","), None);
        assert_eq!(f.text, "hello, world");
        f.key(&plain("backspace"), None);
        assert_eq!(f.text, "hello world");
        f.key(&plain("delete"), None);
        assert_eq!(
            f.text, "helloworld",
            "forward delete takes the char AFTER the caret"
        );
        assert_eq!(f.caret(), 5);
    }

    // A plain arrow on a selection collapses it to that edge, as in every
    // text field; the old one jumped to the end regardless.
    #[test]
    fn an_arrow_collapses_a_selection_to_its_edge() {
        let mut f = Field::open("api", true);
        f.key(&plain("left"), None);
        assert_eq!(
            (f.text.as_str(), f.selected(), f.caret()),
            ("api", false, 0)
        );
        let mut g = Field::open("api", true);
        g.key(&plain("right"), None);
        assert_eq!(g.caret(), 3);
        g.key(&plain("backspace"), None);
        assert_eq!(g.text, "ap");
    }

    #[test]
    fn option_moves_and_deletes_by_word() {
        let mut f = Field::open("one two three", false);
        f.key(&alt("left"), None);
        assert_eq!(f.caret(), 8, "to the start of `three`");
        f.key(&alt("left"), None);
        assert_eq!(f.caret(), 4);
        f.key(&alt("right"), None);
        assert_eq!(f.caret(), 7, "to the end of `two`");
        f.key(&alt("backspace"), None);
        assert_eq!(f.text, "one  three");
        let mut g = Field::open("one two", false);
        g.key(&plain("home"), None);
        g.key(&alt("delete"), None);
        assert_eq!(g.text, " two", "Option+Delete takes the word ahead");
    }

    #[test]
    fn cmd_arrows_and_home_end_go_to_the_line_ends_and_cmd_backspace_clears_to_the_start() {
        let mut f = Field::open("hello world", false);
        f.key(&cmd("left"), None);
        assert_eq!(f.caret(), 0);
        f.key(&cmd("right"), None);
        assert_eq!(f.caret(), 11);
        f.key(&plain("home"), None);
        assert_eq!(f.caret(), 0);
        f.key(&plain("end"), None);
        assert_eq!(f.caret(), 11);
        for _ in 0..5 {
            f.key(&plain("left"), None);
        }
        f.key(&cmd("backspace"), None);
        assert_eq!(f.text, "world");
        f.key(&plain("end"), None);
        f.key(&plain("left"), None);
        f.key(&cmd("delete"), None);
        assert_eq!(f.text, "worl", "Cmd+Delete clears to the end");
    }

    #[test]
    fn shift_extends_a_selection_from_the_caret_in_either_direction() {
        let mut f = Field::open("hello world", false);
        f.key(&shift("left"), None);
        f.key(&shift("left"), None);
        assert_eq!(f.selection(), Some((9, 11)));
        assert_eq!(f.selected_text(), "ld");
        f.key(&alt_shift("left"), None);
        assert_eq!(f.selected_text(), "world");
        f.key(&typed("x"), None);
        assert_eq!(f.text, "hello x");
        let mut g = Field::open("ab", false);
        g.key(&plain("home"), None);
        g.key(&shift("end"), None);
        assert_eq!(g.selected_text(), "ab");
    }

    #[test]
    fn copy_cut_and_paste_work_at_the_caret() {
        let mut f = Field::open("hello world", false);
        f.key(&plain("home"), None);
        f.key(&alt_shift("right"), None);
        assert!(matches!(f.key(&cmd("c"), None), Edit::Copy(s) if s == "hello"));
        assert_eq!(f.text, "hello world", "copy leaves the text alone");
        assert!(matches!(f.key(&cmd("x"), None), Edit::Cut(s) if s == "hello"));
        assert_eq!(f.text, " world");
        f.key(&cmd("v"), Some("bye\nsecond line"));
        assert_eq!(f.text, "bye world", "the first line, at the caret");
        assert!(matches!(f.key(&cmd("t"), None), Edit::Ignored));
    }

    // The Emacs handful the system fields take.
    #[test]
    fn the_emacs_keys_work_too() {
        let mut f = Field::open("hello world", false);
        f.key(&ctrl("a"), None);
        assert_eq!(f.caret(), 0);
        f.key(&ctrl("f"), None);
        f.key(&ctrl("f"), None);
        assert_eq!(f.caret(), 2);
        f.key(&ctrl("d"), None);
        assert_eq!(f.text, "helo world");
        f.key(&ctrl("k"), None);
        assert_eq!(f.text, "he");
        f.key(&ctrl("e"), None);
        f.key(&ctrl("h"), None);
        assert_eq!(f.text, "h");
    }

    // Turkish and everything else: the caret counts characters, not bytes.
    #[test]
    fn the_caret_counts_characters() {
        let mut f = Field::open("şeyler", false);
        f.key(&plain("left"), None);
        f.key(&plain("left"), None);
        f.key(&typed("ğ"), None);
        assert_eq!(f.text, "şeylğer");
        let (before, sel, after) = f.parts();
        assert_eq!(
            (before.as_str(), sel.as_str(), after.as_str()),
            ("şeylğ", "", "er")
        );
    }

    // A dead key on its own (Option+E, waiting for its vowel) arrives with
    // an EMPTY character. The field must not take it: taking it tells
    // macOS the key was handled and the composition never happens. The
    // composed `é` comes back later through `insert_text`.
    #[test]
    fn a_composition_prefix_is_ignored_and_the_result_is_inserted() {
        let mut f = Field::open("caf", false);
        let dead = ks(
            "e",
            Some(""),
            Modifiers {
                alt: true,
                ..Default::default()
            },
        );
        assert!(matches!(f.key(&dead, None), Edit::Ignored));
        assert_eq!(f.text, "caf", "nothing typed");
        f.insert_text("é");
        assert_eq!(f.text, "café");
    }

    // What the renderers draw from.
    #[test]
    fn parts_split_at_the_selection_or_the_caret() {
        let f = Field::open("hello", true);
        assert_eq!(f.parts(), ("".into(), "hello".into(), "".into()));
        let mut g = Field::open("hello", false);
        g.key(&plain("left"), None);
        g.key(&plain("left"), None);
        assert_eq!(g.parts(), ("hel".into(), "".into(), "lo".into()));
        g.key(&shift("right"), None);
        assert_eq!(g.parts(), ("hel".into(), "l".into(), "o".into()));
    }
}
