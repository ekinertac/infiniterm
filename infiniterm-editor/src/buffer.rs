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

/// One cursor's own state, standing in for the primary's three fields
/// (`cursor`, `anchor`, `goal_col`) when there is more than one.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CursorState {
    cursor: usize,
    anchor: Option<usize>,
    goal_col: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Buffer {
    text: Rope,
    cursor: usize,
    anchor: Option<usize>,
    /// The column an up/down keeps aiming at across short lines.
    goal_col: Option<usize>,
    /// Cursors beyond the primary (Batch 2, 2026-09-25). Empty in the
    /// overwhelmingly common single-cursor case, so every method written
    /// before multi-cursor existed keeps working exactly as it did as
    /// long as nothing has added one: `for_each_cursor` is the only place
    /// in this file that knows this field exists.
    extra: Vec<CursorState>,
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
            extra: vec![],
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
        // A whole-document replace (a save, a disk reload) invalidates
        // whatever the extra cursors pointed at.
        self.extra.clear();
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
        // A jump to an absolute line is a reset, not a multi-cursor
        // operation, and is never called per cursor from `for_each_cursor`.
        self.extra.clear();
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
    /// `indentOnInput` a plain editor needs. Between an auto-closed pair
    /// (the cursor right after `{` and right before its `}`) it opens the
    /// pair out onto three lines instead, caret indented on the middle one,
    /// CodeMirror's `closeBrackets` handler for Return.
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
        if self.selection().is_none() && self.cursor > 0 && self.cursor < self.len_chars() {
            let before = self.text.char(self.cursor - 1);
            let after = self.text.char(self.cursor);
            let pair = matches!((before, after), ('(', ')') | ('[', ']') | ('{', '}'));
            if pair {
                let inner = format!("{indent}{INDENT}");
                let inner_len = inner.chars().count();
                let new = format!("\n{inner}\n{indent}");
                let cursor_after = self.cursor + 1 + inner_len;
                self.apply(self.cursor, String::new(), new, cursor_after, false, now);
                return;
            }
        }
        self.insert(&format!("\n{indent}"), now);
    }

    /// Cmd+Shift+D: a copy of the current line inserted right below it,
    /// caret on the copy at the same column.
    pub fn duplicate_line(&mut self, now: f64) {
        let line = self.line_of(self.cursor);
        let col = self.col_of(self.cursor);
        let text = self.line(line);
        let last = line + 1 >= self.line_count();
        let at = if last { self.len_chars() } else { self.line_start(line + 1) };
        let new = if last {
            format!("\n{text}")
        } else {
            format!("{text}\n")
        };
        let cursor_after = at + usize::from(last) + col.min(text.chars().count());
        self.apply(at, String::new(), new, cursor_after, false, now);
    }

    /// Ctrl+Cmd+Up: the current line trades places with the one above it.
    pub fn swap_line_up(&mut self, now: f64) {
        let line = self.line_of(self.cursor);
        if line == 0 {
            return;
        }
        let col = self.col_of(self.cursor);
        let start = self.line_start(line - 1);
        let last = line + 1 >= self.line_count();
        let end = if last {
            self.len_chars()
        } else {
            self.line_start(line + 1)
        };
        let old = self.slice(start..end);
        let cur = self.line(line);
        let prev = self.line(line - 1);
        let new = if last {
            format!("{cur}\n{prev}")
        } else {
            format!("{cur}\n{prev}\n")
        };
        let cursor_after = start + col.min(cur.chars().count());
        self.apply(start, old, new, cursor_after, false, now);
    }

    /// Ctrl+Cmd+Down: the current line trades places with the one below it.
    pub fn swap_line_down(&mut self, now: f64) {
        let line = self.line_of(self.cursor);
        if line + 1 >= self.line_count() {
            return;
        }
        let col = self.col_of(self.cursor);
        let start = self.line_start(line);
        let last = line + 2 >= self.line_count();
        let end = if last {
            self.len_chars()
        } else {
            self.line_start(line + 2)
        };
        let old = self.slice(start..end);
        let cur = self.line(line);
        let next = self.line(line + 1);
        let new = if last {
            format!("{next}\n{cur}")
        } else {
            format!("{next}\n{cur}\n")
        };
        let cur_start_after = start + next.chars().count() + 1;
        let cursor_after = cur_start_after + col.min(cur.chars().count());
        self.apply(start, old, new, cursor_after, false, now);
    }

    /// The char just past line `line`'s own text: where the next line's
    /// text starts, or the end of the document on the last line.
    fn line_span_end(&self, line: usize) -> usize {
        if line + 1 < self.line_count() {
            self.line_start(line + 1)
        } else {
            self.len_chars()
        }
    }

    /// Ctrl+Shift+K: the whole current line, newline included, so the
    /// lines below step up. On the last line the newline ABOVE it goes
    /// instead, since there is none below.
    pub fn delete_line(&mut self, now: f64) {
        let line = self.line_of(self.cursor);
        let col = self.col_of(self.cursor);
        let count = self.line_count();
        let (start, end) = if line + 1 < count {
            (self.line_start(line), self.line_start(line + 1))
        } else if line > 0 {
            (self.line_start(line) - 1, self.len_chars())
        } else {
            (0, self.len_chars())
        };
        let old = self.slice(start..end);
        self.apply(start, old, String::new(), start, false, now);
        let target = line.min(self.line_count().saturating_sub(1));
        let len = self.line(target).chars().count();
        self.set_cursor(self.line_start(target) + col.min(len));
    }

    /// Cmd+Enter: a new line below the current one, indented the way
    /// Enter at the line's end would.
    pub fn add_line_below(&mut self, now: f64) {
        self.move_line_end(false);
        self.newline(now);
    }

    /// Cmd+Shift+Enter: a new blank line above the current one, matching
    /// its indent, caret at the end of it.
    pub fn add_line_above(&mut self, now: f64) {
        let line = self.line_of(self.cursor);
        let start = self.line_start(line);
        let indent: String = self
            .line(line)
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let indent_len = indent.chars().count();
        let new = format!("{indent}\n");
        self.apply(start, String::new(), new, start + indent_len, false, now);
    }

    /// Cmd+Shift+J: the next line joins this one, its leading whitespace
    /// collapsed to a single space, caret at the join.
    pub fn join_lines(&mut self, now: f64) {
        let line = self.line_of(self.cursor);
        if line + 1 >= self.line_count() {
            return;
        }
        let end_of_line = self.line_end(line);
        let next_start = self.line_start(line + 1);
        let next = self.line(line + 1);
        let ws = next.chars().take_while(|c| c.is_whitespace()).count();
        let remove_end = next_start + ws;
        let old = self.slice(end_of_line..remove_end);
        self.apply(end_of_line, old, " ".to_string(), end_of_line + 1, false, now);
    }

    /// Cmd+]: indent the current line (or every selected line), unlike Tab
    /// which inserts the unit at the cursor when nothing is selected.
    pub fn indent_line(&mut self, now: f64) {
        let r = self.selection().unwrap_or(self.cursor..self.cursor);
        self.indent_lines(r, true, now);
    }

    /// Cmd+L, called again on an already-whole-line selection: grows it by
    /// one more line. A fresh call selects the cursor's own line.
    pub fn expand_line_selection(&mut self) {
        let (first, last) = match self.selection() {
            Some(r) => (
                self.line_of(r.start),
                self.line_of(r.end.saturating_sub(1).max(r.start)),
            ),
            None => (self.line_of(self.cursor), self.line_of(self.cursor)),
        };
        let exact = self.selection().is_some_and(|r| {
            r.start == self.line_start(first) && r.end == self.line_span_end(last)
        });
        let last = if exact {
            (last + 1).min(self.line_count().saturating_sub(1))
        } else {
            last
        };
        let start = self.line_start(first);
        let end = self.line_span_end(last);
        self.select_range(start..end);
    }

    /// Cmd+D: the word under the cursor, or, called again on a selection,
    /// ANOTHER cursor on the next occurrence of the same text not already
    /// covered (wrapping) — Sublime's real multi-cursor Cmd+D, not the
    /// single roaming selection this was before there was a second cursor
    /// to add (Batch 1, 2026-09-24).
    pub fn select_word_or_next(&mut self) {
        if self.selection().is_none() && self.extra.is_empty() {
            let r = self.word_at(self.cursor);
            if !r.is_empty() {
                self.select_range(r);
            }
            return;
        }
        let Some(primary) = self.selection() else {
            return;
        };
        let text = self.slice(primary);
        if text.is_empty() {
            return;
        }
        let matches = crate::search::find_all(&self.text(), &text);
        if matches.is_empty() {
            return;
        }
        let states = self.all_cursor_states();
        let covered: std::collections::HashSet<usize> = states
            .iter()
            .filter_map(|c| c.anchor.map(|a| a.min(c.cursor)))
            .collect();
        let rightmost = states
            .iter()
            .map(|c| c.cursor.max(c.anchor.unwrap_or(c.cursor)))
            .max()
            .unwrap_or(0);
        let next = matches
            .iter()
            .find(|(s, _)| *s >= rightmost && !covered.contains(s))
            .or_else(|| matches.iter().find(|(s, _)| !covered.contains(s)))
            .copied();
        if let Some((a, b)) = next {
            self.extra.push(CursorState {
                cursor: b,
                anchor: Some(a),
                goal_col: None,
            });
        }
    }

    // ----- multi-cursor (Batch 2, 2026-09-25) -----

    fn all_cursor_states(&self) -> Vec<CursorState> {
        let mut v = vec![CursorState {
            cursor: self.cursor,
            anchor: self.anchor,
            goal_col: self.goal_col,
        }];
        v.extend(self.extra.iter().copied());
        v
    }

    pub fn cursor_count(&self) -> usize {
        1 + self.extra.len()
    }

    /// Every cursor's position and selection anchor, sorted by position,
    /// for the view to paint: a caret and a selection highlight per
    /// cursor instead of the one `cursor`/`selection` describe.
    pub fn all_selections(&self) -> Vec<(usize, Option<usize>)> {
        let mut v: Vec<(usize, Option<usize>)> = self
            .all_cursor_states()
            .iter()
            .map(|c| (c.cursor, c.anchor))
            .collect();
        v.sort_by_key(|(c, _)| *c);
        v
    }

    /// Cmd+Click: a new cursor at `idx`, or, clicking exactly where one
    /// already sits (and it is not the last one left), that cursor gone —
    /// Sublime's toggle.
    pub fn add_cursor_at(&mut self, idx: usize) {
        if idx == self.cursor && self.anchor.is_none() && self.extra.is_empty() {
            return;
        }
        if let Some(i) = self
            .extra
            .iter()
            .position(|c| c.cursor == idx && c.anchor.is_none())
        {
            self.extra.remove(i);
            return;
        }
        if idx == self.cursor && self.anchor.is_none() {
            return;
        }
        self.extra.push(CursorState {
            cursor: idx,
            anchor: None,
            goal_col: None,
        });
    }

    /// Ctrl+Shift+Down / Ctrl+Shift+Up: one more cursor, a line beyond
    /// whichever existing cursor is furthest in that direction, at its
    /// column. Building a column of cursors from a single one, called
    /// repeatedly, is the case this is for; starting from a scattered set
    /// it grows from the extreme one only, not each in turn, which is
    /// simpler than Sublime's own `select_lines` and enough for that case.
    pub fn add_cursor_line(&mut self, forward: bool) {
        let states = self.all_cursor_states();
        let edge = if forward {
            states.iter().max_by_key(|c| c.cursor)
        } else {
            states.iter().min_by_key(|c| c.cursor)
        };
        let Some(edge) = edge.copied() else {
            return;
        };
        let line = self.line_of(edge.cursor) as i64 + if forward { 1 } else { -1 };
        if line < 0 || line as usize >= self.line_count() {
            return;
        }
        let line = line as usize;
        let col = edge.goal_col.unwrap_or_else(|| self.col_of(edge.cursor));
        let len = self.line(line).chars().count();
        let idx = self.line_start(line) + col.min(len);
        if states.iter().any(|c| c.cursor == idx) {
            return;
        }
        self.extra.push(CursorState {
            cursor: idx,
            anchor: None,
            goal_col: Some(col),
        });
    }

    /// Ctrl+Cmd+G: every occurrence of the current selection (or the word
    /// under the cursor) becomes its own cursor, replacing whatever set
    /// there was before.
    pub fn select_all_occurrences(&mut self) {
        let r = self.selection().unwrap_or_else(|| self.word_at(self.cursor));
        if r.is_empty() {
            return;
        }
        let text = self.slice(r);
        let matches = crate::search::find_all(&self.text(), &text);
        let mut matches = matches.into_iter();
        let Some((a, b)) = matches.next() else {
            return;
        };
        self.anchor = Some(a);
        self.cursor = b;
        self.goal_col = None;
        self.extra = matches
            .map(|(a, b)| CursorState {
                cursor: b,
                anchor: Some(a),
                goal_col: None,
            })
            .collect();
    }

    /// Escape: back to one cursor, the primary's.
    pub fn collapse_to_primary(&mut self) {
        self.extra.clear();
    }

    /// Runs `f` once per cursor, right to left by position so an edit at
    /// one cannot shift a still-unprocessed cursor further left; every
    /// buffer method stays written for exactly one cursor and this is the
    /// only place that knows there can be more. With a single cursor
    /// (`extra` empty, overwhelmingly the common case) it is one direct
    /// call, no allocation.
    ///
    /// Known limit (v1, 2026-09-25): each cursor's edit still goes through
    /// `apply` on its own, so a multi-cursor edit is several undo steps,
    /// not one; Cmd+Z undoes them one cursor at a time, right to left,
    /// rather than the whole set at once.
    pub fn for_each_cursor(&mut self, mut f: impl FnMut(&mut Self)) {
        if self.extra.is_empty() {
            f(self);
            return;
        }
        let mut states = self.all_cursor_states();
        states.sort_by_key(|c| std::cmp::Reverse(c.cursor.max(c.anchor.unwrap_or(c.cursor))));
        let mut updated: Vec<CursorState> = Vec::with_capacity(states.len());
        for s in states {
            // The low end of what this cursor could touch: an edit here
            // can still shift a cursor already recorded (further right,
            // processed earlier) whose OWN position sits at or past it.
            let edit_at = s.cursor.min(s.anchor.unwrap_or(s.cursor));
            let before = self.len_chars() as i64;
            self.cursor = s.cursor;
            self.anchor = s.anchor;
            self.goal_col = s.goal_col;
            f(self);
            let delta = self.len_chars() as i64 - before;
            if delta != 0 {
                for u in &mut updated {
                    if u.cursor >= edit_at {
                        u.cursor = (u.cursor as i64 + delta).max(0) as usize;
                    }
                    if u.anchor.is_some_and(|a| a >= edit_at) {
                        u.anchor = Some((u.anchor.unwrap() as i64 + delta).max(0) as usize);
                    }
                }
            }
            updated.push(CursorState {
                cursor: self.cursor,
                anchor: self.anchor,
                goal_col: self.goal_col,
            });
        }
        // Left to right now, so two cursors an edit collapsed onto the
        // same spot (backspace at the start of a line the previous
        // cursor sits at the end of) dedupe instead of doubling up.
        updated.sort_by_key(|c| c.cursor);
        updated.dedup_by_key(|c| c.cursor);
        let mut iter = updated.into_iter();
        let primary = iter.next().expect("at least the primary itself ran");
        self.cursor = primary.cursor;
        self.anchor = primary.anchor;
        self.goal_col = primary.goal_col;
        self.extra = iter.collect();
    }

    /// Ctrl+Shift+M: the contents of the nearest enclosing bracket pair.
    /// Called again on a selection that already sits just inside one pair,
    /// it walks out to the pair around that one.
    pub fn expand_to_brackets(&mut self) {
        let scan_from = match self.selection() {
            Some(r) if r.start > 0 && matches!(self.text.char(r.start - 1), '(' | '[' | '{') => {
                r.start - 1
            }
            Some(r) => r.start,
            None => self.cursor,
        };
        if let Some((open, close)) = self.enclosing_brackets(scan_from) {
            self.select_range(open + 1..close);
        }
    }

    /// Scans outward from `from`: the nearest unmatched opener before it,
    /// then that opener's own match, the pair `expand_to_brackets` selects
    /// inside of.
    fn enclosing_brackets(&self, from: usize) -> Option<(usize, usize)> {
        let mut depth = 0i32;
        let mut i = from;
        let open = loop {
            if i == 0 {
                return None;
            }
            i -= 1;
            match self.text.char(i) {
                ')' | ']' | '}' => depth += 1,
                '(' | '[' | '{' if depth == 0 => break i,
                '(' | '[' | '{' => depth -= 1,
                _ => {}
            }
        };
        let open_char = self.text.char(open);
        let close_char = match open_char {
            '(' => ')',
            '[' => ']',
            _ => '}',
        };
        let mut depth = 0i32;
        for j in open + 1..self.len_chars() {
            let c = self.text.char(j);
            if c == open_char {
                depth += 1;
            } else if c == close_char {
                if depth == 0 {
                    return Some((open, j));
                }
                depth -= 1;
            }
        }
        None
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
    fn duplicate_line_copies_below_and_keeps_the_column() {
        let mut b = Buffer::new("abc\ndef");
        b.set_cursor(1);
        b.duplicate_line(0.);
        assert_eq!(b.text(), "abc\nabc\ndef");
        assert_eq!(b.cursor(), 5); // col 1 of the new middle line
        let mut b = Buffer::new("abc");
        b.set_cursor(2);
        b.duplicate_line(0.);
        assert_eq!(b.text(), "abc\nabc");
        assert_eq!(b.cursor(), 6);
    }

    #[test]
    fn swap_line_trades_places_and_the_caret_follows() {
        let mut b = Buffer::new("a\nb\nc");
        b.set_cursor(2); // on "b"
        b.swap_line_up(0.);
        assert_eq!(b.text(), "b\na\nc");
        assert_eq!(b.line_of(b.cursor()), 0);
        b.swap_line_down(1.);
        assert_eq!(b.text(), "a\nb\nc");
        assert_eq!(b.line_of(b.cursor()), 1);
        // The doc's edges refuse rather than losing a line.
        b.set_cursor(0);
        b.swap_line_up(2.);
        assert_eq!(b.text(), "a\nb\nc");
        b.set_cursor(4); // on "c", the last line
        b.swap_line_down(3.);
        assert_eq!(b.text(), "a\nb\nc");
    }

    #[test]
    fn delete_line_takes_the_newline_that_keeps_the_rest_together() {
        let mut b = Buffer::new("a\nb\nc");
        b.set_cursor(2); // "b"
        b.delete_line(0.);
        assert_eq!(b.text(), "a\nc");
        assert_eq!(b.cursor(), 2); // "c" took its place
        // The last line has no newline of its own; the one before it goes.
        let mut b = Buffer::new("a\nb");
        b.set_cursor(2);
        b.delete_line(0.);
        assert_eq!(b.text(), "a");
        // The only line: everything goes.
        let mut b = Buffer::new("solo");
        b.delete_line(0.);
        assert_eq!(b.text(), "");
    }

    #[test]
    fn add_line_below_and_above_keep_the_indent() {
        let mut b = Buffer::new("  foo");
        b.set_cursor(3);
        b.add_line_below(0.);
        assert_eq!(b.text(), "  foo\n  ");
        let mut b = Buffer::new("  foo");
        b.set_cursor(3);
        b.add_line_above(0.);
        assert_eq!(b.text(), "  \n  foo");
        assert_eq!(b.cursor(), 2);
    }

    #[test]
    fn join_lines_collapses_the_next_lines_indent_to_one_space() {
        let mut b = Buffer::new("foo\n  bar");
        b.set_cursor(0);
        b.join_lines(0.);
        assert_eq!(b.text(), "foo bar");
        assert_eq!(b.cursor(), 4);
        // The last line has nothing to join.
        b.set_cursor(b.len_chars());
        b.join_lines(1.);
        assert_eq!(b.text(), "foo bar");
    }

    #[test]
    fn indent_line_indents_the_whole_line_regardless_of_the_caret() {
        let mut b = Buffer::new("x");
        b.set_cursor(1);
        b.indent_line(0.);
        assert_eq!(b.text(), "  x");
    }

    #[test]
    fn newline_between_a_closed_pair_opens_it_onto_three_lines() {
        let mut b = Buffer::new("fn a() {}");
        b.set_cursor(8); // between { and }
        b.newline(0.);
        assert_eq!(b.text(), "fn a() {\n  \n}");
        assert_eq!(b.cursor(), 11);
        // A quote pair does not get this treatment.
        let mut b = Buffer::new("\"\"");
        b.set_cursor(1);
        b.newline(0.);
        assert_eq!(b.text(), "\"\n\"");
    }

    #[test]
    fn expand_line_selection_grows_by_one_line_each_call() {
        let mut b = Buffer::new("a\nb\nc");
        b.set_cursor(0);
        b.expand_line_selection();
        assert_eq!(b.selected_text().as_deref(), Some("a\n"));
        b.expand_line_selection();
        assert_eq!(b.selected_text().as_deref(), Some("a\nb\n"));
        b.expand_line_selection();
        assert_eq!(b.selected_text().as_deref(), Some("a\nb\nc"));
        b.expand_line_selection();
        assert_eq!(b.selected_text().as_deref(), Some("a\nb\nc"));
    }

    #[test]
    fn expand_to_brackets_selects_the_nearest_pair_then_the_next_one_out() {
        let mut b = Buffer::new("f(a, (b))");
        b.set_cursor(7); // on "b"
        b.expand_to_brackets();
        assert_eq!(b.selected_text().as_deref(), Some("b"));
        b.expand_to_brackets();
        assert_eq!(b.selected_text().as_deref(), Some("a, (b)"));
    }

    #[test]
    fn add_cursor_at_adds_and_toggles_off_on_a_second_click() {
        let mut b = Buffer::new("abc\ndef\nghi");
        b.set_cursor(1);
        b.add_cursor_at(6); // second line
        assert_eq!(b.cursor_count(), 2);
        b.add_cursor_at(6); // the same spot again: gone
        assert_eq!(b.cursor_count(), 1);
        // Clicking exactly on the sole primary is a no-op, not a second
        // cursor stacked on top of the first.
        b.add_cursor_at(1);
        assert_eq!(b.cursor_count(), 1);
    }

    #[test]
    fn add_cursor_line_builds_a_column_from_the_extreme_cursor() {
        let mut b = Buffer::new("one\ntwo\nthree\nfour");
        b.set_cursor(1); // "one", col 1
        b.add_cursor_line(true);
        assert_eq!(b.cursor_count(), 2);
        b.add_cursor_line(true);
        assert_eq!(b.cursor_count(), 3);
        let mut cols: Vec<usize> = b
            .all_selections()
            .iter()
            .map(|(c, _)| b.col_of(*c))
            .collect();
        cols.sort_unstable();
        assert_eq!(cols, vec![1, 1, 1], "same column on every added line");
        // The doc's last line refuses rather than dropping off the end.
        let mut b = Buffer::new("a\nb");
        b.set_cursor(2);
        b.add_cursor_line(true);
        assert_eq!(b.cursor_count(), 1);
    }

    #[test]
    fn select_all_occurrences_turns_every_match_into_a_cursor() {
        let mut b = Buffer::new("foo bar foo baz foo");
        b.select_range(0..3); // "foo"
        b.select_all_occurrences();
        assert_eq!(b.cursor_count(), 3);
        assert_eq!(b.all_selections(), vec![(3, Some(0)), (11, Some(8)), (19, Some(16))]);
    }

    #[test]
    fn select_word_or_next_adds_a_cursor_on_repeat_instead_of_moving() {
        let mut b = Buffer::new("foo bar foo baz foo");
        b.set_cursor(1);
        b.select_word_or_next(); // selects the first "foo", still one cursor
        assert_eq!(b.cursor_count(), 1);
        b.select_word_or_next(); // adds the second
        assert_eq!(b.cursor_count(), 2);
        b.select_word_or_next(); // adds the third
        assert_eq!(b.cursor_count(), 3);
        assert_eq!(b.all_selections(), vec![(3, Some(0)), (11, Some(8)), (19, Some(16))]);
        // A fourth press wraps rather than adding a duplicate.
        b.select_word_or_next();
        assert_eq!(b.cursor_count(), 3);
    }

    #[test]
    fn collapse_to_primary_drops_every_extra_cursor() {
        let mut b = Buffer::new("a\nb\nc");
        b.set_cursor(0);
        b.add_cursor_line(true);
        b.add_cursor_line(true);
        assert_eq!(b.cursor_count(), 3);
        b.collapse_to_primary();
        assert_eq!(b.cursor_count(), 1);
    }

    #[test]
    fn for_each_cursor_types_at_every_cursor_without_the_others_shifting() {
        let mut b = Buffer::new("a\nb\nc");
        b.set_cursor(0);
        b.add_cursor_line(true);
        b.add_cursor_line(true);
        assert_eq!(b.cursor_count(), 3);
        b.for_each_cursor(|buf| buf.type_char('x', 0.));
        assert_eq!(b.text(), "xa\nxb\nxc");
        // Every cursor landed right after its own inserted "x".
        let cols: Vec<usize> = b
            .all_selections()
            .iter()
            .map(|(c, _)| b.col_of(*c))
            .collect();
        assert_eq!(cols, vec![1, 1, 1]);
    }

    #[test]
    fn for_each_cursor_backspaces_at_every_cursor() {
        let mut b = Buffer::new("xa\nxb\nxc");
        b.set_cursor(1); // right after the first "x"
        b.add_cursor_at(4); // right after the second "x"
        b.add_cursor_at(7); // right after the third "x"
        b.for_each_cursor(|buf| buf.backspace(0.));
        assert_eq!(b.text(), "a\nb\nc");
    }

    #[test]
    fn for_each_cursor_with_one_cursor_is_a_single_direct_call() {
        let mut b = Buffer::new("abc");
        b.set_cursor(1);
        b.for_each_cursor(|buf| buf.type_char('x', 0.));
        assert_eq!(b.text(), "axbc");
        assert_eq!(b.cursor(), 2);
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
