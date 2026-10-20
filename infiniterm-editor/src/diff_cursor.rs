//! The diff pane's line cursor: which row of `diff::diff_rows` it is on and
//! how Up, Down, Page Up, Page Down, Home and End move it (#372).
//!
//! Pure over the rows, so the rules are tested without a window.
//! `infiniterm-ui/src/diff_body.rs` calls it from `DiffBody::key` and draws
//! the row; the editor's own caret is not reused because the diff has no
//! buffer (see docs/agent/browser-and-pages.md for what sharing the editor
//! body would take). A `Collapsed` marker ("⋯ 40 unchanged lines") is not a
//! line of text, so the cursor steps over it and never rests on it.
use crate::diff::DiffRow;

fn is_text(row: &DiffRow) -> bool {
    !matches!(row, DiffRow::Collapsed { .. })
}

/// The first row the cursor may rest on (0 when there are none).
pub fn first(rows: &[DiffRow]) -> usize {
    rows.iter().position(is_text).unwrap_or(0)
}

/// The last row the cursor may rest on (0 when there are none).
pub fn last(rows: &[DiffRow]) -> usize {
    rows.iter().rposition(is_text).unwrap_or(0)
}

/// `cursor` moved by `delta` text rows (negative is up), clamped to the
/// first and last row; collapsed markers do not count. A cursor that sits on
/// a marker (the rows changed under it) is treated as the nearest text row
/// in the direction of travel.
pub fn step(rows: &[DiffRow], cursor: usize, delta: i64) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let mut at = cursor.min(rows.len() - 1);
    let dir: i64 = if delta < 0 { -1 } else { 1 };
    let mut left = delta.unsigned_abs().max(if is_text(&rows[at]) { 0 } else { 1 });
    while left > 0 {
        let next = at as i64 + dir;
        if next < 0 || next >= rows.len() as i64 {
            break;
        }
        at = next as usize;
        if is_text(&rows[at]) {
            left -= 1;
        }
    }
    if is_text(&rows[at]) {
        at
    } else if dir < 0 {
        first(rows)
    } else {
        last(rows)
    }
}

/// The scroll that keeps `cursor` inside a window of `visible` rows, moving
/// as little as it can: a cursor already in view leaves the scroll alone.
pub fn follow(scroll: usize, cursor: usize, visible: usize) -> usize {
    let visible = visible.max(1);
    if cursor < scroll {
        cursor
    } else if cursor >= scroll + visible {
        cursor + 1 - visible
    } else {
        scroll
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(line: usize) -> DiffRow {
        DiffRow::Context {
            line,
            text: String::new(),
        }
    }

    fn rows() -> Vec<DiffRow> {
        // 0 folded, 1 context, 2 deleted, 3 folded, 4 context, 5 folded
        vec![
            DiffRow::Collapsed { count: 9 },
            ctx(10),
            DiffRow::Deleted { text: "x".into() },
            DiffRow::Collapsed { count: 5 },
            ctx(20),
            DiffRow::Collapsed { count: 7 },
        ]
    }

    #[test]
    fn the_ends_are_text_rows_not_markers() {
        assert_eq!(first(&rows()), 1);
        assert_eq!(last(&rows()), 4);
        assert_eq!(first(&[]), 0);
        assert_eq!(last(&[]), 0);
    }

    #[test]
    fn a_step_skips_collapsed_markers_and_clamps_at_the_ends() {
        let r = rows();
        assert_eq!(step(&r, 1, 1), 2);
        assert_eq!(step(&r, 2, 1), 4, "over the marker at 3");
        assert_eq!(step(&r, 4, -1), 2);
        assert_eq!(step(&r, 4, 1), 4, "the last text row stays");
        assert_eq!(step(&r, 1, -1), 1, "the first text row stays");
        assert_eq!(step(&r, 1, 100), 4);
        assert_eq!(step(&r, 4, -100), 1);
        assert_eq!(step(&r, 1, 0), 1);
    }

    #[test]
    fn a_cursor_left_on_a_marker_moves_to_the_nearest_text_row() {
        let r = rows();
        assert_eq!(step(&r, 3, 1), 4);
        assert_eq!(step(&r, 3, -1), 2);
        assert_eq!(step(&r, 0, 1), 1);
        assert_eq!(step(&r, 5, -1), 4);
        assert_eq!(step(&r, 99, -1), 4, "past the end is clamped first");
    }

    #[test]
    fn no_rows_means_row_zero() {
        assert_eq!(step(&[], 3, 1), 0);
    }

    #[test]
    fn follow_scrolls_the_least_that_shows_the_cursor() {
        assert_eq!(follow(10, 12, 5), 10, "in view: no move");
        assert_eq!(follow(10, 9, 5), 9, "above: cursor at the top");
        assert_eq!(follow(10, 15, 5), 11, "below: cursor at the bottom");
        assert_eq!(follow(10, 14, 5), 10, "the last visible row is in view");
        assert_eq!(follow(0, 3, 0), 3, "a zero window counts as one row");
    }
}
