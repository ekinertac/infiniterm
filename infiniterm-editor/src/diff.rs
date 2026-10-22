//! A diff as ONE text for the editor to show, plus the git gutter's marks.
//!
//! The diff card is an editor in a read-only diff mode (2026-10-20, Ekin:
//! "let's just make them read only editors"), so what the old card drew from
//! its own rows is now a buffer: `merged` weaves HEAD's text and the working
//! tree's into a single text in file order (context, removed and added lines,
//! with NO `+`/`-` prefixes, so tree-sitter reads real code) and says, per
//! line, what kind it is and which line numbers it has on each side. The
//! editor paints the washes and the two number columns from that, and folds
//! the unchanged stretches (`MARGIN`, `MIN_COLLAPSE`: the merge view's
//! `collapseUnchanged: { margin: 3, minSize: 4 }`, as before).
//!
//! An unchanged stretch becomes a PLACEHOLDER line ("⋯ N unchanged lines")
//! followed by the stretch's lines, folded under it: a fold hides what comes
//! after its header line, and a stretch at the top of a file has no line above
//! it to be the header. The placeholder stays when the fold is opened, as a
//! rule between the context and the lines it was hiding.
//!
//! `merged_spans` maps syntax spans onto the merged text. The two sides are
//! highlighted SEPARATELY (HEAD's text for removed lines, the working tree's
//! for the rest) and the spans moved onto their lines, because a removed
//! `fn a() {` beside an added `fn a(x) {` would unbalance a parse of the
//! woven text and colour everything below wrongly.
//!
//! Pure over strings, so every rule here is tested without git or a window.
//! Called by `infiniterm-ui/src/editor_body/diff.rs`. `gutter_marks` is the
//! ordinary editor's git gutter and lives here because it diffs the same way.
use crate::highlight::Span;
use similar::{ChangeTag, TextDiff};

/// Context kept around a change, and the shortest unchanged run that
/// collapses: the reference's `collapseUnchanged: { margin: 3, minSize: 4 }`.
pub const MARGIN: usize = 3;
pub const MIN_COLLAPSE: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// In both texts.
    Context,
    /// Only in the working tree.
    Added,
    /// Only in HEAD: exists in the merged text and nowhere in the file.
    Removed,
    /// "⋯ N unchanged lines": the header of a fold, in neither text.
    Placeholder,
}

/// What the editor needs to know about one line of the merged text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// The line's number in HEAD's text (1-based), `None` for an added line.
    pub old: Option<usize>,
    /// The line's number in the working tree's text, `None` for a removed line.
    pub new: Option<usize>,
    /// Where the line starts, in bytes, in the text it came from: HEAD's for
    /// a removed line, the working tree's for the rest. For `merged_spans`.
    src_start: usize,
}

/// The merged text and what the editor decorates it with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Merged {
    /// Lines joined by `\n`, no trailing newline, so the editor's line count
    /// is `lines.len()` (a text ending in a newline has one empty line more).
    /// Empty when both texts are.
    pub text: String,
    /// One per line of `text`.
    pub lines: Vec<DiffLine>,
    /// Folds as (header line, last hidden line), 0-based lines of `text`:
    /// each header is a placeholder.
    pub folds: Vec<(usize, usize)>,
}

