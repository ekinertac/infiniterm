//! A unified diff as rows to draw: what `@codemirror/merge`'s
//! `unifiedMergeView` showed the reference, with its collapse rule.
//! HEAD's text is the original, the working tree's the current; deleted
//! lines appear where they were, inserted lines with their new numbers,
//! and unchanged stretches longer than `MIN_COLLAPSE` fold to one marker
//! keeping `MARGIN` lines of context on each side. Pure over two strings,
//! so the rule is tested without git.
use similar::{ChangeTag, TextDiff};

/// Context kept around a change, and the shortest unchanged run that
/// collapses: the reference's `collapseUnchanged: { margin: 3, minSize: 4 }`.
pub const MARGIN: usize = 3;
pub const MIN_COLLAPSE: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffRow {
    /// An unchanged line, with its number in the current file.
    Context {
        line: usize,
        text: String,
    },
    Added {
        line: usize,
        text: String,
    },
    /// A line HEAD had; it has no number in the current file.
    Deleted {
        text: String,
    },
    /// `count` unchanged lines folded away.
    Collapsed {
        count: usize,
    },
}

impl DiffRow {
    /// The line in the current file this row shows, for blame and spans.
    pub fn line(&self) -> Option<usize> {
        match self {
            DiffRow::Context { line, .. } | DiffRow::Added { line, .. } => Some(*line),
            _ => None,
        }
    }

    pub fn text(&self) -> &str {
        match self {
            DiffRow::Context { text, .. }
            | DiffRow::Added { text, .. }
            | DiffRow::Deleted { text } => text,
            DiffRow::Collapsed { .. } => "",
        }
    }
}

/// The rows for `original` against `current`. Lines are compared whole,
/// as the merge view compared them before its character-level marks.
pub fn diff_rows(original: &str, current: &str) -> Vec<DiffRow> {
    let diff = TextDiff::from_lines(original, current);
    let mut rows: Vec<DiffRow> = vec![];
    let mut new_line = 0usize;
    for change in diff.iter_all_changes() {
        let text = change.value().trim_end_matches(['\n', '\r']).to_string();
        match change.tag() {
            ChangeTag::Equal => {
                new_line += 1;
                rows.push(DiffRow::Context {
                    line: new_line,
                    text,
                });
            }
            ChangeTag::Insert => {
                new_line += 1;
                rows.push(DiffRow::Added {
                    line: new_line,
                    text,
                });
            }
            ChangeTag::Delete => rows.push(DiffRow::Deleted { text }),
        }
    }
    collapse(rows)
}

/// Folds unchanged runs, keeping `MARGIN` lines beside each change. A run
/// at the start or the end keeps context only on the side that has one.
fn collapse(rows: Vec<DiffRow>) -> Vec<DiffRow> {
    let mut out = vec![];
    let mut i = 0;
    let n = rows.len();
    while i < n {
        if !matches!(rows[i], DiffRow::Context { .. }) {
            out.push(rows[i].clone());
            i += 1;
            continue;
        }
        let start = i;
        while i < n && matches!(rows[i], DiffRow::Context { .. }) {
            i += 1;
        }
        let run = i - start;
        let lead = if start == 0 { 0 } else { MARGIN };
        let trail = if i == n { 0 } else { MARGIN };
        if run >= lead + trail + MIN_COLLAPSE {
            out.extend(rows[start..start + lead].iter().cloned());
            out.push(DiffRow::Collapsed {
                count: run - lead - trail,
            });
            out.extend(rows[i - trail..i].iter().cloned());
        } else {
            out.extend(rows[start..i].iter().cloned());
        }
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
/// `diff_rows`'s `line`). The second value is every 1-based line number
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

    #[test]
    fn a_change_in_the_middle_keeps_three_lines_of_context_each_side() {
        let original = lines(20);
        let current = original.replace("line 10\n", "line ten\n");
        let rows = diff_rows(&original, &current);
        assert_eq!(rows[0], DiffRow::Collapsed { count: 6 });
        assert_eq!(
            rows[1],
            DiffRow::Context {
                line: 7,
                text: "line 7".into()
            }
        );
        assert_eq!(
            rows[4],
            DiffRow::Deleted {
                text: "line 10".into()
            }
        );
        assert_eq!(
            rows[5],
            DiffRow::Added {
                line: 10,
                text: "line ten".into()
            }
        );
        assert_eq!(rows[9], DiffRow::Collapsed { count: 7 });
        assert_eq!(rows.len(), 10);
    }

    #[test]
    fn short_unchanged_runs_stay_and_a_new_file_is_all_added() {
        let rows = diff_rows("a\nb\nc\n", "a\nx\nc\n");
        assert!(!rows.iter().any(|r| matches!(r, DiffRow::Collapsed { .. })));
        assert_eq!(rows.len(), 4);
        let rows = diff_rows("", "one\ntwo\n");
        assert!(rows.iter().all(|r| matches!(r, DiffRow::Added { .. })));
        assert_eq!(rows[1].line(), Some(2));
    }

    #[test]
    fn identical_texts_collapse_to_one_marker() {
        let t = lines(10);
        assert_eq!(diff_rows(&t, &t), vec![DiffRow::Collapsed { count: 10 }]);
    }

    #[test]
    fn identical_text_is_every_line_clean() {
        let (marks, deleted) = gutter_marks("a\nb\nc\n", "a\nb\nc\n");
        assert_eq!(marks, vec![GutterMark::Clean; 3]);
        assert!(deleted.is_empty());
    }

    #[test]
    fn a_pure_addition_marks_only_the_new_lines() {
        let (marks, deleted) = gutter_marks("a\nc\n", "a\nb\nc\n");
        assert_eq!(
            marks,
            vec![GutterMark::Clean, GutterMark::Added, GutterMark::Clean]
        );
        assert!(deleted.is_empty());
    }

    #[test]
    fn a_pure_deletion_marks_the_boundary_not_a_line() {
        let (marks, deleted) = gutter_marks("a\nb\nc\n", "a\nc\n");
        assert_eq!(marks, vec![GutterMark::Clean, GutterMark::Clean]);
        assert_eq!(deleted, vec![2]); // before "c", the second remaining line
        // Deleting the very first line marks before line 1.
        let (_, deleted) = gutter_marks("a\nb\n", "b\n");
        assert_eq!(deleted, vec![1]);
    }

    #[test]
    fn a_same_size_replace_is_modified_not_added_plus_deleted() {
        let (marks, deleted) = gutter_marks("one\ntwo\nthree\n", "one\nTWO\nthree\n");
        assert_eq!(
            marks,
            vec![GutterMark::Clean, GutterMark::Modified, GutterMark::Clean]
        );
        assert!(deleted.is_empty());
    }

    #[test]
    fn an_uneven_replace_pairs_what_it_can_and_marks_the_rest() {
        // Two lines replaced by one: one Modified, one leftover delete.
        let (marks, deleted) = gutter_marks("a\nb\nc\nd\n", "a\nX\nd\n");
        assert_eq!(
            marks,
            vec![GutterMark::Clean, GutterMark::Modified, GutterMark::Clean]
        );
        assert_eq!(deleted, vec![3]); // before "d"
        // One line replaced by two: one Modified, one leftover Added.
        let (marks, deleted) = gutter_marks("a\nb\nd\n", "a\nX\nY\nd\n");
        assert_eq!(
            marks,
            vec![
                GutterMark::Clean,
                GutterMark::Modified,
                GutterMark::Added,
                GutterMark::Clean
            ]
        );
        assert!(deleted.is_empty());
    }
}
