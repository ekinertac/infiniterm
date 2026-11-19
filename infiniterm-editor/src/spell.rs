//! Spell checking's logic (#850): which buffers are checked, which parts of
//! a line are prose, and the misspelled words found so far, kept per line so
//! an edit re-asks only the lines it changed. The asking itself is the
//! platform's (`infiniterm-ui/src/spelling.rs`, macOS's `NSSpellChecker`);
//! this file hands it one line at a time as a closure and keeps the answers.
//!
//! Checked: plain text and Markdown files, an untitled buffer (a message
//! being written), and an in-place editor (`ift <file>` from a terminal, the
//! card `EDITOR=ift` opens for Claude Code's Ctrl+G or `git commit`) unless
//! its file is code. Code files are not checked at all.
//!
//! What is not prose is blanked before a line is asked about, char for char,
//! so the columns of what comes back are the line's own: Markdown's fenced
//! blocks and inline code, and words that are names rather than words (a
//! URL, a path, an email address, `snake_case`, `camelCase`).
//!
//! Called by `infiniterm-ui/src/editor_body.rs` from its idle step and its
//! paint. Pure: lines in, char ranges out; the closure is the only way out.
//! Columns are CHARS, as the editor places everything.
use crate::language::Language;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;

/// The buffer a pass was run over: its version and the editor's load count,
/// the key every other per-text cache in the editor uses.
pub type Key = (u64, u64);

/// Whether a buffer is spell checked: `path` is `None` for an untitled one,
/// `cover` says it is the in-place editor over a terminal.
pub fn checks(path: Option<&str>, cover: bool) -> bool {
    let Some(path) = path else {
        return true;
    };
    if Language::is_prose(path) {
        return true;
    }
    // A commit message or a crontab has no extension and no grammar; a
    // source file opened in place is still code.
    cover && matches!(Language::for_path(path), None | Some(Language::Markdown))
}

/// Whether Markdown's code is left out of the check.
pub fn is_markdown(path: Option<&str>) -> bool {
    path.is_some_and(|p| Language::for_path(p) == Some(Language::Markdown))
}

/// What of each line is prose: the line with everything else replaced by
/// spaces, one space per char. `markdown` blanks fenced blocks (the fences
/// too) and inline code.
pub fn prose_lines(lines: &[String], markdown: bool) -> Vec<String> {
    let mut fence: Option<String> = None;
    lines
        .iter()
        .map(|line| prose_line(line, markdown, &mut fence))
        .collect()
}

/// One line of `prose_lines`; `fence` is the open fenced block's mark,
/// carried from line to line.
fn prose_line(line: &str, markdown: bool, fence: &mut Option<String>) -> String {
    if markdown {
        let mark: String = line
            .trim_start()
            .chars()
            .take_while(|c| *c == '`' || *c == '~')
            .collect();
        let is_fence = mark.len() >= 3 && !(mark.contains('`') && mark.contains('~'));
        match fence {
            Some(open) => {
                // A fence closes on the same character, at least as long.
                if is_fence && mark.starts_with(open.as_str()) {
                    *fence = None;
                }
                return blank(line);
            }
            None if is_fence => {
                *fence = Some(mark);
                return blank(line);
            }
            None => {}
        }
    }
    let mut chars: Vec<char> = line.chars().collect();
    if markdown {
        blank_inline_code(&mut chars);
    }
    blank_names(&mut chars);
    chars.into_iter().collect()
}

fn blank(line: &str) -> String {
    " ".repeat(line.chars().count())
}

/// Inline code: a run of backticks to the next run of the same length.
/// An unclosed run is left alone, as Markdown reads it as text.
fn blank_inline_code(chars: &mut [char]) {
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '`' {
            i += 1;
            continue;
        }
        let open = chars[i..].iter().take_while(|c| **c == '`').count();
        let mut j = i + open;
        let mut close = None;
        while j < chars.len() {
            if chars[j] == '`' {
                let run = chars[j..].iter().take_while(|c| **c == '`').count();
                if run == open {
                    close = Some(j + run);
                    break;
                }
                j += run;
            } else {
                j += 1;
            }
        }
        match close {
            Some(end) => {
                chars[i..end].fill(' ');
                i = end;
            }
            None => i += open,
        }
    }
}

