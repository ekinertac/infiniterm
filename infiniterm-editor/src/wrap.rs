//! Soft line wrapping for prose: a line longer than the columns is shown
//! as several visual rows, broken after a space where one falls in range
//! and at the column otherwise, the way CodeMirror's `lineWrapping` does.
//! The buffer never changes; only the view's rows do. Pure, so the row
//! table the body draws and hit-tests with is one tested function.

/// The visual rows of one line, as char ranges into it; a line always has
/// at least one row.
pub fn wrap_line(text: &str, cols: usize) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let cols = cols.max(1);
    if chars.len() <= cols {
        return vec![(0, chars.len())];
    }
    let mut rows = vec![];
    let mut start = 0;
    while start + cols < chars.len() {
        // The last space within the window keeps the word whole; a word
        // longer than the window is cut.
        let window = &chars[start..start + cols + 1];
        let cut = window
            .iter()
            .rposition(|c| *c == ' ')
            .filter(|i| *i > 0)
            .map(|i| start + i + 1)
            .unwrap_or(start + cols);
        rows.push((start, cut));
        start = cut;
    }
    rows.push((start, chars.len()));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_lines_are_one_row_and_long_ones_break_after_spaces() {
        assert_eq!(wrap_line("hello", 10), vec![(0, 5)]);
        assert_eq!(wrap_line("", 10), vec![(0, 0)]);
        let rows = wrap_line("the quick brown fox jumps", 10);
        // "the quick " | "brown fox " | "jumps"
        assert_eq!(rows, vec![(0, 10), (10, 20), (20, 25)]);
    }

    #[test]
    fn a_word_longer_than_the_width_is_cut() {
        assert_eq!(
            wrap_line("abcdefghijkl", 5),
            vec![(0, 5), (5, 10), (10, 12)]
        );
    }
}