/// `original` (HEAD) against `current` (the working tree). Lines are compared
/// whole, endings included, as the merge view compared them before its
/// character-level marks; a line is shown without its ending.
pub fn merged(original: &str, current: &str) -> Merged {
    let diff = TextDiff::from_lines(original, current);
    let (mut old_n, mut new_n) = (0usize, 0usize);
    let (mut old_at, mut new_at) = (0usize, 0usize);
    let mut rows: Vec<(DiffLine, String)> = vec![];
    for change in diff.iter_all_changes() {
        let value = change.value();
        let text = value.trim_end_matches(['\n', '\r']).to_string();
        let line = match change.tag() {
            ChangeTag::Equal => {
                old_n += 1;
                new_n += 1;
                DiffLine {
                    kind: LineKind::Context,
                    old: Some(old_n),
                    new: Some(new_n),
                    src_start: new_at,
                }
            }
            ChangeTag::Insert => {
                new_n += 1;
                DiffLine {
                    kind: LineKind::Added,
                    old: None,
                    new: Some(new_n),
                    src_start: new_at,
                }
            }
            ChangeTag::Delete => {
                old_n += 1;
                DiffLine {
                    kind: LineKind::Removed,
                    old: Some(old_n),
                    new: None,
                    src_start: old_at,
                }
            }
        };
        if change.tag() != ChangeTag::Insert {
            old_at += value.len();
        }
        if change.tag() != ChangeTag::Delete {
            new_at += value.len();
        }
        rows.push((line, text));
    }

    let mut lines: Vec<DiffLine> = vec![];
    let mut texts: Vec<String> = vec![];
    let mut folds: Vec<(usize, usize)> = vec![];
    let n = rows.len();
    let mut i = 0;
    while i < n {
        if rows[i].0.kind != LineKind::Context {
            lines.push(rows[i].0);
            texts.push(rows[i].1.clone());
            i += 1;
            continue;
        }
        let start = i;
        while i < n && rows[i].0.kind == LineKind::Context {
            i += 1;
        }
        let run = i - start;
        // A run at the start or the end keeps context only on the side that
        // has a change to be context for.
        let lead = if start == 0 { 0 } else { MARGIN };
        let trail = if i == n { 0 } else { MARGIN };
        let keep = |lines: &mut Vec<DiffLine>, texts: &mut Vec<String>, from: usize, to: usize| {
            for (line, text) in &rows[from..to] {
                lines.push(*line);
                texts.push(text.clone());
            }
        };
        if run >= lead + trail + MIN_COLLAPSE {
            keep(&mut lines, &mut texts, start, start + lead);
            let hidden = run - lead - trail;
            let header = lines.len();
            lines.push(DiffLine {
                kind: LineKind::Placeholder,
                old: None,
                new: None,
                src_start: 0,
            });
            texts.push(format!("⋯ {hidden} unchanged lines"));
            keep(&mut lines, &mut texts, start + lead, i - trail);
            folds.push((header, lines.len() - 1));
            keep(&mut lines, &mut texts, i - trail, i);
        } else {
            keep(&mut lines, &mut texts, start, i);
        }
    }
    Merged {
        text: texts.join("\n"),
        lines,
        folds,
    }
}

/// The syntax spans of the merged text. `old_spans` were computed on HEAD's
/// text and `new_spans` on the working tree's (both sorted by start, as
/// `Highlighting::spans` returns them); a removed line takes the first, every
/// other line the second, each clipped to its line and moved to where the
/// line sits in the merged text. A placeholder has none.
pub fn merged_spans(merged: &Merged, old_spans: &[Span], new_spans: &[Span]) -> Vec<Span> {
    let mut out = vec![];
    let mut at = 0usize; // byte where the current line starts in the merged text
    for (line, text) in merged.lines.iter().zip(merged.text.split('\n')) {
        let (spans, from) = match line.kind {
            LineKind::Placeholder => {
                at += text.len() + 1;
                continue;
            }
            LineKind::Removed => (old_spans, line.src_start),
            LineKind::Context | LineKind::Added => (new_spans, line.src_start),
        };
        let to = from + text.len();
        // The first span that does not end before the line starts.
        let first = spans.partition_point(|s| s.end <= from);
        for s in spans[first..].iter().take_while(|s| s.start < to) {
            out.push(Span {
                start: at + s.start.max(from) - from,
                end: at + s.end.min(to) - from,
                capture: s.capture,
            });
        }
        at += text.len() + 1;
    }
    out
}

/// A line's git-gutter status, one of `gutter_marks`'s per-line marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GutterMark {
    Clean,
    Added,
    Modified,
}