/// Words that are names: a whitespace-separated token that looks like a
/// URL, a path, an email address or an identifier is blanked whole.
fn blank_names(chars: &mut [char]) {
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let end = chars[i..]
            .iter()
            .position(|c| c.is_whitespace())
            .map_or(chars.len(), |n| i + n);
        if is_name(&chars[i..end]) {
            chars[i..end].fill(' ');
        }
        i = end;
    }
}

fn is_name(token: &[char]) -> bool {
    let has = |c: char| token.contains(&c);
    if has('/') || has('\\') || has('_') || has('@') || has('`') {
        return true;
    }
    let s: String = token.iter().collect();
    if s.starts_with("www.") {
        return true;
    }
    // A capital after a small letter: `camelCase`, `EditorBody`, `macOS`.
    token
        .windows(2)
        .any(|w| w[0].is_lowercase() && w[1].is_uppercase())
}

/// A count of UTF-16 units into `text` as chars: what `NSString` measures
/// in, back to what the editor does.
pub fn utf16_to_char(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (i, c) in text.chars().enumerate() {
        if seen >= units {
            return i;
        }
        seen += c.len_utf16();
    }
    text.chars().count()
}

/// The other way: `chars` chars into `text` as UTF-16 units.
pub fn char_to_utf16(text: &str, chars: usize) -> usize {
    text.chars().take(chars).map(char::len_utf16).sum()
}

