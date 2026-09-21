//! The text buffer: a rope, one cursor with an optional selection anchor,
//! and an undo stack. What `@codemirror/state` and `@codemirror/commands`
//! gave the reference's editor, for the one cursor the app ever used.
//!
//! Every position is a char index into the rope; lines and columns are
//! derived when the view asks. Edits go through `apply`, which records the
//! inverse for undo; typing merges into the previous step while it stays
//! one run of characters, so an undo takes back a word rather than a
//! letter, as CodeMirror's history groups it. Movement is what a Mac text
//! field does: Option by word, Cmd to the line's ends, Shift extends.
//!
//! No knowledge of files, drafts, languages or drawing: the body in
//! `infiniterm-ui` owns those and calls in.
use ropey::Rope;
use std::ops::Range;

/// Two spaces, CodeMirror's default indent unit and what `indentWithTab`
/// inserts.
pub const INDENT: &str = "  ";

/// Typing continues the same undo step for this long after the last key:
/// CodeMirror's `newGroupDelay`.
const GROUP_MS: f64 = 500.;

/// How far `matching_bracket` looks: a few thousand lines, well past any
/// bracket a person is matching by eye. shortcut: O(n) per frame while
/// the caret is on a bracket, fine under this cap.
const BRACKET_SCAN: usize = 200_000;

#[derive(Clone, Debug, PartialEq)]
struct Edit {
    at: usize,
    old: String,
    new: String,
    cursor_before: usize,
    cursor_after: usize,
    /// Whether the next typed character may merge into this step.
    typing: bool,
    when: f64,
}

