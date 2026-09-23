//! Snippets: predefined text pasted into the focused card from a picker
//! (`snippet.paste`, Cmd+Ctrl+S). The folder IS the interface: one plain
//! file per snippet in `~/.config/infiniterm/snippets/`, the file's name
//! (without its extension) is the snippet's name and its contents are the
//! text, exactly as written. It was one JSON file for a day (2026-09-22);
//! a multi-line prompt in JSON is `\n`s or an array of quoted lines, and
//! YAML's block scalars were weighed and passed over for a folder because
//! a prompt needs no escaping and no indentation rule there, and our
//! editor card already browses a folder.
//!
//! This module is pure: it turns file names and contents into rows, and
//! turns the old JSON file into files once. The ui reads and writes the
//! disk (`refresh_snippets` in runtime.rs) every time the picker opens, so
//! a save in the editor is live on the next Cmd+Ctrl+S; the model opens the
//! picker (`Source::Snippets`) and asks for the paste (`Effect::PasteText`),
//! which goes through the terminal's paste path, bracketed when the program
//! asked. Related: config_files.rs (`snippets_dir`).
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub name: String,
    pub text: String,
}

/// The picker's last row: opens the folder in an editor card. A name a
/// snippet could never have, since no file name starts with this ellipsis
/// by accident.
pub const EDIT_ROW: &str = "…edit";

/// What an empty or missing folder is seeded with, so the first "Edit
/// snippets…" lands on files that show the shape: one line, and several.
pub const EXAMPLES: &[(&str, &str)] = &[
    (
        "fix tests.md",
        "run the tests, fix what fails, and do not touch unrelated code\n",
    ),
    (
        "review.md",
        "review the diff on this branch as a careful colleague would:\ncorrectness first, then what could be simpler\n",
    ),
];

/// A file's snippet name: the name without its last extension. Hidden
/// files (`.DS_Store`, an editor's swap file) are not snippets.
pub fn name_of(file_name: &str) -> Option<String> {
    if file_name.starts_with('.') {
        return None;
    }
    let stem = match file_name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => file_name,
    };
    (!stem.trim().is_empty()).then(|| stem.to_string())
}

/// A file's text as it is pasted: exactly the contents, less ONE trailing
/// line break. Every editor ends a saved file with one, and pasted into a
/// shell that did not ask for bracketed paste it would run the line.
pub fn text_of(contents: &str) -> String {
    let t = contents
        .strip_suffix("\r\n")
        .or_else(|| contents.strip_suffix('\n'))
        .unwrap_or(contents);
    t.to_string()
}

/// The picker's rows from the folder's files, `(file name, contents)`,
/// sorted by name the way the tree shows them. Two files with one name
/// (`review.md` and `review.txt`) keep the first; the picker's rows are
/// keyed by name.
pub fn collect(mut files: Vec<(String, String)>) -> Vec<Snippet> {
    files.sort_by_key(|f| f.0.to_lowercase());
    let mut out: Vec<Snippet> = vec![];
    for (file, contents) in files {
        let Some(name) = name_of(&file) else { continue };
        if out.iter().any(|s| s.name == name) {
            continue;
        }
        out.push(Snippet {
            name,
            text: text_of(&contents),
        });
    }
    out
}

/// A file name for a snippet name, for the one-time move from JSON: a
/// slash or a colon cannot be in a macOS file name the way it is typed.
pub fn file_name_for(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| if matches!(c, '/' | ':') { '-' } else { c })
        .collect();
    format!("{}.md", safe.trim())
}

/// The day-old `snippets.json` as files, `(file name, contents)`: a string
/// or an array of lines per name, the shape it had. `Err` when it is not a
/// JSON object; a bad value is skipped so one typo does not lose the rest.
pub fn from_json(text: &str) -> Result<Vec<(String, String)>, String> {
    let v: Value = crate::jsonc::parse_jsonc(text).map_err(|e| e.to_string())?;
    let Some(obj) = v.as_object() else {
        return Err("snippets.json is not an object of name to text".into());
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
            Some((file_name_for(name), format!("{text}\n")))
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
    fn f(name: &str, text: &str) -> (String, String) {
        (name.to_string(), text.to_string())
    }

    #[test]
    fn the_file_name_is_the_name_and_the_contents_are_the_text() {
        let got = collect(vec![
            f("review.md", "line one\nline two\n"),
            f("fix tests.txt", "run them\n"),
        ]);
        assert_eq!(
            got,
            vec![
                Snippet {
                    name: "fix tests".into(),
                    text: "run them".into()
                },
                Snippet {
                    name: "review".into(),
                    text: "line one\nline two".into()
                },
            ]
        );
    }

    #[test]
    fn only_one_trailing_line_break_goes() {
        assert_eq!(text_of("a\n"), "a");
        assert_eq!(text_of("a\r\n"), "a");
        // A blank line the author put there on purpose stays.
        assert_eq!(text_of("a\n\n"), "a\n");
        assert_eq!(text_of("a"), "a");
    }

    #[test]
    fn hidden_files_and_nameless_files_are_not_snippets() {
        assert_eq!(name_of(".DS_Store"), None);
        assert_eq!(name_of(".review.md.swp"), None);
        assert_eq!(name_of("notes"), Some("notes".into()));
        assert_eq!(name_of("v1.2 release.md"), Some("v1.2 release".into()));
        let got = collect(vec![f("a.md", "1"), f("A.txt", "2"), f(".x", "3")]);
        assert_eq!(got.len(), 2);
        let dup = collect(vec![f("a.md", "1"), f("a.txt", "2")]);
        assert_eq!(dup.len(), 1, "one row per name");
    }

    #[test]
    fn the_old_json_file_becomes_files() {
        let files = from_json(r#"{"b/c": "one", "a": ["x", "y"], "n": 3}"#).unwrap();
        assert_eq!(files, vec![f("b-c.md", "one\n"), f("a.md", "x\ny\n")]);
        assert!(from_json("[1]").is_err());
        // And reads back as it was written.
        let back = collect(files);
        assert_eq!(back[0].text, "x\ny");
    }

    #[test]
    fn the_examples_read_back_and_the_hint_is_the_first_line() {
        let got = collect(EXAMPLES.iter().map(|(n, t)| f(n, t)).collect());
        assert_eq!(got.len(), 2);
        assert_eq!(
            first_line(&got[1].text),
            "review the diff on this branch as a careful colleague would:"
        );
        assert_eq!(first_line(""), "");
    }
}