fn hash_of(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// How much one call of `Spell::run` may do: lines handed to the checker
/// (the slow part), and lines walked in all (a large file's cached lines).
#[derive(Clone, Copy)]
pub struct Budget {
    pub asks: usize,
    pub walk: usize,
}

/// A pass in flight: where it is, and what it found so far.
struct Pass {
    key: Key,
    next: usize,
    fence: Option<String>,
    lines: Vec<(u64, Vec<Range<usize>>)>,
    used: HashMap<String, Vec<Range<usize>>>,
}

/// The misspelled words of one buffer.
#[derive(Default)]
pub struct Spell {
    /// Prose line to its misspelled char ranges: the answers so far. A
    /// finished pass keeps only what it used, so the map never outgrows the
    /// text.
    known: HashMap<String, Vec<Range<usize>>>,
    pass: Option<Pass>,
    /// The last finished pass, per line: a hash of the line's text and its
    /// misspelled ranges. A line whose text no longer matches shows none
    /// until the next pass, so an edit never leaves an underline misplaced.
    lines: Vec<(u64, Vec<Range<usize>>)>,
    done: Option<Key>,
    /// The checker's generation (`spelling::generation`): Learn and Ignore
    /// change what is misspelled everywhere, so every answer is dropped.
    generation: u64,
}

impl Spell {
    /// A pass over the `count` lines `line_of` gives, asking `ask` (prose
    /// text in, misspelled char ranges out) about the ones it has no answer
    /// for, within `budget`. True once every line is answered and the result
    /// is in; false means call again: the pass resumes where it stopped, or
    /// starts over if the text changed, keeping every answer either way.
    // Each argument is a different input of the pass; bundling them into a
    // struct only for the count would hide which one a caller forgot.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self,
        key: Key,
        generation: u64,
        count: usize,
        line_of: &dyn Fn(usize) -> String,
        markdown: bool,
        budget: Budget,
        ask: &mut dyn FnMut(&str) -> Vec<Range<usize>>,
    ) -> bool {
        if generation != self.generation {
            self.clear();
            self.generation = generation;
        }
        if self.done == Some(key) {
            return true;
        }
        if self.pass.as_ref().is_some_and(|p| p.key != key) {
            let old = self.pass.take().unwrap();
            self.known.extend(old.used);
        }
        let pass = self.pass.get_or_insert_with(|| Pass {
            key,
            next: 0,
            fence: None,
            lines: Vec::with_capacity(count),
            used: HashMap::new(),
        });
        let (mut asked, mut walked) = (0, 0);
        while pass.next < count {
            if walked >= budget.walk {
                return false;
            }
            let raw = line_of(pass.next);
            let mut fence = pass.fence.clone();
            let p = prose_line(&raw, markdown, &mut fence);
            let found = if let Some(f) = pass.used.get(&p).or_else(|| self.known.get(&p)) {
                f.clone()
            } else if !p.chars().any(char::is_alphabetic) {
                vec![]
            } else if asked < budget.asks {
                asked += 1;
                ask(&p)
            } else {
                return false;
            };
            pass.fence = fence;
            pass.lines.push((hash_of(&raw), found.clone()));
            pass.used.insert(p, found);
            pass.next += 1;
            walked += 1;
        }
        let pass = self.pass.take().unwrap();
        self.known = pass.used;
        self.lines = pass.lines;
        self.done = Some(key);
        true
    }

    /// Whether the last pass was over this buffer as it is now, with the
    /// checker as it is now.
    pub fn is_done(&self, key: Key, generation: u64) -> bool {
        self.done == Some(key) && self.generation == generation
    }

    /// Drops every answer, for a setting turned off or a file switched.
    pub fn clear(&mut self) {
        self.known.clear();
        self.pass = None;
        self.lines.clear();
        self.done = None;
    }

    /// The misspelled char ranges of line `line`, whose text is `text` now:
    /// none when the line changed since the last pass.
    pub fn ranges(&self, line: usize, text: &str) -> &[Range<usize>] {
        match self.lines.get(line) {
            Some((h, f)) if *h == hash_of(text) => f,
            _ => &[],
        }
    }

    /// The misspelled word at column `col` of `line` (the caret just after
    /// it counts), as chars of the line.
    pub fn word_at(&self, line: usize, text: &str, col: usize) -> Option<Range<usize>> {
        self.ranges(line, text)
            .iter()
            .find(|r| r.start <= col && col <= r.end)
            .cloned()
    }

    /// The first misspelled word after column `col` of `line`, wrapping past
    /// the end to the top: its line and its char range in that line. Lines
    /// that changed since the last pass are skipped.
    pub fn next_after(
        &self,
        line: usize,
        col: usize,
        text_of: &dyn Fn(usize) -> String,
    ) -> Option<(usize, Range<usize>)> {
        let n = self.lines.len();
        (0..=n).find_map(|step| {
            let l = (line + step) % n.max(1);
            let found = self.ranges(l, &text_of(l));
            let pick = if step == 0 {
                found.iter().find(|r| r.start > col)
            } else if step == n {
                // Back on the caret's line: a word before the caret.
                found.iter().find(|r| r.start <= col)
            } else {
                found.first()
            };
            pick.map(|r| (l, r.clone()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &[&str]) -> Vec<String> {
        s.iter().map(|l| l.to_string()).collect()
    }

    /// One misspelled range, as `Spell::ranges` answers it.
    fn one(start: usize, end: usize) -> &'static [Range<usize>] {
        Box::leak(Box::new([Range { start, end }]))
    }

    /// A checker that knows one misspelling, "teh", and counts the asks.
    fn teh(asks: &mut usize) -> impl FnMut(&str) -> Vec<Range<usize>> + '_ {
        move |text: &str| {
            *asks += 1;
            let chars: Vec<char> = text.chars().collect();
            let mut out = vec![];
            for i in 0..chars.len().saturating_sub(2) {
                let word: String = chars[i..i + 3].iter().collect();
                let left = i == 0 || !chars[i - 1].is_alphabetic();
                let right = chars.get(i + 3).is_none_or(|c| !c.is_alphabetic());
                if word == "teh" && left && right {
                    out.push(i..i + 3);
                }
            }
            out
        }
    }

    #[test]
    fn prose_and_markdown_are_checked_code_is_not() {
        assert!(checks(Some("notes.txt"), false));
        assert!(checks(Some("README.md"), false));
        assert!(checks(None, false), "an untitled buffer is a message");
        assert!(!checks(Some("main.rs"), false));
        assert!(!checks(Some("COMMIT_EDITMSG"), false));
    }

    // Claude Code's Ctrl+G writes `claude-prompt-<id>.md`, prose already; a
    // commit message has no extension and is checked only in place.
    #[test]
    fn the_in_place_editor_is_checked_unless_its_file_is_code() {
        assert!(checks(Some("/tmp/claude-prompt-1a2b.md"), true));
        assert!(checks(Some("/repo/.git/COMMIT_EDITMSG"), true));
        assert!(!checks(Some("/repo/src/main.rs"), true));
    }

    #[test]
    fn fenced_blocks_and_inline_code_are_blanked_in_markdown() {
        let got = prose_lines(
            &lines(&["a `teh` b", "```rust", "teh code", "```", "teh"]),
            true,
        );
        assert_eq!(got[0], "a       b");
        assert!(got[1..4].iter().all(|l| l.trim().is_empty()));
        assert_eq!(got[4], "teh");
        // Columns hold: every line keeps its length in chars.
        assert_eq!(got[2].chars().count(), "teh code".chars().count());
    }

    #[test]
    fn a_fence_closes_only_on_its_own_mark() {
        let got = prose_lines(&lines(&["~~~~", "```", "x", "~~~~", "y"]), true);
        assert!(got[2].trim().is_empty(), "``` inside ~~~~ does not close it");
        assert_eq!(got[4], "y");
    }

    #[test]
    fn plain_text_keeps_backticks_and_fences() {
        let got = prose_lines(&lines(&["```", "a teh"]), false);
        assert_eq!(got[1], "a teh");
    }

    #[test]
    fn an_unclosed_backtick_is_text() {
        let got = prose_lines(&lines(&["it`s teh"]), true);
        assert_eq!(got[0], "     teh", "the token with the backtick is a name");
        let got = prose_lines(&lines(&["`` a ` b `` teh"]), true);
        assert_eq!(got[0], "            teh");
    }

    #[test]
    fn names_are_not_words() {
        let names = "https://x.io src/main.rs a@b.io snake_case camelCase";
        let got = prose_lines(&lines(&[&format!("see {names} teh")]), false);
        assert_eq!(got[0], format!("see {} teh", " ".repeat(names.len())));
    }

    #[test]
    fn utf16_offsets_turn_into_chars_and_back() {
        let s = "a\u{1F600}b"; // the emoji is two UTF-16 units, one char
        assert_eq!(utf16_to_char(s, 0), 0);
        assert_eq!(utf16_to_char(s, 1), 1);
        assert_eq!(utf16_to_char(s, 3), 2);
        assert_eq!(utf16_to_char(s, 4), 3);
        assert_eq!(char_to_utf16(s, 2), 3);
        assert_eq!(char_to_utf16(s, 3), 4);
    }

    /// One `Spell::run` over `text` with room for `asks` checker calls.
    fn run(s: &mut Spell, key: u64, generation: u64, text: &[String], asks: usize, n: &mut usize) -> bool {
        let budget = Budget { asks, walk: 1000 };
        s.run(
            (key, 0),
            generation,
            text.len(),
            &|l| text[l].clone(),
            false,
            budget,
            &mut teh(n),
        )
    }

    #[test]
    fn a_pass_finds_the_misspelled_words_per_line() {
        let mut s = Spell::default();
        let text = lines(&["teh cat", "fine", "and teh"]);
        let mut asks = 0;
        assert!(run(&mut s, 1, 0, &text, 100, &mut asks));
        assert_eq!(s.ranges(0, "teh cat"), one(0, 3));
        assert!(s.ranges(1, "fine").is_empty());
        assert_eq!(s.ranges(2, "and teh"), one(4, 7));
        assert_eq!(asks, 3);
    }

    // An edit re-asks only the lines it changed; the rest come from the
    // answers already given.
    #[test]
    fn an_edit_asks_again_only_for_changed_lines() {
        let mut s = Spell::default();
        let mut asks = 0;
        run(&mut s, 1, 0, &lines(&["teh", "b"]), 100, &mut asks);
        let mut asks = 0;
        run(&mut s, 2, 0, &lines(&["teh", "b", "c teh"]), 100, &mut asks);
        assert_eq!(asks, 1);
        assert_eq!(s.ranges(2, "c teh"), one(2, 5));
    }

    #[test]
    fn a_changed_line_shows_nothing_until_the_next_pass() {
        let mut s = Spell::default();
        let mut asks = 0;
        run(&mut s, 1, 0, &lines(&["teh"]), 100, &mut asks);
        assert!(s.ranges(0, "xteh").is_empty());
    }

    #[test]
    fn the_budget_spreads_a_pass_over_several_calls() {
        let mut s = Spell::default();
        let text = lines(&["a teh", "b teh", "c teh"]);
        let mut asks = 0;
        assert!(!run(&mut s, 1, 0, &text, 2, &mut asks));
        assert!(!s.is_done((1, 0), 0));
        assert!(run(&mut s, 1, 0, &text, 2, &mut asks));
        assert_eq!(asks, 3, "the first two answers were kept");
        assert_eq!(s.ranges(2, "c teh"), one(2, 5));
    }

    // A large file's cached lines are walked a slice per call, not at once.
    #[test]
    fn the_walk_budget_resumes_where_it_stopped() {
        let mut s = Spell::default();
        let text = lines(&["a", "b", "c teh", "d", "e"]);
        let mut asks = 0;
        let mut step = || {
            s.run(
                (1, 0),
                0,
                text.len(),
                &|l| text[l].clone(),
                false,
                Budget { asks: 10, walk: 2 },
                &mut teh(&mut asks),
            )
        };
        assert!(!step());
        assert!(!step());
        assert!(step());
        assert_eq!(asks, 5, "each line asked once");
        assert_eq!(s.ranges(2, "c teh"), one(2, 5));
    }

    // Typing during a pass starts it over without asking again for what the
    // abandoned pass already learned.
    #[test]
    fn a_pass_cut_short_by_an_edit_keeps_its_answers() {
        let mut s = Spell::default();
        let mut asks = 0;
        assert!(!run(&mut s, 1, 0, &lines(&["a teh", "b teh", "c"]), 2, &mut asks));
        let mut asks = 0;
        assert!(run(&mut s, 2, 0, &lines(&["a teh", "b teh", "c!"]), 10, &mut asks));
        assert_eq!(asks, 1, "only the changed line");
    }

    #[test]
    fn a_new_generation_drops_every_answer() {
        let mut s = Spell::default();
        let mut asks = 0;
        run(&mut s, 1, 0, &lines(&["teh"]), 100, &mut asks);
        assert!(!s.is_done((1, 0), 1));
        run(&mut s, 1, 1, &lines(&["teh"]), 100, &mut asks);
        assert_eq!(asks, 2);
    }

    #[test]
    fn the_word_at_the_caret_includes_its_end() {
        let mut s = Spell::default();
        let mut asks = 0;
        run(&mut s, 1, 0, &lines(&["a teh b"]), 100, &mut asks);
        assert_eq!(s.word_at(0, "a teh b", 2), Some(2..5));
        assert_eq!(s.word_at(0, "a teh b", 5), Some(2..5));
        assert_eq!(s.word_at(0, "a teh b", 6), None);
    }

    #[test]
    fn next_goes_forward_and_wraps_to_the_top() {
        let mut s = Spell::default();
        let text = lines(&["teh a teh", "b", "teh"]);
        let mut asks = 0;
        run(&mut s, 1, 0, &text, 100, &mut asks);
        let of = |l: usize| text[l].clone();
        assert_eq!(s.next_after(0, 0, &of), Some((0, 6..9)));
        assert_eq!(s.next_after(0, 6, &of), Some((2, 0..3)));
        assert_eq!(s.next_after(2, 0, &of), Some((0, 0..3)), "wraps");
        // One misspelling, the caret on it: it is found again.
        let one = lines(&["teh"]);
        let mut s = Spell::default();
        run(&mut s, 1, 0, &one, 100, &mut asks);
        assert_eq!(s.next_after(0, 0, &|_| "teh".into()), Some((0, 0..3)));
    }
}
