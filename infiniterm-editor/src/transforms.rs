//! Palette-only text transforms: pure functions over a string, one per
//! `EditorAction::Transform*` the palette lists (`cards_cmd.rs`). No
//! buffer, no selection: `editors.rs` picks the range (the selection, or
//! the whole document when there is none) and calls in here.

/// Splits into "words" the way case-conversion tools do: a run of
/// digits/letters breaks on whitespace, `_` and `-`, and again wherever a
/// lower-to-upper or an acronym-to-word boundary falls (`camelCase`,
/// `HTTPServer` -> "HTTP", "Server").
fn split_words(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut words = vec![];
    let mut cur = String::new();
    for i in 0..chars.len() {
        let c = chars[i];
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if !cur.is_empty() {
            let prev_char = cur.chars().last().unwrap();
            let starts_new = (prev_char.is_lowercase() || prev_char.is_ascii_digit())
                && c.is_uppercase()
                || (prev_char.is_uppercase()
                    && c.is_uppercase()
                    && chars.get(i + 1).is_some_and(|n| n.is_lowercase()));
            if starts_new {
                words.push(std::mem::take(&mut cur));
            }
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

pub fn to_upper(s: &str) -> String {
    s.to_uppercase()
}

pub fn to_lower(s: &str) -> String {
    s.to_lowercase()
}

/// The first letter of every whitespace-separated word capitalised, the
/// rest untouched (so acronyms inside a word survive).
pub fn to_title_case(s: &str) -> String {
    s.split_inclusive(char::is_whitespace)
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

pub fn to_snake_case(s: &str) -> String {
    split_words(s)
        .iter()
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join("_")
}

pub fn to_kebab_case(s: &str) -> String {
    split_words(s)
        .iter()
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join("-")
}

pub fn to_camel_case(s: &str) -> String {
    let words = split_words(s);
    words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if i == 0 {
                w.to_lowercase()
            } else {
                let mut c = w.to_lowercase();
                let mut chars = c.drain(..);
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect()
}

pub fn sort_lines(s: &str) -> String {
    let mut lines: Vec<&str> = s.split('\n').collect();
    lines.sort_unstable();
    lines.join("\n")
}

/// Every line kept once, in the order it first appeared.
pub fn unique_lines(s: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    s.split('\n')
        .filter(|l| seen.insert(*l))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn reverse_lines(s: &str) -> String {
    s.split('\n').rev().collect::<Vec<_>>().join("\n")
}

/// Whitespace at the end of every line gone; the newlines themselves stay.
pub fn trim_trailing_whitespace(s: &str) -> String {
    s.split('\n')
        .map(|l| l.trim_end_matches([' ', '\t']))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What a save does to the text before it is written (#778), each half
/// opt-in: `editor.trimTrailingWhitespaceOnSave` trims every line,
/// `editor.insertFinalNewlineOnSave` ends a non-empty text with a newline.
pub fn tidy_on_save(s: &str, trim: bool, final_newline: bool) -> String {
    let mut out = if trim {
        trim_trailing_whitespace(s)
    } else {
        s.to_string()
    };
    if final_newline && !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Every line that is empty or only whitespace gone.
pub fn remove_blank_lines(s: &str) -> String {
    s.split('\n')
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The byte length of a line-number prefix (`  12`, `12:`, `12|`, `12→`,
/// `12<tab>`, `12)` and one space after), if the line has one.
fn line_number_prefix(l: &str) -> Option<usize> {
    let lead = l.len() - l.trim_start_matches([' ', '\t']).len();
    let digits = l[lead..].chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let after = &l[lead + digits..];
    let sep = after.chars().next()?;
    if !matches!(sep, ':' | '|' | '→' | '\t' | ')' | ' ') {
        return None;
    }
    let mut len = lead + digits + sep.len_utf8();
    if !matches!(sep, '\t' | ' ') && l[len..].starts_with(' ') {
        len += 1;
    }
    Some(len)
}

/// The line numbers copied along with code (`cat -n`, a terminal or a web
/// listing) gone. Only when every non-blank line has one: a list that merely
/// starts with a year or a count is left alone.
pub fn strip_line_numbers(s: &str) -> String {
    let lines: Vec<&str> = s.split('\n').collect();
    let all_numbered = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .all(|l| line_number_prefix(l).is_some());
    if !all_numbered {
        return s.to_string();
    }
    lines
        .iter()
        .map(|l| line_number_prefix(l).map_or(*l, |n| &l[n..]))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Leading tabs, `width` spaces apiece; the rest of the line untouched.
pub fn indent_tabs_to_spaces(s: &str, width: usize) -> String {
    let unit = " ".repeat(width);
    s.split('\n')
        .map(|l| {
            let tabs = l.chars().take_while(|c| *c == '\t').count();
            format!("{}{}", unit.repeat(tabs), &l[tabs..])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Leading runs of `width` spaces, one tab apiece; a shorter leftover run
/// stays spaces.
pub fn indent_spaces_to_tabs(s: &str, width: usize) -> String {
    if width == 0 {
        return s.to_string();
    }
    s.split('\n')
        .map(|l| {
            let leading = l.chars().take_while(|c| *c == ' ').count();
            let tabs = leading / width;
            let rest_spaces = leading % width;
            let byte_at = leading;
            format!(
                "{}{}{}",
                "\t".repeat(tabs),
                " ".repeat(rest_spaces),
                &l[byte_at..]
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_conversions_split_on_camel_case_and_separators() {
        assert_eq!(to_snake_case("fooBarBaz"), "foo_bar_baz");
        assert_eq!(to_snake_case("foo-bar baz"), "foo_bar_baz");
        assert_eq!(to_kebab_case("FooBar_baz"), "foo-bar-baz");
        assert_eq!(to_camel_case("foo_bar-baz"), "fooBarBaz");
        assert_eq!(to_snake_case("HTTPServer"), "http_server");
        assert_eq!(to_upper("Shout"), "SHOUT");
        assert_eq!(to_lower("Shout"), "shout");
        assert_eq!(to_title_case("the quick fox"), "The Quick Fox");
    }

    #[test]
    fn line_transforms_operate_per_line() {
        assert_eq!(sort_lines("c\na\nb"), "a\nb\nc");
        assert_eq!(unique_lines("a\nb\na\nc\nb"), "a\nb\nc");
        assert_eq!(reverse_lines("a\nb\nc"), "c\nb\na");
        assert_eq!(trim_trailing_whitespace("a  \nb\t\nc"), "a\nb\nc");
    }

    #[test]
    fn blank_lines_go_whitespace_only_ones_too() {
        assert_eq!(remove_blank_lines("a\n\n  \nb\n"), "a\nb");
    }

    #[test]
    fn line_numbers_go_only_when_every_line_has_one() {
        assert_eq!(strip_line_numbers("  1\tfn a() {\n  2\t}"), "fn a() {\n}");
        assert_eq!(strip_line_numbers("1: a\n2: b\n\n3: c"), "a\nb\n\nc");
        assert_eq!(strip_line_numbers("10| x\n11| y"), "x\ny");
        assert_eq!(strip_line_numbers("1→a"), "a");
        // A year at the start of one line is not a numbered listing.
        assert_eq!(
            strip_line_numbers("2024 was\nnot numbered"),
            "2024 was\nnot numbered"
        );
    }

    #[test]
    fn indentation_converts_leading_whitespace_only() {
        assert_eq!(indent_tabs_to_spaces("\t\tfoo\tbar", 2), "    foo\tbar");
        assert_eq!(indent_spaces_to_tabs("    foo  bar", 2), "\t\tfoo  bar");
        assert_eq!(indent_spaces_to_tabs("   foo", 2), "\t foo");
    }

    #[test]
    fn a_save_tidies_only_what_is_switched_on() {
        assert_eq!(tidy_on_save("a  \nb\t", false, false), "a  \nb\t");
        assert_eq!(tidy_on_save("a  \nb\t", true, false), "a\nb");
        assert_eq!(tidy_on_save("a  \nb\t", true, true), "a\nb\n");
        assert_eq!(tidy_on_save("a\n", false, true), "a\n");
        assert_eq!(tidy_on_save("", false, true), "", "an empty file stays empty");
    }
}
