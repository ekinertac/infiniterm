//! Snippets: predefined text pasted into the focused card from a picker
//! (`snippet.paste`, Cmd+Ctrl+S). The file IS the interface: one JSON
//! object in `~/.config/infiniterm/snippets.json`, a name to a string or
//! to an array of lines, re-read every time the picker opens so an edit is
//! live. No editor of its own beyond a row that opens that file in a card.
//!
//! This module parses the file and shapes the rows; the model opens the
//! picker (`Source::Snippets`, palette_state.rs) and asks for the paste
//! (`Effect::PasteText`); the ui reads the file (`refresh_snippets` in
//! runtime.rs) and performs the paste through the body, bracketed when the
//! program asked, so Claude gets a multi-line prompt as one block. Related:
//! config_files.rs (the path), palette_usage.rs (the recency the rows get).
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub name: String,
    pub text: String,
}

/// The picker's last row: opens the file in an editor card. A name a
/// snippet could never have, since a JSON key with this ellipsis is nobody's.
pub const EDIT_ROW: &str = "…edit";

/// What a missing file is created with, so the first "Edit snippets…" lands
/// in a file that shows the two shapes a value can take.
pub const EXAMPLE: &str = r#"{
  "fix tests": "run the tests, fix what fails, and do not touch unrelated code",
  "review": [
    "review the diff on this branch as a careful colleague would:",
    "correctness first, then what could be simpler"
  ]
}
"#;

/// The file's snippets in the file's order; a value that is neither a
/// string nor an array of strings is skipped, not fatal, so one typo does
/// not empty the picker. `Err` only when the file is not a JSON object.
pub fn parse(text: &str) -> Result<Vec<Snippet>, String> {
    let v: Value = crate::jsonc::parse_jsonc(text).map_err(|e| e.to_string())?;
    let Some(obj) = v.as_object() else {
        return Err("snippets.json must be an object of name to text".into());
    };
    Ok(obj
        .iter()
        .filter_map(|(name, value)| {
            let text = match value {
                Value::String(s) => s.clone(),
                Value::Array(lines) => lines
                    .iter()
                    .map(|l| l.as_str())
                    .collect::<Option<Vec<_>>>()?
                    .join("\n"),
                _ => return None,
            };
            Some(Snippet {
                name: name.clone(),
                text,
            })
        })
        .collect())
}

/// The row's hint: the first line, so a name like "review" shows what it
/// starts with.
pub fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_string_and_an_array_of_lines_both_parse_in_file_order() {
        let got = parse(r#"{"b": "one", "a": ["x", "y"]}"#).unwrap();
        assert_eq!(
            got,
            vec![
                Snippet {
                    name: "b".into(),
                    text: "one".into()
                },
                Snippet {
                    name: "a".into(),
                    text: "x\ny".into()
                },
            ]
        );
    }

    #[test]
    fn a_bad_value_is_skipped_and_a_non_object_is_refused() {
        let got = parse(r#"{"ok": "text", "n": 3, "mixed": ["a", 1]}"#).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "ok");
        assert!(parse("[1, 2]").is_err());
        assert!(parse("not json").is_err());
    }

    #[test]
    fn comments_are_allowed_like_the_other_config_files() {
        let got = parse("{\n  // mine\n  \"a\": \"b\",\n}").unwrap();
        assert_eq!(got[0].text, "b");
    }

    #[test]
    fn the_example_parses_and_the_hint_is_the_first_line() {
        let got = parse(EXAMPLE).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(
            first_line(&got[1].text),
            "review the diff on this branch as a careful colleague would:"
        );
        assert_eq!(first_line(""), "");
    }
}
