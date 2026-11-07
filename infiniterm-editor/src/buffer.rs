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
use unicode_segmentation::UnicodeSegmentation;

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
    /// The multi-cursor pass this edit belongs to (`for_each_cursor`), 0
    /// for an edit of its own. Undo and redo take every edit of a group in
    /// one step, so Cmd+Z undoes what one keypress did at every cursor.
    group: u32,
}

/// What a group of edits needs beyond the edits: every cursor before it and
/// after it, so undo and redo bring the whole set back, not only the one the
/// last edit happened at. `typing` is whether every edit was plain typed
/// text, which lets the next keypress at the same cursors join the group.
#[derive(Clone, Debug)]
struct GroupRec {
    before: Vec<CursorState>,
    after: Vec<CursorState>,
    typing: bool,
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
    /// Folded blocks (2026-09-29), each the hidden char range from the end
    /// of the header line to the end of the block's last line; edits shift
    /// them (`shift_folds`) and an edit that touches one drops it.
    folds: Vec<(usize, usize)>,
    /// The only blocks that CAN fold, when the text is a read-only diff
    /// (`set_fixed_folds`): same hidden char ranges as `folds`. Empty for an
    /// ordinary buffer, which folds by indentation. Never shifted by edits,
    /// because a buffer that has them is never edited.
    fixed_folds: Vec<(usize, usize)>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    /// The records of the multi-cursor groups the stacks hold, by id.
    groups: std::collections::HashMap<u32, GroupRec>,
    /// The id the last group got; ids only grow.
    last_group_id: u32,
    /// The group `apply` stamps its edits with while `for_each_cursor` runs
    /// (0 outside it), and whether every edit of the pass was plain typing.
    open_group: u32,
    group_typing: bool,
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
            folds: vec![],
            fixed_folds: vec![],
            undo: vec![],
            redo: vec![],
            groups: Default::default(),
            last_group_id: 0,
            open_group: 0,
            group_typing: true,
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
        // whatever the extra cursors and the folds pointed at.
        self.extra.clear();
        self.folds.clear();
        self.fixed_folds.clear();
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

    /// How many chars either side of a position are read to find the
    /// grapheme-cluster boundaries around it: a cluster is a few chars (a
    /// family emoji is eleven), never this many, and a line can be a megabyte.
    const GRAPHEME_WINDOW: usize = 64;

    /// The grapheme-cluster boundaries near `idx`, as char indices. A
    /// cluster is what a person calls one character: a letter with its
    /// combining accent, an emoji with its skin tone or a joined family, a
    /// flag (two regional indicators), a CRLF. The caret and Backspace move
    /// by these, so a caret never lands inside an emoji and Delete never
    /// leaves half of one behind.
    fn boundaries_near(&self, idx: usize) -> Vec<usize> {
        let lo = idx.saturating_sub(Self::GRAPHEME_WINDOW);
        let hi = (idx + Self::GRAPHEME_WINDOW).min(self.len_chars());
        let s = self.slice(lo..hi);
        let mut out = Vec::new();
        let mut at = lo;
        for g in s.graphemes(true) {
            out.push(at);
            at += g.chars().count();
        }
        out.push(at);
        out
    }

    /// The boundary just before `idx` (the start of the cluster to its left).
    pub fn grapheme_before(&self, idx: usize) -> usize {
        let idx = idx.min(self.len_chars());
        if idx == 0 {
            return 0;
        }
        let near = self.boundaries_near(idx);
        near.iter()
            .rev()
            .find(|b| **b < idx)
            .copied()
            .unwrap_or(idx - 1)
    }

    /// The boundary just after `idx` (the end of the cluster to its right).
    pub fn grapheme_after(&self, idx: usize) -> usize {
        let len = self.len_chars();
        if idx >= len {
            return len;
        }
        let near = self.boundaries_near(idx);
        near.iter().find(|b| **b > idx).copied().unwrap_or(idx + 1)
    }

    /// `idx` itself when it is a boundary, else the one before it: a click or
    /// a vertical move can name a char in the middle of a cluster.
    fn snap_to_boundary(&self, idx: usize) -> usize {
        let idx = idx.min(self.len_chars());
        if idx == 0 || idx == self.len_chars() {
            return idx;
        }
        let near = self.boundaries_near(idx);
        near.iter()
            .rev()
            .find(|b| **b <= idx)
            .copied()
            .unwrap_or(idx)
    }

    fn place(&mut self, idx: usize, select: bool) {
        let idx = self.snap_to_boundary(idx);
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
        self.place(self.grapheme_before(self.cursor), select);
    }

