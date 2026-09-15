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
}