#[derive(Clone, Debug)]
pub struct Buffer {
    text: Rope,
    cursor: usize,
    anchor: Option<usize>,
    /// The column an up/down keeps aiming at across short lines.
    goal_col: Option<usize>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    /// Bumped by every change, so a view can cache against it.
    pub version: u64,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Buffer {
    pub fn new(text: &str) -> Buffer {
        Buffer {
            text: Rope::from_str(text),
            cursor: 0,
            anchor: None,
            goal_col: None,
            undo: vec![],
            redo: vec![],
            version: 0,
        }
    }

    /// Replaces everything: a file loaded, the disk version taken. The
    /// cursor is clamped and the history kept, as CodeMirror's dispatch of
    /// a whole-document change would.
    pub fn set_text(&mut self, text: &str, now: f64) {
        let old = self.text.to_string();
        if old == text {
            return;
        }
        let cursor = self.cursor;
        self.apply(
            0,
            old,
            text.to_string(),
            cursor.min(text.chars().count()),
            false,
            now,
        );
        self.anchor = None;
    }

    pub fn text(&self) -> String {
        self.text.to_string()
    }

    /// Whether the buffer holds exactly `other`, without building a string.
    pub fn equals(&self, other: &str) -> bool {
        self.text.len_bytes() == other.len()
            && self.text.bytes().zip(other.bytes()).all(|(a, b)| a == b)
    }

    pub fn len_chars(&self) -> usize {
        self.text.len_chars()
    }

    pub fn line_count(&self) -> usize {
        self.text.len_lines()
    }

    /// Line `n` (0-based) without its newline.
    pub fn line(&self, n: usize) -> String {
        if n >= self.line_count() {
            return String::new();
        }
        let s = self.text.line(n).to_string();
        s.trim_end_matches(['\n', '\r']).to_string()
    }

    pub fn line_of(&self, idx: usize) -> usize {
        self.text.char_to_line(idx.min(self.len_chars()))
    }

    pub fn col_of(&self, idx: usize) -> usize {
        let idx = idx.min(self.len_chars());
        idx - self.text.line_to_char(self.line_of(idx))
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.text
            .line_to_char(line.min(self.line_count().saturating_sub(1)))
    }

    /// The BYTE offset a line starts at, which is what tree-sitter's spans
    /// are measured in. From the rope, so it is right for every line rather
    /// than only the ones some caller happened to scan for.
    pub fn line_byte_start(&self, line: usize) -> usize {
        self.text
            .line_to_byte(line.min(self.line_count().saturating_sub(1)))
    }

    /// The char index just before line `line`'s newline.
    pub fn line_end(&self, line: usize) -> usize {
        let start = self.line_start(line);
        start + self.line(line).chars().count()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn anchor(&self) -> Option<usize> {
        self.anchor
    }

    /// The selection as an ordered range, `None` when empty.
    pub fn selection(&self) -> Option<Range<usize>> {
        let a = self.anchor?;
        if a == self.cursor {
            return None;
        }
        Some(a.min(self.cursor)..a.max(self.cursor))
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|r| self.text.slice(r).to_string())
    }

    pub fn slice(&self, r: Range<usize>) -> String {
        self.text
            .slice(r.start.min(self.len_chars())..r.end.min(self.len_chars()))
            .to_string()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    // ----- movement -----

    fn place(&mut self, idx: usize, select: bool) {
        let idx = idx.min(self.len_chars());
        if select {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
        self.cursor = idx;
    }

    pub fn set_cursor(&mut self, idx: usize) {
        self.place(idx, false);
        self.goal_col = None;
    }

    /// A drag: the anchor stays where the press was.
    pub fn select_to(&mut self, idx: usize) {
        self.place(idx, true);
        self.goal_col = None;
    }

    pub fn select_range(&mut self, r: Range<usize>) {
        self.anchor = Some(r.start.min(self.len_chars()));
        self.cursor = r.end.min(self.len_chars());
        self.goal_col = None;
    }

    pub fn select_all(&mut self) {
        self.select_range(0..self.len_chars());
    }

    pub fn move_left(&mut self, select: bool) {
        self.goal_col = None;
        if let (false, Some(r)) = (select, self.selection()) {
            self.place(r.start, false);
            return;
        }
        self.place(self.cursor.saturating_sub(1), select);
    }

    pub fn move_right(&mut self, select: bool) {
        self.goal_col = None;
        if let (false, Some(r)) = (select, self.selection()) {
            self.place(r.end, false);
            return;
        }
        self.place(self.cursor + 1, select);
    }

    fn vertical(&mut self, delta: i64, select: bool) {
        let line = self.line_of(self.cursor) as i64;
        let col = *self.goal_col.get_or_insert(self.col_of(self.cursor));
        let target = line + delta;
        if target < 0 {
            self.place(0, select);
            return;
        }
        if target as usize >= self.line_count() {
            self.place(self.len_chars(), select);
            return;
        }
        let target = target as usize;
        let len = self.line(target).chars().count();
        let idx = self.line_start(target) + col.min(len);
        self.place(idx, select);
    }

    pub fn move_up(&mut self, select: bool) {
        self.vertical(-1, select);
    }

    pub fn move_down(&mut self, select: bool) {
        self.vertical(1, select);
    }

    pub fn move_page(&mut self, lines: usize, down: bool, select: bool) {
        self.vertical(if down { lines as i64 } else { -(lines as i64) }, select);
    }

    /// Option+Left: to the start of the word before the cursor.
    pub fn move_word_left(&mut self, select: bool) {
        self.goal_col = None;
        let mut i = self.cursor;
        while i > 0 && !is_word(self.text.char(i - 1)) {
            i -= 1;
        }
        while i > 0 && is_word(self.text.char(i - 1)) {
            i -= 1;
        }
        self.place(i, select);
    }

    /// Option+Right: to the end of the word after the cursor.
    pub fn move_word_right(&mut self, select: bool) {
        self.goal_col = None;
        let n = self.len_chars();
        let mut i = self.cursor;
        while i < n && !is_word(self.text.char(i)) {
            i += 1;
        }
        while i < n && is_word(self.text.char(i)) {
            i += 1;
        }
        self.place(i, select);
    }

    /// Cmd+Left, Home: the first non-blank of the line, then column 0.
    pub fn move_line_start(&mut self, select: bool) {
        self.goal_col = None;
        let line = self.line_of(self.cursor);
        let start = self.line_start(line);
        let text = self.line(line);
        let indent = text.chars().take_while(|c| c.is_whitespace()).count();
        let target = if self.cursor == start + indent || indent == text.chars().count() {
            start
        } else {
            start + indent
        };
        self.place(target, select);
    }

    pub fn move_line_end(&mut self, select: bool) {
        self.goal_col = None;
        let line = self.line_of(self.cursor);
        self.place(self.line_end(line), select);
    }

    pub fn move_doc_start(&mut self, select: bool) {
        self.goal_col = None;
        self.place(0, select);
    }

    pub fn move_doc_end(&mut self, select: bool) {
        self.goal_col = None;
        self.place(self.len_chars(), select);
    }

    /// Caret to the start of line `n` (1-based, clamped), nothing selected.
    pub fn go_to_line(&mut self, n: usize) {
        let line = n.max(1).min(self.line_count()) - 1;
        self.set_cursor(self.line_start(line));
    }

    /// The word around `idx`, for a double click.
    pub fn word_at(&self, idx: usize) -> Range<usize> {
        let n = self.len_chars();
        let idx = idx.min(n);
        let mut a = idx;
        let mut b = idx;
        while a > 0 && is_word(self.text.char(a - 1)) {
            a -= 1;
        }
        while b < n && is_word(self.text.char(b)) {
            b += 1;
        }
        a..b
    }

    pub fn line_range(&self, idx: usize) -> Range<usize> {
        let line = self.line_of(idx);
        let end = if line + 1 < self.line_count() {
            self.line_start(line + 1)
        } else {
            self.len_chars()
        };
        self.line_start(line)..end
    }

    // ----- edits -----

    /// The one way text changes. Records the inverse, moves the cursor,
    /// bumps the version.
    fn apply(
        &mut self,
        at: usize,
        old: String,
        new: String,
        cursor_after: usize,
        typing: bool,
        now: f64,
    ) {
        let old_len = old.chars().count();
        self.text.remove(at..at + old_len);
        self.text.insert(at, &new);
        let cursor_before = self.cursor;
        self.cursor = cursor_after.min(self.len_chars());
        self.anchor = None;
        self.goal_col = None;
        self.version += 1;
        self.redo.clear();
        // A typed character extends the step before it when that step was
        // typing too, ended where this begins, and is recent.
        if typing && old.is_empty() {
            if let Some(last) = self.undo.last_mut() {
                let end = last.at + last.new.chars().count();
                if last.typing && last.old.is_empty() && end == at && now - last.when <= GROUP_MS {
                    last.new.push_str(&new);
                    last.cursor_after = self.cursor;
                    last.when = now;
                    return;
                }
            }
        }
        self.undo.push(Edit {
            at,
            old,
            new,
            cursor_before,
            cursor_after: self.cursor,
            typing,
            when: now,
        });
    }

    /// Replaces the selection, or inserts at the cursor.
    pub fn insert(&mut self, s: &str, now: f64) {
        self.insert_typed(s, false, now);
    }

    fn insert_typed(&mut self, s: &str, typing: bool, now: f64) {
        let (at, old) = match self.selection() {
            Some(r) => (r.start, self.slice(r)),
            None => (self.cursor, String::new()),
        };
        let after = at + s.chars().count();
        self.apply(at, old, s.to_string(), after, typing, now);
    }

    /// A key with a character. Brackets and quotes come in pairs and the
    /// cursor lands between them; typing the closing half over one that is
    /// already there steps past it (CodeMirror's `closeBrackets`).
    pub fn type_char(&mut self, c: char, now: f64) {
        let next = (self.cursor < self.len_chars()).then(|| self.text.char(self.cursor));
        if self.selection().is_none() {
            if let Some(close) = closing_of(c) {
                let after_ok = next.is_none_or(|n| n.is_whitespace() || ")]}".contains(n));
                // A quote right after a word is an apostrophe, not a pair.
                let before_word = self.cursor > 0 && is_word(self.text.char(self.cursor - 1));
                if after_ok && !(is_quote(c) && before_word) {
                    let pair = format!("{c}{close}");
                    let at = self.cursor;
                    self.apply(at, String::new(), pair, at + 1, false, now);
                    return;
                }
            }
            if next == Some(c) && ")]}\"'`".contains(c) {
                self.set_cursor(self.cursor + 1);
                return;
            }
        }
        let mut s = String::new();
        s.push(c);
        self.insert_typed(&s, true, now);
    }

    /// Return: a newline plus the current line's indentation, the part of
    /// `indentOnInput` a plain editor needs.
    pub fn newline(&mut self, now: f64) {
        let line = self.line(self.line_of(self.cursor));
        let indent: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let indent: String = indent
            .chars()
            .take(self.col_of(self.cursor).min(indent.chars().count()))
            .collect();
        self.insert(&format!("\n{indent}"), now);
    }

    /// Tab: indent every selected line, or insert the unit at the cursor.
    pub fn tab(&mut self, now: f64) {
        match self.selection() {
            Some(r) if self.line_of(r.start) != self.line_of(r.end.saturating_sub(1)) => {
                self.indent_lines(r, true, now)
            }
            _ => self.insert(INDENT, now),
        }
    }

    /// Shift+Tab: outdent the selected lines, or the cursor's.
    pub fn outdent(&mut self, now: f64) {
        let r = self.selection().unwrap_or(self.cursor..self.cursor);
        self.indent_lines(r, false, now);
    }

    fn indent_lines(&mut self, r: Range<usize>, indent: bool, now: f64) {
        let first = self.line_of(r.start);
        let last = self.line_of(r.end.saturating_sub(1).max(r.start));
        let start = self.line_start(first);
        let end = self.line_end(last);
        let old = self.slice(start..end);
        let new: Vec<String> = old
            .split('\n')
            .map(|l| {
                if indent {
                    format!("{INDENT}{l}")
                } else if let Some(rest) = l.strip_prefix(INDENT) {
                    rest.to_string()
                } else {
                    l.trim_start_matches([' ', '\t']).to_string()
                }
            })
            .collect();
        let new = new.join("\n");
        let new_len = new.chars().count();
        self.apply(start, old, new, start + new_len, false, now);
        self.select_range(start..start + new_len);
    }

    /// Cmd+/: comment every selected line with `token`, or uncomment when
    /// they all are. The token goes at the shallowest indent of the block.
    pub fn toggle_comment(&mut self, token: &str, now: f64) {
        let r = self.selection().unwrap_or(self.cursor..self.cursor);
        let first = self.line_of(r.start);
        let last = self.line_of(r.end.saturating_sub(1).max(r.start));
        let start = self.line_start(first);
        let end = self.line_end(last);
        let old = self.slice(start..end);
        let lines: Vec<&str> = old.split('\n').collect();
        let lead = format!("{token} ");
        let all_commented = lines
            .iter()
            .filter(|l| !l.trim().is_empty())
            .all(|l| l.trim_start().starts_with(token));
        let indent = lines
            .iter()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.chars().take_while(|c| c.is_whitespace()).count())
            .min()
            .unwrap_or(0);
        let new: Vec<String> = lines
            .iter()
            .map(|l| {
                if all_commented {
                    let (ws, rest) = l.split_at(l.len() - l.trim_start().len());
                    let rest = rest
                        .strip_prefix(&lead)
                        .or_else(|| rest.strip_prefix(token))
                        .unwrap_or(rest);
                    format!("{ws}{rest}")
                } else if l.trim().is_empty() {
                    l.to_string()
                } else {
                    let (ws, rest) = l.split_at(
                        l.char_indices()
                            .nth(indent)
                            .map(|(i, _)| i)
                            .unwrap_or(l.len()),
                    );
                    format!("{ws}{lead}{rest}")
                }
            })
            .collect();
        let new = new.join("\n");
        let new_len = new.chars().count();
        let had_selection = self.selection().is_some();
        // Without a selection the caret goes to the NEXT line at the same
        // column, JetBrains' rule, so Cmd+/ down a block is one key per
        // line and the caret ends where the eye is; on the last line it
        // stays put. With a selection the block stays selected.
        let col = self.col_of(self.cursor);
        let line = self.line_of(self.cursor);
        self.apply(start, old, new, start, false, now);
        if had_selection {
            self.select_range(start..start + new_len);
            return;
        }
        let target = if line + 1 < self.line_count() { line + 1 } else { line };
        let len = self.line(target).chars().count();
        self.set_cursor(self.line_start(target) + col.min(len));
    }

    pub fn backspace(&mut self, now: f64) {
        if let Some(r) = self.selection() {
            let old = self.slice(r.clone());
            self.apply(r.start, old, String::new(), r.start, false, now);
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let prev = self.text.char(self.cursor - 1);
        let next = (self.cursor < self.len_chars()).then(|| self.text.char(self.cursor));
        // Between an auto-inserted pair, both go.
        let n = if closing_of(prev).is_some() && next == closing_of(prev) {
            2
        } else {
            1
        };
        let at = self.cursor - 1;
        let old = self.slice(at..at + n);
        self.apply(at, old, String::new(), at, false, now);
    }

    pub fn delete_forward(&mut self, now: f64) {
        if let Some(r) = self.selection() {
            let old = self.slice(r.clone());
            self.apply(r.start, old, String::new(), r.start, false, now);
            return;
        }
        if self.cursor >= self.len_chars() {
            return;
        }
        let at = self.cursor;
        let old = self.slice(at..at + 1);
        self.apply(at, old, String::new(), at, false, now);
    }

    /// Option+Backspace.
    pub fn delete_word_back(&mut self, now: f64) {
        if self.selection().is_some() {
            self.backspace(now);
            return;
        }
        let end = self.cursor;
        self.move_word_left(false);
        let start = self.cursor;
        self.cursor = end;
        if start < end {
            let old = self.slice(start..end);
            self.apply(start, old, String::new(), start, false, now);
        }
    }

    /// Cmd+Backspace.
    pub fn delete_to_line_start(&mut self, now: f64) {
        let end = self.cursor;
        let start = self.line_start(self.line_of(end));
        if start < end {
            let old = self.slice(start..end);
            self.apply(start, old, String::new(), start, false, now);
        }
    }

    /// Replace `r` with `s`, for search-and-replace; the cursor lands after.
    pub fn replace_range(&mut self, r: Range<usize>, s: &str, now: f64) {
        let old = self.slice(r.clone());
        let after = r.start + s.chars().count();
        self.apply(r.start, old, s.to_string(), after, false, now);
    }

    pub fn undo(&mut self) {
        let Some(e) = self.undo.pop() else {
            return;
        };
        let new_len = e.new.chars().count();
        self.text.remove(e.at..e.at + new_len);
        self.text.insert(e.at, &e.old);
        self.cursor = e.cursor_before.min(self.len_chars());
        self.anchor = None;
        self.goal_col = None;
        self.version += 1;
        self.redo.push(e);
    }

    pub fn redo(&mut self) {
        let Some(e) = self.redo.pop() else {
            return;
        };
        let old_len = e.old.chars().count();
        self.text.remove(e.at..e.at + old_len);
        self.text.insert(e.at, &e.new);
        self.cursor = e.cursor_after.min(self.len_chars());
        self.anchor = None;
        self.goal_col = None;
        self.version += 1;
        self.undo.push(e);
    }

    /// The bracket matching the one at or just before the cursor, for the
    /// view to light up. The scan stops after `BRACKET_SCAN` chars each
    /// way: the view asks every frame, a rope char read is a tree walk,
    /// and an unmatched bracket in a megabyte file would otherwise cost a
    /// full pass per frame for as long as the caret sat on it.
    pub fn matching_bracket(&self) -> Option<(usize, usize)> {
        let n = self.len_chars();
        let candidates = [self.cursor, self.cursor.wrapping_sub(1)];
        for &i in &candidates {
            if i >= n {
                continue;
            }
            let c = self.text.char(i);
            let (open, close, forward) = match c {
                '(' => ('(', ')', true),
                '[' => ('[', ']', true),
                '{' => ('{', '}', true),
                ')' => ('(', ')', false),
                ']' => ('[', ']', false),
                '}' => ('{', '}', false),
                _ => continue,
            };
            let mut depth = 0i32;
            if forward {
                for j in i..n.min(i + BRACKET_SCAN) {
                    let ch = self.text.char(j);
                    if ch == open {
                        depth += 1;
                    } else if ch == close {
                        depth -= 1;
                        if depth == 0 {
                            return Some((i, j));
                        }
                    }
                }
            } else {
                for j in (i.saturating_sub(BRACKET_SCAN)..=i).rev() {
                    let ch = self.text.char(j);
                    if ch == close {
                        depth += 1;
                    } else if ch == open {
                        depth -= 1;
                        if depth == 0 {
                            return Some((j, i));
                        }
                    }
                }
            }
            return None;
        }
        None
    }
}

fn closing_of(c: char) -> Option<char> {
    match c {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        '\'' => Some('\''),
        '`' => Some('`'),
        _ => None,
    }
}

fn is_quote(c: char) -> bool {
    matches!(c, '"' | '\'' | '`')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(b: &mut Buffer, s: &str, t: &mut f64) {
        for c in s.chars() {
            *t += 10.;
            if c == '\n' {
                b.newline(*t);
            } else {
                b.type_char(c, *t);
            }
        }
    }

    // CodeMirror's history: keys within 500 ms of each other are one step.
    #[test]
    fn typing_in_one_run_is_one_undo_step() {
        let mut b = Buffer::new("");
        let mut t = 0.;
        typed(&mut b, "hello world", &mut t);
        assert_eq!(b.text(), "hello world");
        b.undo();
        assert_eq!(b.text(), "");
        b.redo();
        assert_eq!(b.text(), "hello world");
        assert_eq!(b.cursor(), 11);
    }

    #[test]
    fn a_pause_starts_a_new_step() {
        let mut b = Buffer::new("");
        b.type_char('a', 0.);
        b.type_char('b', 2000.);
        b.undo();
        assert_eq!(b.text(), "a");
    }

    #[test]
    fn brackets_close_themselves_and_step_over() {
        let mut b = Buffer::new("");
        b.type_char('(', 0.);
        assert_eq!(b.text(), "()");
        assert_eq!(b.cursor(), 1);
        b.type_char('x', 1.);
        b.type_char(')', 2.);
        assert_eq!(b.text(), "(x)");
        assert_eq!(b.cursor(), 3);
        // An apostrophe after a word is not a pair.
        let mut b = Buffer::new("don");
        b.set_cursor(3);
        b.type_char('\'', 0.);
        assert_eq!(b.text(), "don'");
        // Backspace inside a fresh pair removes both.
        let mut b = Buffer::new("");
        b.type_char('[', 0.);
        b.backspace(1.);
        assert_eq!(b.text(), "");
    }

    #[test]
    fn newline_keeps_the_indent() {
        let mut b = Buffer::new("  foo");
        b.move_doc_end(false);
        b.newline(0.);
        assert_eq!(b.text(), "  foo\n  ");
        assert_eq!(b.cursor(), 8);
    }

    #[test]
    fn selection_replaces_and_movement_collapses_it() {
        let mut b = Buffer::new("hello world");
        b.set_cursor(0);
        b.move_word_right(true);
        assert_eq!(b.selected_text().as_deref(), Some("hello"));
        b.move_left(false);
        assert_eq!(b.selection(), None);
        assert_eq!(b.cursor(), 0);
        b.select_range(0..5);
        b.insert("bye", 0.);
        assert_eq!(b.text(), "bye world");
        b.select_all();
        assert_eq!(b.selected_text().as_deref(), Some("bye world"));
    }

    #[test]
    fn vertical_movement_remembers_the_goal_column() {
        let mut b = Buffer::new("a long line\nx\nanother long line");
        b.set_cursor(8);
        b.move_down(false);
        assert_eq!(b.cursor(), 13); // end of "x"
        b.move_down(false);
        assert_eq!((b.line_of(b.cursor()), b.col_of(b.cursor())), (2, 8));
        b.move_up(false);
        b.move_up(false);
        assert_eq!(b.cursor(), 8);
    }

    #[test]
    fn line_start_toggles_between_indent_and_column_zero() {
        let mut b = Buffer::new("    code");
        b.move_doc_end(false);
        b.move_line_start(false);
        assert_eq!(b.cursor(), 4);
        b.move_line_start(false);
        assert_eq!(b.cursor(), 0);
        b.move_line_end(false);
        assert_eq!(b.cursor(), 8);
    }

    #[test]
    fn word_deletes_and_line_deletes() {
        let mut b = Buffer::new("one two three");
        b.move_doc_end(false);
        b.delete_word_back(0.);
        assert_eq!(b.text(), "one two ");
        b.delete_to_line_start(1.);
        assert_eq!(b.text(), "");
        b.undo();
        assert_eq!(b.text(), "one two ");
    }

    #[test]
    fn toggle_comment_adds_and_removes_the_token_at_the_shallowest_indent() {
        let mut b = Buffer::new("fn a() {\n    x();\n\n    y();\n}");
        b.select_range(9..27);
        b.toggle_comment("//", 0.);
        assert_eq!(b.text(), "fn a() {\n    // x();\n\n    // y();\n}");
        b.toggle_comment("//", 1.);
        assert_eq!(b.text(), "fn a() {\n    x();\n\n    y();\n}");
        // Without a selection the caret moves to the next line at the same
        // column (clamped to it), and stays on the last line.
        let mut b = Buffer::new("abcdef\nxy\nz");
        b.set_cursor(4);
        b.toggle_comment("#", 0.);
        assert_eq!(b.text(), "# abcdef\nxy\nz");
        assert_eq!(b.cursor(), 9 + 2, "line 2, column 4 clamped to its length");
        b.toggle_comment("#", 1.);
        assert_eq!(b.text(), "# abcdef\n# xy\nz");
        assert_eq!(b.line_of(b.cursor()), 2);
        b.toggle_comment("#", 2.);
        assert_eq!(b.text(), "# abcdef\n# xy\n# z");
        assert_eq!(b.line_of(b.cursor()), 2, "the last line: stays");
    }

    #[test]
    fn tab_indents_a_multi_line_selection_and_inserts_otherwise() {
        let mut b = Buffer::new("a\nb");
        b.select_all();
        b.tab(0.);
        assert_eq!(b.text(), "  a\n  b");
        b.outdent(1.);
        assert_eq!(b.text(), "a\nb");
        let mut b = Buffer::new("x");
        b.set_cursor(1);
        b.tab(0.);
        assert_eq!(b.text(), "x  ");
    }

    #[test]
    fn go_to_line_clamps_and_lands_at_the_start() {
        let mut b = Buffer::new("one\ntwo\nthree");
        b.go_to_line(2);
        assert_eq!(b.cursor(), 4);
        b.go_to_line(99);
        assert_eq!(b.cursor(), 8);
        b.go_to_line(0);
        assert_eq!(b.cursor(), 0);
    }

    #[test]
    fn matching_bracket_is_found_both_ways() {
        let mut b = Buffer::new("f(a, [b])");
        b.set_cursor(1);
        assert_eq!(b.matching_bracket(), Some((1, 8)));
        b.set_cursor(9);
        assert_eq!(b.matching_bracket(), Some((1, 8)));
        b.set_cursor(6);
        assert_eq!(b.matching_bracket(), Some((5, 7)));
    }

    #[test]
    fn set_text_is_undoable_and_equals_avoids_a_copy() {
        let mut b = Buffer::new("old");
        b.set_text("new text", 0.);
        assert!(b.equals("new text"));
        assert!(!b.equals("new"));
        b.undo();
        assert_eq!(b.text(), "old");
    }

    #[test]
    fn unicode_positions_are_chars_not_bytes() {
        let mut b = Buffer::new("héllo wörld");
        b.set_cursor(0);
        b.move_word_right(false);
        assert_eq!(b.cursor(), 5);
        assert_eq!(b.word_at(8), 6..11);
        b.set_cursor(2);
        b.backspace(0.);
        assert_eq!(b.text(), "hllo wörld");
    }
}