/// Per-line git-gutter marks for `current` against `original` (HEAD's
/// text), one per line as `Buffer` counts them (1-based, matching
/// the new-side numbers of `merged`). The second value is every 1-based line number
/// with a deletion RIGHT BEFORE it that nothing in `current` absorbed (0
/// meaning before the first line): a removed line has none of its own to
/// mark, so it marks the boundary instead. A run of deletes immediately
/// followed by a run of inserts is a REPLACE — `Modified`, not `Added`
/// plus a deleted marker — the same pairing most gutters and `git diff`
/// show; only the leftover, unpaired side (whichever run is longer)
/// keeps its own mark.
pub fn gutter_marks(original: &str, current: &str) -> (Vec<GutterMark>, Vec<usize>) {
    let diff = TextDiff::from_lines(original, current);
    let changes: Vec<_> = diff.iter_all_changes().collect();
    let mut marks = vec![];
    let mut deleted_before = vec![];
    let mut new_line = 0usize;
    let mut i = 0;
    while i < changes.len() {
        match changes[i].tag() {
            ChangeTag::Equal => {
                new_line += 1;
                marks.push(GutterMark::Clean);
                i += 1;
            }
            ChangeTag::Insert => {
                new_line += 1;
                marks.push(GutterMark::Added);
                i += 1;
            }
            ChangeTag::Delete => {
                let del_start = i;
                while i < changes.len() && changes[i].tag() == ChangeTag::Delete {
                    i += 1;
                }
                let del_count = i - del_start;
                let ins_start = i;
                while i < changes.len() && changes[i].tag() == ChangeTag::Insert {
                    i += 1;
                }
                let ins_count = i - ins_start;
                let paired = del_count.min(ins_count);
                for _ in 0..paired {
                    new_line += 1;
                    marks.push(GutterMark::Modified);
                }
                for _ in paired..ins_count {
                    new_line += 1;
                    marks.push(GutterMark::Added);
                }
                if del_count > ins_count {
                    deleted_before.push(new_line + 1);
                }
            }
        }
    }
    (marks, deleted_before)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

    fn kinds(m: &Merged) -> Vec<LineKind> {
        m.lines.iter().map(|l| l.kind).collect()
    }

    fn numbers(m: &Merged) -> Vec<(Option<usize>, Option<usize>)> {
        m.lines.iter().map(|l| (l.old, l.new)).collect()
    }

    /// Whatever the input, the text has exactly one line per `DiffLine`:
    /// the editor indexes its decoration by buffer line.
    fn assert_one_line_each(m: &Merged) {
        let count = if m.text.is_empty() {
            0
        } else {
            m.text.split('\n').count()
        };
        assert_eq!(count, m.lines.len(), "{:?}", m.text);
        assert!(!m.text.ends_with('\n'));
    }

    use LineKind::{Added, Context, Placeholder, Removed};

    #[test]
    fn a_pure_addition_is_added_lines_with_new_numbers_only() {
        let m = merged("a\nc\n", "a\nb\nc\n");
        assert_eq!(m.text, "a\nb\nc");
        assert_eq!(kinds(&m), vec![Context, Added, Context]);
        assert_eq!(
            numbers(&m),
            vec![(Some(1), Some(1)), (None, Some(2)), (Some(2), Some(3)),]
        );
        assert!(m.folds.is_empty());
        assert_one_line_each(&m);
    }

    #[test]
    fn a_pure_deletion_keeps_the_line_with_an_old_number_only() {
        let m = merged("a\nb\nc\n", "a\nc\n");
        assert_eq!(m.text, "a\nb\nc");
        assert_eq!(kinds(&m), vec![Context, Removed, Context]);
        assert_eq!(
            numbers(&m),
            vec![(Some(1), Some(1)), (Some(2), None), (Some(3), Some(2)),]
        );
        assert_one_line_each(&m);
    }

    #[test]
    fn a_changed_line_is_the_removed_line_then_the_added_one() {
        let m = merged("one\ntwo\nthree\n", "one\nTWO\nthree\n");
        assert_eq!(m.text, "one\ntwo\nTWO\nthree");
        assert_eq!(kinds(&m), vec![Context, Removed, Added, Context]);
        assert_eq!(
            numbers(&m),
            vec![
                (Some(1), Some(1)),
                (Some(2), None),
                (None, Some(2)),
                (Some(3), Some(3)),
            ]
        );
    }

    #[test]
    fn a_file_with_no_trailing_newline_differs_from_one_with() {
        // The last line is compared with its ending, as before: "c" and "c\n"
        // are different lines, and no phantom empty line follows either.
        let m = merged("a\nb\nc", "a\nb\nc");
        assert_eq!(m.text, "a\nb\nc");
        assert_eq!(kinds(&m), vec![Context; 3]);
        assert_one_line_each(&m);
        let m = merged("a\nb\nc", "a\nb\nc\n");
        assert_eq!(m.text, "a\nb\nc\nc");
        assert_eq!(kinds(&m), vec![Context, Context, Removed, Added]);
        assert_one_line_each(&m);
    }

    #[test]
    fn a_new_file_is_all_added() {
        let m = merged("", "one\ntwo\n");
        assert_eq!(m.text, "one\ntwo");
        assert_eq!(kinds(&m), vec![Added, Added]);
        assert_eq!(numbers(&m), vec![(None, Some(1)), (None, Some(2))]);
        assert!(m.folds.is_empty());
        assert_one_line_each(&m);
    }

    #[test]
    fn a_deleted_file_is_all_removed() {
        let m = merged("one\ntwo\n", "");
        assert_eq!(m.text, "one\ntwo");
        assert_eq!(kinds(&m), vec![Removed, Removed]);
        assert_eq!(numbers(&m), vec![(Some(1), None), (Some(2), None)]);
        assert_one_line_each(&m);
    }

    #[test]
    fn nothing_on_either_side_is_no_lines() {
        let m = merged("", "");
        assert_eq!(m.text, "");
        assert!(m.lines.is_empty());
        assert!(m.folds.is_empty());
    }

    #[test]
    fn crlf_endings_are_not_part_of_a_line() {
        let m = merged("a\r\nb\r\nc\r\n", "a\r\nB\r\nc\r\n");
        assert_eq!(m.text, "a\nb\nB\nc");
        assert!(!m.text.contains('\r'));
        assert_eq!(kinds(&m), vec![Context, Removed, Added, Context]);
        // An ending change alone is a change, as git sees it.
        let m = merged("a\nb\n", "a\r\nb\n");
        assert_eq!(m.text, "a\na\nb");
        assert_eq!(kinds(&m), vec![Removed, Added, Context]);
    }

    #[test]
    fn a_change_in_the_middle_keeps_three_lines_of_context_each_side_and_folds_the_rest() {
        let original = lines(20);
        let current = original.replace("line 10\n", "line ten\n");
        let m = merged(&original, &current);
        // Lines 1-6 hidden under a placeholder, 7-9 context, the change,
        // 11-13 context, a placeholder over 14-20.
        assert_eq!(m.lines[0].kind, Placeholder);
        assert_eq!(m.text.lines().next().unwrap(), "⋯ 6 unchanged lines");
        assert_eq!(m.lines[1].new, Some(1), "the hidden lines are real lines");
        assert_eq!(m.lines[6].new, Some(6));
        assert_eq!(m.lines[7].new, Some(7), "first context line");
        assert_eq!(m.lines[10].kind, Removed);
        assert_eq!(m.lines[11].kind, Added);
        assert_eq!(m.lines[11].new, Some(10));
        assert_eq!(m.folds, vec![(0, 6), (15, 22)]);
        let placeholders: Vec<_> = m
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.kind == Placeholder)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            placeholders,
            vec![0, 15],
            "every fold heads on a placeholder"
        );
        assert_eq!(m.text.split('\n').nth(15).unwrap(), "⋯ 7 unchanged lines");
        assert_eq!(m.lines.len(), 23);
        assert_one_line_each(&m);
    }

    #[test]
    fn short_unchanged_runs_stay_unfolded() {
        let m = merged("a\nb\nc\n", "a\nx\nc\n");
        assert!(m.folds.is_empty());
        assert!(!kinds(&m).contains(&Placeholder));
    }

    #[test]
    fn identical_texts_are_one_fold_over_everything() {
        let t = lines(10);
        let m = merged(&t, &t);
        assert_eq!(m.lines.len(), 11);
        assert_eq!(m.lines[0].kind, Placeholder);
        assert_eq!(m.text.lines().next().unwrap(), "⋯ 10 unchanged lines");
        assert_eq!(m.folds, vec![(0, 10)]);
    }

    fn span(start: usize, end: usize, capture: &'static str) -> Span {
        Span {
            start,
            end,
            capture,
        }
    }

    #[test]
    fn spans_follow_their_side_onto_the_merged_text() {
        // HEAD: "let a = 1;\nlet b = 2;\n"   now: "let a = 1;\nlet c = 3;\n"
        let m = merged("let a = 1;\nlet b = 2;\n", "let a = 1;\nlet c = 3;\n");
        assert_eq!(m.text, "let a = 1;\nlet b = 2;\nlet c = 3;");
        // `let` is a keyword on every line; the digit's capture says which
        // side it was highlighted on.
        let old = [
            span(0, 3, "keyword"),
            span(11, 14, "keyword"),
            span(19, 20, "old"),
        ];
        let new = [
            span(0, 3, "keyword"),
            span(11, 14, "keyword"),
            span(19, 20, "new"),
        ];
        let got = merged_spans(&m, &old, &new);
        // Merged lines start at 0, 11 and 22; the removed line (HEAD's second)
        // takes the old spans, the added one (the working tree's second) the new.
        assert_eq!(
            got,
            vec![
                span(0, 3, "keyword"),
                span(11, 14, "keyword"),
                span(19, 20, "old"),
                span(22, 25, "keyword"),
                span(30, 31, "new"),
            ]
        );
    }

    #[test]
    fn a_span_over_two_lines_is_cut_at_each_and_a_placeholder_has_none() {
        // One span over "a\nb" (a multi-line string) in an unchanged file.
        let m = merged("a\nb\n", "a\nb\n");
        let got = merged_spans(&m, &[], &[span(0, 3, "string")]);
        assert_eq!(got, vec![span(0, 1, "string"), span(2, 3, "string")]);
        // With a fold in front, the lines move down by the placeholder's.
        let t = lines(10);
        let current = format!("{t}extra\n");
        let m = merged(&t, &current);
        assert_eq!(m.lines[0].kind, Placeholder);
        let first_line_len = "⋯ 7 unchanged lines".len();
        let got = merged_spans(&m, &[], &[span(0, 4, "keyword")]);
        // "line 1" is the first hidden line, right after the placeholder.
        assert_eq!(
            got,
            vec![span(first_line_len + 1, first_line_len + 5, "keyword")]
        );
    }
}