    pub fn move_right(&mut self, select: bool) {
        self.goal_col = None;
        if let (false, Some(r)) = (select, self.selection()) {
            self.place(r.end, false);
            return;
        }
        self.place(self.grapheme_after(self.cursor), select);
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
        let mut target = target as usize;
        // A folded block's lines are not there to land on: keep going the
        // same way until one is, or the document ends.
        while self.is_line_hidden(target) {
            let next = target as i64 + delta.signum();
            if next < 0 {
                self.place(0, select);
                return;
            }
            if next as usize >= self.line_count() {
                self.place(self.len_chars(), select);
                return;
            }
            target = next as usize;
        }
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
        self.shift_folds(at, old_len, new.chars().count());
        let cursor_before = self.cursor;
        self.cursor = cursor_after.min(self.len_chars());
        self.anchor = None;
        self.goal_col = None;
        self.version += 1;
        self.redo.clear();
        if self.open_group != 0 {
            self.group_typing &= typing && old.is_empty();
        }
        // A typed character extends the step before it when that step was
        // typing too, ended where this begins, and is recent. Never across a
        // multi-cursor group: the group joins whole (`for_each_cursor`).
        if typing && old.is_empty() && self.open_group == 0 {
            if let Some(last) = self.undo.last_mut().filter(|l| l.group == 0) {
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
            group: self.open_group,
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
    /// already there steps past it (CodeMirror's `closeBrackets`). Over a
    /// selection, an opening half wraps it.
    pub fn type_char(&mut self, c: char, now: f64) {
        // An opening bracket or a quote over a selection wraps it instead of
        // replacing it, and the inner text stays selected, so a second press
        // wraps again (Sublime and VS Code both do this).
        if let (Some(r), Some(close)) = (self.selection(), closing_of(c)) {
            let inner = self.slice(r.clone());
            let new = format!("{c}{inner}{close}");
            let after = r.start + 1 + inner.chars().count();
            self.apply(r.start, inner, new, after, false, now);
            self.select_range(r.start + 1..after);
            return;
        }
        if self.selection().is_none() && matches!(c, '}' | ')' | ']') {
            // Typed over an auto-inserted closer it is a step over, not a
            // new bracket, and the dedent is the closer's own line's matter.
            let over = self.cursor < self.len_chars() && self.text.char(self.cursor) == c;
            if !over {
                self.dedent_before_closer(now);
            }
        }
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
        // After an opening bracket the next line is one level deeper, as in
        // every editor: `if x {` then Enter. Looking left past spaces, so
        // `foo(  ` counts; text to the right of the caret moves down with it.
        let before_caret: String =
            self.slice(self.line_start(self.line_of(self.cursor))..self.cursor);
        let opens = before_caret
            .trim_end_matches([' ', '\t'])
            .chars()
            .next_back()
            .is_some_and(|c| matches!(c, '{' | '(' | '['));
        let indent = if opens && self.selection().is_none() {
            format!("{indent}{INDENT}")
        } else {
            indent
        };
        self.insert(&format!("\n{indent}"), now);
    }

    /// Typing a closing bracket on a line that holds only indent takes one
    /// level of it off first, so the `}` lines up with what opened the block.
    /// A tab counts as a level; so does a run of `INDENT` spaces. Anything
    /// else on the line, or no indent to take, leaves it alone.
    fn dedent_before_closer(&mut self, now: f64) {
        let line_start = self.line_start(self.line_of(self.cursor));
        let before = self.slice(line_start..self.cursor);
        if before.is_empty() || !before.chars().all(|c| c == ' ' || c == '\t') {
            return;
        }
        let cut = if before.ends_with('\t') {
            1
        } else if before.ends_with(INDENT) {
            INDENT.chars().count()
        } else {
            return;
        };
        let at = self.cursor - cut;
        let old = self.slice(at..self.cursor);
        self.apply(at, old, String::new(), at, false, now);
    }

    /// The first and last line the selection touches, or the caret's line.
    /// A selection that ends at column 0 of a line leaves that line out,
    /// as VS Code and Sublime do, so a Cmd+L selection moves one line.
    fn touched_lines(&self) -> (usize, usize) {
        match self.selection() {
            Some(r) => {
                let first = self.line_of(r.start);
                let mut last = self.line_of(r.end);
                if last > first && self.col_of(r.end) == 0 {
                    last -= 1;
                }
                (first, last)
            }
            None => {
                let line = self.line_of(self.cursor);
                (line, line)
            }
        }
    }

    /// Puts the selection back after a line edit moved its text by `shift`
    /// chars; `apply` leaves only a caret.
    fn reselect(&mut self, anchor: Option<usize>, shift: i64) {
        let len = self.len_chars();
        self.anchor = anchor.map(|a| ((a as i64 + shift).max(0) as usize).min(len));
    }

    /// Cmd+Shift+D: a selection inside one line is copied right after
    /// itself and the copy stays selected; otherwise every line the
    /// selection touches (or the caret's line) is copied below, caret and
    /// selection on the copy.
    pub fn duplicate_line(&mut self, now: f64) {
        if let Some(r) = self.selection() {
            if self.line_of(r.start) == self.line_of(r.end) {
                let text = self.slice(r.clone());
                let len = text.chars().count();
                let backwards = self.anchor.is_some_and(|a| a > self.cursor);
                self.apply(r.end, String::new(), text, r.end + len, false, now);
                let (from, to) = (r.end, r.end + len);
                let (a, c) = if backwards { (to, from) } else { (from, to) };
                self.anchor = Some(a);
                self.cursor = c;
                return;
            }
        }
        self.copy_lines(false, now);
    }

    /// Shift+Alt+Down (below) and Shift+Alt+Up (above): copies every line
    /// the selection touches. The caret and selection go with the copy, so
    /// below they move down by the block and above they keep their place.
    pub fn copy_lines(&mut self, up: bool, now: f64) {
        let (first, last) = self.touched_lines();
        let (anchor, cursor) = (self.anchor, self.cursor);
        let start = self.line_start(first);
        let body = self.slice(start..self.line_end(last));
        let body_len = body.chars().count() as i64;
        let no_newline = self.line_span_end(last) == self.line_end(last);
        let (at, new, shift) = if up {
            (start, format!("{body}\n"), 0)
        } else if no_newline {
            (self.len_chars(), format!("\n{body}"), body_len + 1)
        } else {
            (self.line_span_end(last), format!("{body}\n"), body_len + 1)
        };
        let cursor_after = (cursor as i64 + shift) as usize;
        self.apply(at, String::new(), new, cursor_after, false, now);
        self.reselect(anchor, shift);
    }

    /// Alt+Up / Alt+Down: every line the selection touches trades places
    /// with the line above or below it, and the selection moves with them.
    pub fn move_lines(&mut self, up: bool, now: f64) {
        let (first, last) = self.touched_lines();
        if (up && first == 0) || (!up && last + 1 >= self.line_count()) {
            return;
        }
        let (anchor, cursor) = (self.anchor, self.cursor);
        let neighbour = if up { first - 1 } else { last + 1 };
        let (lo, hi) = if up {
            (neighbour, last)
        } else {
            (first, neighbour)
        };
        let start = self.line_start(lo);
        let end = self.line_span_end(hi);
        let old = self.slice(start..end);
        let body = self.slice(self.line_start(first)..self.line_end(last));
        let other = self.line(neighbour);
        let other_len = other.chars().count() as i64;
        let tail = if end > self.line_end(hi) { "\n" } else { "" };
        let (new, shift) = if up {
            (format!("{body}\n{other}{tail}"), -(other_len + 1))
        } else {
            (format!("{other}\n{body}{tail}"), other_len + 1)
        };
        let cursor_after = (cursor as i64 + shift).max(0) as usize;
        self.apply(start, old, new, cursor_after, false, now);
        self.reselect(anchor, shift);
    }

    /// Ctrl+Cmd+Up: the line (or every selected line) trades places with
    /// the one above it.
    pub fn swap_line_up(&mut self, now: f64) {
        self.move_lines(true, now);
    }

    /// Ctrl+Cmd+Down: the line (or every selected line) trades places with
    /// the one below it.
    pub fn swap_line_down(&mut self, now: f64) {
        self.move_lines(false, now);
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
        self.apply(
            end_of_line,
            old,
            " ".to_string(),
            end_of_line + 1,
            false,
            now,
        );
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

    // ----- folding (Batch 2, 2026-09-29) -----

    /// Leading blanks of a line: what "deeper" means for a fold. A tab
    /// counts as one, so a file that mixes tabs and spaces folds oddly.
    fn indent_of(text: &str) -> usize {
        text.chars().take_while(|c| *c == ' ' || *c == '\t').count()
    }

    /// The hidden range a fold starting at `line` would have: the line's
    /// end to the end of the last following line indented deeper than it
    /// (blank lines inside a block belong to it; blank lines after it do
    /// not). `None` when nothing is indented under it.
    fn fold_range_at(&self, line: usize) -> Option<(usize, usize)> {
        if !self.fixed_folds.is_empty() {
            let end = self.line_end(line);
            return self.fixed_folds.iter().find(|(s, _)| *s == end).copied();
        }
        let text = self.line(line);
        if text.trim().is_empty() {
            return None;
        }
        let base = Self::indent_of(&text);
        let mut last = None;
        for l in line + 1..self.line_count() {
            let t = self.line(l);
            if t.trim().is_empty() {
                continue;
            }
            if Self::indent_of(&t) > base {
                last = Some(l);
            } else {
                break;
            }
        }
        Some((self.line_end(line), self.line_end(last?)))
    }

    /// The nearest line at or above `line` that can fold: itself when a
    /// block starts there, else the header of the block it sits in.
    fn enclosing_fold_line(&self, line: usize) -> Option<usize> {
        if self.fold_range_at(line).is_some() {
            return Some(line);
        }
        if !self.fixed_folds.is_empty() {
            // Inside an opened block: its header, by position.
            let at = self.line_start(line);
            return self
                .fixed_folds
                .iter()
                .find(|(s, e)| at > *s && at <= *e)
                .map(|(s, _)| self.line_of(*s));
        }
        let mut want = Self::indent_of(&self.line(line));
        for l in (0..line).rev() {
            let t = self.line(l);
            if t.trim().is_empty() {
                continue;
            }
            let ind = Self::indent_of(&t);
            if ind < want {
                if self.fold_range_at(l).is_some() {
                    return Some(l);
                }
                want = ind;
            }
        }
        None
    }

    /// Whether `line` is inside a folded block, so not drawn or landed on.
    pub fn is_line_hidden(&self, line: usize) -> bool {
        if self.folds.is_empty() {
            return false;
        }
        let ls = self.line_start(line);
        self.folds.iter().any(|(s, e)| ls > *s && ls <= *e)
    }

    /// Whether a fold starts on `line`, for the marker after its text.
    pub fn is_fold_header(&self, line: usize) -> bool {
        self.folds.iter().any(|(s, _)| self.line_of(*s) == line)
    }

    /// Whether anything is folded at all, the painter's cheap way out.
    pub fn has_folds(&self) -> bool {
        !self.folds.is_empty()
    }

    /// Makes these the only blocks that can fold, all folded now: for a
    /// read-only diff, whose unchanged stretches are folds that have nothing
    /// to do with indentation. Each is (header line, last hidden line),
    /// 0-based; the header stays visible. Call after the text is set, which
    /// forgets them.
    pub fn set_fixed_folds(&mut self, blocks: &[(usize, usize)]) {
        self.fixed_folds = blocks
            .iter()
            .filter(|(h, last)| h < last)
            .map(|(h, last)| (self.line_end(*h), self.line_end(*last)))
            .collect();
        self.folds = self.fixed_folds.clone();
        self.extra.clear();
    }

    /// Cmd+Alt+[: fold the block the caret's line starts or sits in, and
    /// leave the caret on its header so it does not land in what it hid.
    pub fn fold_at_cursor(&mut self) {
        let line = self.line_of(self.cursor);
        let Some(header) = self.enclosing_fold_line(line) else {
            return;
        };
        let Some(range) = self.fold_range_at(header) else {
            return;
        };
        if !self.folds.contains(&range) {
            self.folds.push(range);
        }
        self.extra.clear();
        self.set_cursor(range.0);
    }

    /// A click in the gutter: fold the block `line` starts, or open the one
    /// it heads. A line with nothing under it does nothing (unlike
    /// `fold_at_cursor`, which finds the block around it).
    pub fn toggle_fold_line(&mut self, line: usize) {
        let end = self.line_end(line);
        if self.folds.iter().any(|(s, _)| *s == end) {
            self.folds.retain(|(s, _)| *s != end);
        } else if let Some(range) = self.fold_range_at(line) {
            self.folds.push(range);
            let c = self.cursor;
            if c > range.0 && c <= range.1 {
                self.set_cursor(range.0);
            }
        }
    }

    /// Cmd+Alt+]: open the fold the caret's line heads or hides in.
    pub fn unfold_at_cursor(&mut self) {
        let c = self.cursor;
        let line = self.line_of(c);
        let end = self.line_end(line);
        self.folds
            .retain(|(s, e)| !(*s == end || (c > *s && c <= *e)));
    }

    pub fn unfold_all(&mut self) {
        self.folds.clear();
    }

    /// Folds every outermost block: the lines at the file's smallest
    /// indent that have something under them.
    pub fn fold_all(&mut self) {
        if !self.fixed_folds.is_empty() {
            self.folds = self.fixed_folds.clone();
            let c = self.cursor;
            if let Some((s, _)) = self.folds.iter().find(|(s, e)| c > *s && c <= *e).copied() {
                self.set_cursor(s);
            }
            self.extra.clear();
            return;
        }
        let count = self.line_count();
        let base = (0..count)
            .map(|l| self.line(l))
            .filter(|t| !t.trim().is_empty())
            .map(|t| Self::indent_of(&t))
            .min()
            .unwrap_or(0);
        let mut folds = vec![];
        let mut l = 0;
        while l < count {
            let t = self.line(l);
            if !t.trim().is_empty() && Self::indent_of(&t) == base {
                if let Some(range) = self.fold_range_at(l) {
                    l = self.line_of(range.1);
                    folds.push(range);
                }
            }
            l += 1;
        }
        self.folds = folds;
        let c = self.cursor;
        if self.folds.iter().any(|(s, e)| c > *s && c <= *e) {
            let s = self
                .folds
                .iter()
                .find(|(s, e)| c > *s && c <= *e)
                .unwrap()
                .0;
            self.set_cursor(s);
        }
        self.extra.clear();
    }

    /// A caret or selection end inside a folded block opens it: whatever
    /// moved there (a search match, an undo, a click) meant to see it.
    pub fn reveal_cursors(&mut self) {
        if self.folds.is_empty() {
            return;
        }
        let points: Vec<usize> = self
            .all_cursor_states()
            .iter()
            .flat_map(|c| [Some(c.cursor), c.anchor])
            .flatten()
            .collect();
        self.folds
            .retain(|(s, e)| !points.iter().any(|p| *p > *s && *p <= *e));
    }

    /// An edit replaced `old_len` chars at `at` with `new_len`: folds
    /// wholly before it stay, folds wholly after it move by the length
    /// change, and one the edit touches is dropped.
    fn shift_folds(&mut self, at: usize, old_len: usize, new_len: usize) {
        if self.folds.is_empty() {
            return;
        }
        let delta = new_len as i64 - old_len as i64;
        self.folds.retain_mut(|(s, e)| {
            if at + old_len <= *s {
                *s = (*s as i64 + delta) as usize;
                *e = (*e as i64 + delta) as usize;
                true
            } else {
                at >= *e
            }
        });
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
        let r = self
            .selection()
            .unwrap_or_else(|| self.word_at(self.cursor));
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

    /// Cmd+Shift+L: a selection spanning several lines becomes one
    /// selection per line, each the original's own slice of it — Sublime's
    /// Split into Lines, the shape Cmd+A then this is asked for most: type
    /// over the lot and every line takes the same edit at once. A line
    /// the selection reaches only at its very boundary (nothing of the
    /// line itself selected) still gets a point cursor there, not a gap in
    /// the middle of the set. Nothing to split (no selection, or one
    /// line) is a no-op.
    pub fn split_into_lines(&mut self) {
        let Some(r) = self.selection() else {
            return;
        };
        let first = self.line_of(r.start);
        let last = self.line_of(r.end.saturating_sub(1).max(r.start));
        if first == last {
            return;
        }
        let ranges: Vec<(usize, usize)> = (first..=last)
            .map(|line| {
                let start = r.start.max(self.line_start(line));
                let end = r.end.min(self.line_end(line));
                (start, end)
            })
            .collect();
        let mut iter = ranges.into_iter();
        let (a, b) = iter.next().expect("first..=last is never empty");
        self.cursor = b;
        self.anchor = (a != b).then_some(a);
        self.goal_col = None;
        self.extra = iter
            .map(|(a, b)| CursorState {
                cursor: b,
                anchor: (a != b).then_some(a),
                goal_col: None,
            })
            .collect();
    }

    /// Runs `f` once per cursor, right to left by position so an edit at
    /// one cannot shift a still-unprocessed cursor further left; every
    /// buffer method stays written for exactly one cursor and this is the
    /// only place that knows there can be more. With a single cursor
    /// (`extra` empty, overwhelmingly the common case) it is one direct
    /// call, no allocation.
    ///
    /// Each cursor's edit still goes through `apply` on its own, but the
    /// edits of one pass share a group (`Edit::group`), so one Cmd+Z undoes
    /// the pass at every cursor and brings every cursor back. Typed
    /// characters at the same set of cursors, within the typing window, join
    /// the previous group, as they do for one cursor.
    pub fn for_each_cursor(&mut self, mut f: impl FnMut(&mut Self)) {
        if self.extra.is_empty() {
            f(self);
            return;
        }
        let before_states = self.all_cursor_states();
        let first_edit = self.undo.len();
        self.last_group_id += 1;
        self.open_group = self.last_group_id;
        self.group_typing = true;
        self.run_cursors(&mut f);
        self.open_group = 0;
        if self.undo.len() > first_edit {
            self.close_group(first_edit, before_states);
        }
    }

    /// Files the edits a pass made from `first_edit` on as one group, or as
    /// a continuation of the previous one.
    fn close_group(&mut self, first_edit: usize, before: Vec<CursorState>) {
        let id = self.last_group_id;
        let after = self.all_cursor_states();
        let positions = |states: &[CursorState]| {
            let mut v: Vec<usize> = states.iter().map(|c| c.cursor).collect();
            v.sort_unstable();
            v
        };
        let joins = first_edit
            .checked_sub(1)
            .and_then(|i| self.undo.get(i))
            .filter(|prev| prev.group != 0)
            .and_then(|prev| {
                let rec = self.groups.get(&prev.group)?;
                let recent = self
                    .undo
                    .last()
                    .is_some_and(|e| e.when - prev.when <= GROUP_MS);
                let same_spots = positions(&rec.after) == positions(&before)
                    && before.iter().all(|c| c.anchor.is_none());
                (self.group_typing && rec.typing && recent && same_spots).then_some(prev.group)
            });
        match joins {
            Some(prev) => {
                for e in &mut self.undo[first_edit..] {
                    e.group = prev;
                }
                if let Some(rec) = self.groups.get_mut(&prev) {
                    rec.after = after;
                }
            }
            None => {
                self.groups.insert(
                    id,
                    GroupRec {
                        before,
                        after,
                        typing: self.group_typing,
                    },
                );
            }
        }
    }

    fn run_cursors(&mut self, f: &mut impl FnMut(&mut Self)) {
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
        let target = if line + 1 < self.line_count() {
            line + 1
        } else {
            line
        };
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
        let pair = closing_of(prev).is_some() && next == closing_of(prev);
        // Otherwise the whole cluster to the left: an emoji, a letter and its
        // accent, a CRLF, not half of one.
        let at = if pair {
            self.cursor - 1
        } else {
            self.grapheme_before(self.cursor)
        };
        let end = if pair { self.cursor + 1 } else { self.cursor };
        let old = self.slice(at..end);
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
        let old = self.slice(at..self.grapheme_after(at));
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

    /// Replace all: every `(start, end, new text)` in one edit, so one
    /// Cmd+Z puts the whole document back. `edits` is in order and does not
    /// overlap (`search::replace_edits`). The caret goes to the end of the
    /// last replacement, so a single replace steps on to the next match;
    /// returns how many were made.
    pub fn replace_ranges(&mut self, edits: &[(usize, usize, String)], now: f64) -> usize {
        let (Some(first), Some(last)) = (edits.first(), edits.last()) else {
            return 0;
        };
        let (start, end) = (first.0, last.1);
        let old = self.slice(start..end);
        let mut new = String::with_capacity(old.len());
        let mut at = start;
        for (a, b, text) in edits {
            new.push_str(&self.slice(at..*a));
            new.push_str(text);
            at = *b;
        }
        self.collapse_to_primary();
        let after = start + new.chars().count();
        self.apply(start, old, new, after, false, now);
        edits.len()
    }

    /// Undoes the last edit, or every edit of the last multi-cursor group
    /// and puts all its cursors back.
    pub fn undo(&mut self) {
        let Some(e) = self.undo.pop() else {
            return;
        };
        let group = e.group;
        self.undo_edit(e);
        if group != 0 {
            while let Some(e) = self.undo.pop_if(|e| e.group == group) {
                self.undo_edit(e);
            }
            if let Some(rec) = self.groups.get(&group).cloned() {
                self.restore_cursors(&rec.before);
            }
        }
    }

    fn undo_edit(&mut self, e: Edit) {
        let new_len = e.new.chars().count();
        self.text.remove(e.at..e.at + new_len);
        self.text.insert(e.at, &e.old);
        self.shift_folds(e.at, new_len, e.old.chars().count());
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
        let group = e.group;
        self.redo_edit(e);
        if group != 0 {
            while let Some(e) = self.redo.pop_if(|e| e.group == group) {
                self.redo_edit(e);
            }
            if let Some(rec) = self.groups.get(&group).cloned() {
                self.restore_cursors(&rec.after);
            }
        }
    }

    fn redo_edit(&mut self, e: Edit) {
        let old_len = e.old.chars().count();
        self.text.remove(e.at..e.at + old_len);
        self.text.insert(e.at, &e.new);
        self.shift_folds(e.at, old_len, e.new.chars().count());
        self.cursor = e.cursor_after.min(self.len_chars());
        self.anchor = None;
        self.goal_col = None;
        self.version += 1;
        self.undo.push(e);
    }

    /// The cursors of a group's before or after, the first as the primary.
    fn restore_cursors(&mut self, states: &[CursorState]) {
        let len = self.len_chars();
        let clamp = |c: &CursorState| CursorState {
            cursor: c.cursor.min(len),
            anchor: c.anchor.map(|a| a.min(len)),
            goal_col: None,
        };
        let Some((first, rest)) = states.split_first() else {
            return;
        };
        let first = clamp(first);
        self.cursor = first.cursor;
        self.anchor = first.anchor;
        self.goal_col = None;
        self.extra = rest.iter().map(clamp).collect();
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

    /// Three cursors at the start of three lines.
    fn three_cursors() -> Buffer {
        let mut b = Buffer::new("x\ny\nz");
        b.set_cursor(0);
        b.add_cursor_line(true);
        b.add_cursor_line(true);
        assert_eq!(b.cursor_count(), 3);
        b
    }

    fn type_at_every_cursor(b: &mut Buffer, s: &str, now: &mut f64) {
        for c in s.chars() {
            b.for_each_cursor(|b| b.type_char(c, *now));
            *now += 10.;
        }
    }

    #[test]
    fn a_multi_cursor_edit_is_one_undo_step_that_restores_every_cursor() {
        let mut b = three_cursors();
        let cursors = b.all_selections();
        b.for_each_cursor(|b| b.type_char('a', 0.));
        b.for_each_cursor(|b| b.move_line_end(false));
        let at_ends = b.all_selections();
        b.for_each_cursor(|b| b.type_char('!', 1000.));
        assert_eq!(b.text(), "ax!\nay!\naz!");
        b.undo();
        assert_eq!(b.text(), "ax\nay\naz", "the typing at all three");
        assert_eq!(b.all_selections(), at_ends);
        b.undo();
        assert_eq!(b.text(), "x\ny\nz");
        assert_eq!(b.all_selections(), cursors);
        b.redo();
        b.redo();
        assert_eq!(b.text(), "ax!\nay!\naz!");
        assert_eq!(b.cursor_count(), 3);
    }

    #[test]
    fn typing_at_every_cursor_in_one_run_is_one_step_and_a_pause_starts_another() {
        let mut b = three_cursors();
        let mut t = 0.;
        type_at_every_cursor(&mut b, "ab", &mut t);
        t += 1000.;
        type_at_every_cursor(&mut b, "c", &mut t);
        assert_eq!(b.text(), "abcx\nabcy\nabcz");
        b.undo();
        assert_eq!(b.text(), "abx\naby\nabz");
        b.undo();
        assert_eq!(b.text(), "x\ny\nz");
        assert!(!b.can_undo());
        b.redo();
        b.redo();
        assert_eq!(b.text(), "abcx\nabcy\nabcz");
    }

    #[test]
    fn moving_the_cursors_ends_the_typing_run() {
        let mut b = three_cursors();
        let mut t = 0.;
        type_at_every_cursor(&mut b, "a", &mut t);
        b.for_each_cursor(|b| b.move_left(false));
        type_at_every_cursor(&mut b, "b", &mut t);
        assert_eq!(b.text(), "bax\nbay\nbaz");
        b.undo();
        assert_eq!(b.text(), "ax\nay\naz");
    }

    #[test]
    fn a_single_cursor_edit_never_joins_a_group() {
        let mut b = three_cursors();
        let mut t = 0.;
        type_at_every_cursor(&mut b, "a", &mut t);
        b.collapse_to_primary();
        b.type_char('b', t);
        b.undo();
        assert_eq!(b.text(), "ax\nay\naz", "only the lone b");
        b.undo();
        assert_eq!(b.text(), "x\ny\nz");
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

    // After an opening bracket the next line is one level deeper, and a
    // closing bracket typed on an indent-only line steps back out (#602).
    #[test]
    fn enter_after_an_opening_bracket_indents_and_a_closer_dedents() {
        let mut b = Buffer::new("if x {");
        b.move_doc_end(false);
        b.newline(0.);
        assert_eq!(b.text(), "if x {\n  ");
        b.type_char('}', 1.);
        assert_eq!(b.text(), "if x {\n}", "the closer lines up with the opener");
        // Under existing indent, trailing spaces after the bracket, a tab.
        let mut b = Buffer::new("  call(  ");
        b.move_doc_end(false);
        b.newline(0.);
        assert_eq!(b.text(), "  call(  \n    ");
        let mut b = Buffer::new("\tfn f() {");
        b.move_doc_end(false);
        b.newline(0.);
        assert_eq!(
            b.text(),
            "\tfn f() {\n\t  ",
            "a tab keeps its tab, plus one level"
        );
        // Text after the caret moves down with the new indent.
        let mut b = Buffer::new("a [b]");
        b.set_cursor(3);
        b.newline(0.);
        assert_eq!(b.text(), "a [\n  b]");
        // No opener: as before. A selection is replaced, not indented.
        let mut b = Buffer::new("x = 1");
        b.move_doc_end(false);
        b.newline(0.);
        assert_eq!(b.text(), "x = 1\n");
        // A closer after text on the line is a plain character, and over an
        // existing closer it is a step over, never a dedent.
        let mut b = Buffer::new("  a ");
        b.move_doc_end(false);
        b.type_char(')', 0.);
        assert_eq!(b.text(), "  a )");
        let mut b = Buffer::new("f(\n  )");
        b.set_cursor(5);
        b.type_char(')', 0.);
        assert_eq!(b.text(), "f(\n  )", "stepped over");
        assert_eq!(b.cursor(), 6);
        // A closer with no indent to take is untouched.
        let mut b = Buffer::new("");
        b.type_char('}', 0.);
        assert_eq!(b.text(), "}");
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

    // A person's "one character" is a grapheme cluster (#602): the caret and
    // the deletions move by those, so an emoji is never split in half.
    #[test]
    fn the_caret_and_deletions_move_by_grapheme_cluster() {
        // Family (5 chars), a flag (2), a thumbs up with a skin tone (2), an
        // "e" with a combining acute (2), a CRLF (2), and plain letters.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let flag = "\u{1F1F9}\u{1F1F7}";
        let thumb = "\u{1F44D}\u{1F3FD}";
        let acute = "e\u{0301}";
        let text = format!("a{family}{flag}{thumb}{acute}\r\nb");
        let mut b = Buffer::new(&text);
        b.set_cursor(1);
        let mut stops = vec![b.cursor()];
        for _ in 0..6 {
            b.move_right(false);
            stops.push(b.cursor());
        }
        // a | family | flag | thumb | e+acute | CRLF | b
        assert_eq!(stops, vec![1, 6, 8, 10, 12, 14, 15]);
        for expect in [14, 12, 10, 8, 6, 1] {
            b.move_left(false);
            assert_eq!(b.cursor(), expect);
        }
        // A click or a vertical move into the middle of a cluster snaps back.
        b.set_cursor(3);
        assert_eq!(b.cursor(), 1);
        b.set_cursor(7);
        assert_eq!(b.cursor(), 6);
        // Shift+arrow selects whole clusters.
        b.set_cursor(1);
        b.move_right(true);
        assert_eq!(b.selection(), Some(1..6));
        // Backspace and Delete take a whole cluster.
        let mut b = Buffer::new(&text);
        b.set_cursor(6);
        b.backspace(0.);
        assert_eq!(b.text(), format!("a{flag}{thumb}{acute}\r\nb"));
        b.set_cursor(1);
        b.delete_forward(1.);
        assert_eq!(b.text(), format!("a{thumb}{acute}\r\nb"));
        let mut b = Buffer::new("x\r\ny");
        b.set_cursor(3);
        b.backspace(0.);
        assert_eq!(b.text(), "xy", "a CRLF is one line break, not two");
        // Plain text is unchanged, and the ends are still the ends.
        let mut b = Buffer::new("ab");
        b.move_left(false);
        assert_eq!(b.cursor(), 0);
        b.move_right(false);
        b.move_right(false);
        b.move_right(false);
        assert_eq!(b.cursor(), 2);
        b.backspace(0.);
        assert_eq!(b.text(), "a");
    }

    #[test]
    fn a_pair_still_goes_together_and_a_long_line_is_not_read_whole() {
        let mut b = Buffer::new("()");
        b.set_cursor(1);
        b.backspace(0.);
        assert_eq!(b.text(), "");
        // A megabyte line: a step reads a window, not the line.
        let long = "x".repeat(1_000_000);
        let mut b = Buffer::new(&long);
        b.set_cursor(500_000);
        let started = std::time::Instant::now();
        for _ in 0..200 {
            b.move_right(false);
        }
        assert_eq!(b.cursor(), 500_200);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn move_lines_carries_the_whole_selection() {
        let mut b = Buffer::new("a\nb\nc\nd");
        b.select_range(2..5); // "b\nc", anchor first
        b.move_lines(true, 0.);
        assert_eq!(b.text(), "b\nc\na\nd");
        assert_eq!(b.selection(), Some(0..3));
        b.move_lines(false, 1.);
        assert_eq!(b.text(), "a\nb\nc\nd");
        assert_eq!(b.selection(), Some(2..5));
        // Down onto the last line, which has no newline of its own.
        b.move_lines(false, 2.);
        assert_eq!(b.text(), "a\nd\nb\nc");
        assert_eq!(b.selection(), Some(4..7));
        b.move_lines(false, 3.);
        assert_eq!(b.text(), "a\nd\nb\nc");
    }

    #[test]
    fn move_lines_leaves_out_a_line_the_selection_only_reaches_the_start_of() {
        let mut b = Buffer::new("a\nb\nc");
        b.select_range(2..4); // "b\n", ends at column 0 of "c"
        b.move_lines(false, 0.);
        assert_eq!(b.text(), "a\nc\nb");
    }

    #[test]
    fn move_lines_keeps_a_backwards_selection_backwards() {
        let mut b = Buffer::new("a\nb\nc");
        b.set_cursor(3);
        b.select_to(2);
        b.move_lines(true, 0.);
        assert_eq!(b.text(), "b\na\nc");
        assert_eq!(b.anchor(), Some(1));
        assert_eq!(b.cursor(), 0);
    }

    #[test]
    fn duplicate_line_on_a_selection() {
        // Inside one line: the selected text is copied after itself.
        let mut b = Buffer::new("foo bar");
        b.select_range(0..3);
        b.duplicate_line(0.);
        assert_eq!(b.text(), "foofoo bar");
        assert_eq!(b.selection(), Some(3..6));
        // Across lines: the block is copied below and stays selected.
        let mut b = Buffer::new("a\nb\nc");
        b.select_range(0..3);
        b.duplicate_line(0.);
        assert_eq!(b.text(), "a\nb\na\nb\nc");
        assert_eq!(b.selection(), Some(4..7));
    }

    #[test]
    fn copy_lines_above_keeps_the_selection_on_the_upper_copy() {
        let mut b = Buffer::new("a\nb");
        b.set_cursor(2);
        b.copy_lines(true, 0.);
        assert_eq!(b.text(), "a\nb\nb");
        assert_eq!(b.cursor(), 2);
        b.copy_lines(false, 1.);
        assert_eq!(b.text(), "a\nb\nb\nb");
        assert_eq!(b.cursor(), 4);
    }

    #[test]
    fn replace_ranges_is_one_undo_step() {
        let mut b = Buffer::new("a-é-a-b");
        let n = b.replace_ranges(&[(0, 1, "xx".into()), (4, 5, "".into())], 0.);
        assert_eq!(n, 2);
        assert_eq!(b.text(), "xx-é--b");
        assert_eq!(b.cursor(), 5); // the end of the last replacement
        b.undo();
        assert_eq!(b.text(), "a-é-a-b");
        assert!(!b.can_undo());
        assert_eq!(b.replace_ranges(&[], 1.), 0);
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
    fn a_quote_or_bracket_over_a_selection_wraps_it_and_keeps_it_selected() {
        for (typed, wrapped) in [
            ('\'', "a 'foo' b"),
            ('"', "a \"foo\" b"),
            ('(', "a (foo) b"),
            ('[', "a [foo] b"),
            ('{', "a {foo} b"),
            ('`', "a `foo` b"),
        ] {
            let mut b = Buffer::new("a foo b");
            b.select_range(2..5);
            b.type_char(typed, 0.);
            assert_eq!(b.text(), wrapped, "{typed}");
            assert_eq!(b.selected_text().as_deref(), Some("foo"), "{typed}");
        }
        // A second press wraps again, around the same word.
        let mut b = Buffer::new("foo");
        b.select_all();
        b.type_char('(', 0.);
        b.type_char('[', 1.);
        assert_eq!(b.text(), "([foo])");
        // One undo takes the wrap back.
        b.undo();
        assert_eq!(b.text(), "(foo)");
        // A closing half is not a wrap: it replaces, as before.
        let mut b = Buffer::new("a foo b");
        b.select_range(2..5);
        b.type_char(')', 0.);
        assert_eq!(b.text(), "a ) b");
        // A backwards selection (anchor after the caret) wraps the same.
        let mut b = Buffer::new("a foo b");
        b.set_cursor(5);
        b.select_to(2);
        b.type_char('"', 0.);
        assert_eq!(b.text(), "a \"foo\" b");
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
        assert_eq!(
            b.all_selections(),
            vec![(3, Some(0)), (11, Some(8)), (19, Some(16))]
        );
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
        assert_eq!(
            b.all_selections(),
            vec![(3, Some(0)), (11, Some(8)), (19, Some(16))]
        );
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
    fn split_into_lines_gives_one_selection_per_line_of_the_original() {
        // Cmd+A then Cmd+Shift+L: the whole buffer, one cursor per line,
        // each selecting that whole line (Ekin's own most-used case).
        let mut b = Buffer::new("one\ntwo\nthree");
        b.select_all();
        b.split_into_lines();
        assert_eq!(b.cursor_count(), 3);
        assert_eq!(
            b.all_selections(),
            vec![(3, Some(0)), (7, Some(4)), (13, Some(8))]
        );
        // A partial multi-line selection keeps only each line's own slice.
        let mut b = Buffer::new("one\ntwo\nthree");
        b.select_range(1..10); // "ne\ntwo\nth"
        b.split_into_lines();
        assert_eq!(
            b.all_selections(),
            vec![(3, Some(1)), (7, Some(4)), (10, Some(8))]
        );
        // A selection that never leaves one line is a no-op: nothing to split.
        let mut b = Buffer::new("one\ntwo");
        b.select_range(0..2);
        b.split_into_lines();
        assert_eq!(b.cursor_count(), 1);
        assert_eq!(b.selection(), Some(0..2));
        // No selection at all is a no-op too.
        let mut b = Buffer::new("one\ntwo");
        b.set_cursor(1);
        b.split_into_lines();
        assert_eq!(b.cursor_count(), 1);
        assert_eq!(b.selection(), None);
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

    const CODE: &str = "fn a() {\n    x();\n\n    y();\n}\nfn b() {\n    z();\n}";

    #[test]
    fn folding_hides_the_block_under_a_line_and_lands_the_caret_on_its_header() {
        let mut b = Buffer::new(CODE);
        b.set_cursor(2); // in "fn a() {"
        b.fold_at_cursor();
        assert!(b.is_fold_header(0));
        assert!(b.is_line_hidden(1) && b.is_line_hidden(2) && b.is_line_hidden(3));
        assert!(
            !b.is_line_hidden(4),
            "the closing brace is at the header's indent"
        );
        assert!(!b.is_line_hidden(5));
        // From inside the block, the block folds and the caret goes up to its header.
        let mut b = Buffer::new(CODE);
        b.set_cursor(b.line_start(3) + 4); // in "y();"
        b.fold_at_cursor();
        assert!(b.is_fold_header(0));
        assert_eq!(b.line_of(b.cursor()), 0);
        // A line with nothing under it does not fold.
        let mut b = Buffer::new("a\nb");
        b.fold_at_cursor();
        assert!(!b.has_folds());
    }

    // A diff's unchanged stretches fold by their own ranges, not by indent:
    // the placeholder line heads a fold from the start, a click or the fold
    // keys open it and fold it again, and nothing else can fold.
    #[test]
    fn fixed_folds_replace_the_indent_rule() {
        // Line 0 is a placeholder over lines 1-3; lines 4-5 are indented code.
        let mut b = Buffer::new("p\na\nb\nc\nfn x() {\n    y();\n}");
        b.set_fixed_folds(&[(0, 3)]);
        assert!(b.is_fold_header(0));
        assert!(b.is_line_hidden(1) && b.is_line_hidden(3));
        assert!(!b.is_line_hidden(4));
        // The click on the header opens it, the next one closes it again.
        b.toggle_fold_line(0);
        assert!(!b.has_folds());
        b.toggle_fold_line(0);
        assert!(b.is_line_hidden(2));
        // Indented code does not fold: it is not a fixed block.
        b.toggle_fold_line(4);
        assert!(!b.is_fold_header(4));
        b.set_cursor(b.line_start(4));
        b.fold_at_cursor();
        assert!(!b.is_fold_header(4));
        // From inside an opened block, Cmd+Alt+[ folds it again.
        b.unfold_all();
        b.set_cursor(b.line_start(2));
        b.fold_at_cursor();
        assert!(b.is_fold_header(0) && b.is_line_hidden(2));
        assert_eq!(b.line_of(b.cursor()), 0, "the caret goes up to the header");
        b.unfold_all();
        b.fold_all();
        assert!(b.is_fold_header(0));
        // Replacing the text forgets them.
        b.set_text("q\nr", 0.);
        b.toggle_fold_line(0);
        assert!(!b.has_folds());
    }

    #[test]
    fn vertical_movement_steps_over_folded_lines() {
        let mut b = Buffer::new(CODE);
        b.set_cursor(0);
        b.fold_at_cursor();
        b.move_down(false);
        assert_eq!(
            b.line_of(b.cursor()),
            4,
            "from the header to the brace after the block"
        );
        b.move_up(false);
        assert_eq!(b.line_of(b.cursor()), 0);
    }

    #[test]
    fn folds_follow_edits_above_them_and_die_when_touched() {
        let mut b = Buffer::new(CODE);
        b.set_cursor(0);
        b.fold_at_cursor();
        // A line added above: the fold now heads line 1 and hides the same lines.
        b.set_cursor(0);
        b.insert("// new\n", 0.);
        assert!(b.is_fold_header(1));
        assert!(b.is_line_hidden(2) && b.is_line_hidden(4));
        assert!(!b.is_line_hidden(5));
        // Undo puts the text back and the fold with it.
        b.undo();
        assert!(b.is_fold_header(0) && b.is_line_hidden(1));
        // An edit across the hidden text drops the fold.
        b.replace_range(9..20, "", 1.);
        assert!(!b.has_folds());
    }

    #[test]
    fn unfold_reveal_and_the_all_versions() {
        let mut b = Buffer::new(CODE);
        b.fold_all();
        assert!(b.is_fold_header(0) && b.is_fold_header(5));
        assert!(b.is_line_hidden(1) && b.is_line_hidden(6));
        // The caret ends up on a header, not inside a block.
        assert!(!b.is_line_hidden(b.line_of(b.cursor())));
        b.unfold_all();
        assert!(!b.has_folds());
        // Opening at the caret: only the fold it heads.
        b.fold_all();
        b.set_cursor(0);
        b.unfold_at_cursor();
        assert!(!b.is_fold_header(0) && b.is_fold_header(5));
        // A caret that arrives inside a fold opens it.
        b.fold_all();
        b.set_cursor(b.line_start(6) + 2);
        b.reveal_cursors();
        assert!(!b.is_fold_header(5));
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
