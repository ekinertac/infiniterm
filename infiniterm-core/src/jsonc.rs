//! JSON with comments and trailing commas: the dialect the config files are
//! written in. Port of jsonc.ts with the cases of jsonc.test.ts and
//! jsoncPatch.test.ts.
//!
//! The defaults file is documentation, every setting with a comment above
//! it, and it must be pasteable into the user's own file. Comments are
//! STRIPPED to spaces, never parsed into a tree: nothing reads them back,
//! and keeping offsets identical is what lets `patch_json_text` scan the
//! blanked copy for structure while editing the original at the same
//! positions. That is the "never reserialise the user's file" rule: the
//! theme picker writes `theme` into a file the user hand-edits, and a
//! reserialise would delete every comment.
//!
//! Hand-written, not a regex: `//` and `/*` appear inside strings (URLs,
//! Windows paths) and a regex stripper fails silently, reverting every
//! setting to its default. Works on bytes; every structural character is
//! ASCII and a multi-byte character inside a comment becomes as many
//! spaces, so offsets and UTF-8 validity both hold.
//!
//! Callers: `config.rs`, `settings_doc.rs`, and the settings writer.
use serde_json::Value;

/// Replaces comments with spaces, keeping every byte offset the same.
pub fn strip_comments(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'"' {
            // Copy the whole string literal verbatim, escapes and all:
            // anything inside quotes is data, not syntax.
            i = copy_string(b, i, &mut out);
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            // Blanked rather than deleted, so a parse error still points at
            // the right line and column of the original file.
            while i < b.len() && b[i] != b'\n' {
                out.push(b' ');
                i += 1;
            }
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                out.push(if b[i] == b'\n' { b'\n' } else { b' ' });
                i += 1;
            }
            out.extend_from_slice(b"  ");
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out.truncate(b.len());
    String::from_utf8(out).expect("only ASCII bytes were replaced")
}

/// Copies the string literal starting at `i` into `out`; returns the index
/// after its closing quote (or the end of input when unterminated).
fn copy_string(b: &[u8], mut i: usize, out: &mut Vec<u8>) -> usize {
    out.push(b'"');
    i += 1;
    while i < b.len() {
        if b[i] == b'\\' {
            out.extend_from_slice(&b[i..(i + 2).min(b.len())]);
            i += 2;
            continue;
        }
        out.push(b[i]);
        if b[i] == b'"' {
            return i + 1;
        }
        i += 1;
    }
    i
}

/// Removes a comma that is followed only by a closing brace or bracket.
pub fn strip_trailing_commas(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'"' {
            i = copy_string(b, i, &mut out);
            continue;
        }
        if c == b',' {
            // Look past whitespace for what actually follows.
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if matches!(b.get(j), Some(b'}') | Some(b']')) {
                out.push(b' ');
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    String::from_utf8(out).expect("only ASCII bytes were replaced")
}

/// Parses JSON that may contain comments and trailing commas. Fails exactly
/// where a strict parser would, so a caller that already handles a bad
/// config file needs no new error path.
pub fn parse_jsonc(text: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str(&strip_trailing_commas(&strip_comments(text)))
}

/// Replaces one value in JSON text, leaving everything else byte for byte.
/// `path` is a setting's dotted name (`ui.showFps`). A file that already
/// holds it, flat or under a group (the shape before flat keys), has that
/// value replaced where it is; a file without it gets the FLAT key, so
/// nothing writes the nested shape any more. `None` when nothing would
/// change, so writing the value already there touches no file, and when
/// the text has no object to edit.
pub fn patch_json_text(text: &str, path: &str, value: &Value) -> Option<String> {
    let mask = strip_comments(text);
    let encoded = value.to_string();
    let object = find_object(&mask, 0)?;
    if let Some(flat) = find_key(&mask, object, path) {
        return replace_value(text, flat, &encoded);
    }
    if let Some((head, tail)) = path.split_once('.') {
        if let Some(outer) = find_key(&mask, object, head) {
            if let Some(inner) = find_object(&mask, outer.start) {
                if let Some(leaf) = find_key(&mask, inner, tail) {
                    return replace_value(text, leaf, &encoded);
                }
            }
        }
    }
    Some(insert_key(text, &mask, object, path, &encoded))
}

fn replace_value(text: &str, span: Span, encoded: &str) -> Option<String> {
    if text[span.start..span.end].trim() == encoded {
        return None;
    }
    Some(format!(
        "{}{encoded}{}",
        &text[..span.start],
        &text[span.end..]
    ))
}

/// Byte offsets; for an object, of its braces (`end` inclusive). For a
/// value, `end` is exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
}

/// The `{ ... }` beginning at or after `from`.
fn find_object(mask: &str, from: usize) -> Option<Span> {
    let b = mask.as_bytes();
    let start = from + b[from..].iter().position(|&c| c == b'{')?;
    let mut depth = 0i32;
    let mut i = start;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i = skip_string(b, i);
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(Span { start, end: i });
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Index of the closing quote of the string starting at `i`.
fn skip_string(b: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return i,
            _ => i += 1,
        }
    }
    i
}

/// A key directly inside `object`, as the span of its value.
fn find_key(mask: &str, object: Span, key: &str) -> Option<Span> {
    let b = mask.as_bytes();
    let mut depth = 0i32;
    let mut i = object.start;
    while i <= object.end {
        match b[i] {
            b'"' => {
                let close = skip_string(b, i);
                // Only keys at THIS level, so a `"theme"` one object deeper is not it.
                if depth == 1 && b.get(i + 1..close) == Some(key.as_bytes()) {
                    if let Some(colon) = b[close..].iter().position(|&c| c == b':') {
                        let start = first_non_space(b, close + colon + 1);
                        return Some(Span {
                            start,
                            end: value_end_at(b, start),
                        });
                    }
                }
                i = close;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

fn first_non_space(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Where the value starting at `start` ends (exclusive).
fn value_end_at(b: &[u8], start: usize) -> usize {
    match b.get(start) {
        Some(b'"') => skip_string(b, start) + 1,
        Some(open @ (b'{' | b'[')) => {
            let close = if *open == b'{' { b'}' } else { b']' };
            let mut depth = 0i32;
            let mut i = start;
            while i < b.len() {
                let c = b[i];
                if c == b'"' {
                    i = skip_string(b, i);
                } else if c == *open {
                    depth += 1;
                } else if c == close {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                i += 1;
            }
            start
        }
        _ => {
            let mut i = start;
            while i < b.len() && !b",}]\n".contains(&b[i]) {
                i += 1;
            }
            // Trailing spaces belong to the layout, not the value.
            while i > start && b[i - 1].is_ascii_whitespace() {
                i -= 1;
            }
            i
        }
    }
}

/// Adds a key just before an object's closing brace, matching its indentation.
/// The comma that separates it from the last entry goes right after that
/// entry's value (before any comment on its line, or it would be part of the
/// comment), and not at all when the entry already ends in one: JSONC allows
/// a trailing comma and people leave them, and adding a second one wrote
/// `0.9,,` and a file that no longer parsed (#246).
fn insert_key(text: &str, mask: &str, object: Span, key: &str, encoded: &str) -> String {
    let inner = &mask[object.start + 1..object.end];
    let has_entries = !inner.trim().is_empty();
    // The indentation of the closing brace, plus one step for the new line.
    let line_start = text[..object.end].rfind('\n').map_or(0, |n| n + 1);
    let close_line = &text[line_start..object.end];
    let close_indent = &close_line[..close_line.len() - close_line.trim_start().len()];
    let mut head = text[..object.end].to_string();
    if has_entries {
        // Where the last real token ends; the mask has the comments blanked.
        let last = object.start + 1 + inner.trim_end().len();
        if mask.as_bytes()[last - 1] != b',' {
            head.insert(last, ',');
        }
    }
    let before = head.trim_end();
    format!(
        "{before}\n{close_indent}  {}: {encoded}\n{close_indent}{}",
        Value::String(key.into()),
        &text[object.end..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn p(text: &str) -> Value {
        parse_jsonc(text).unwrap()
    }

    // stripComments
    #[test]
    fn removes_line_and_block_comments() {
        assert_eq!(p("{ \"a\": 1 } // trailing"), json!({"a": 1}));
        assert_eq!(p("// leading\n{ \"a\": 1 }"), json!({"a": 1}));
        assert_eq!(p("{ /* why */ \"a\": 1 }"), json!({"a": 1}));
    }

    // The reason this is not a regex: a URL in a value is the common case and
    // the failure is silent.
    #[test]
    fn leaves_a_double_slash_inside_a_string_alone() {
        assert_eq!(
            p(r#"{ "url": "https://example.com" }"#),
            json!({"url": "https://example.com"})
        );
        assert_eq!(p(r#"{ "p": "C:/x/*y*/z" }"#), json!({"p": "C:/x/*y*/z"}));
    }

    #[test]
    fn survives_escaped_quotes_in_a_string() {
        assert_eq!(p(r#"{ "s": "a \" // b" }"#), json!({"s": "a \" // b"}));
    }

    // Blanked rather than deleted, so an error still points at the right column.
    #[test]
    fn keeps_every_byte_offset_the_same() {
        let before = "{ \"a\": 1 } // note";
        assert_eq!(strip_comments(before).len(), before.len());
    }

    #[test]
    fn keeps_line_numbers_across_a_multi_line_block_comment() {
        let before = "{\n/* one\ntwo */\n\"a\": 1 }";
        assert_eq!(
            strip_comments(before).lines().count(),
            before.lines().count()
        );
    }

    // stripTrailingCommas
    #[test]
    fn removes_a_comma_before_a_closing_brace_or_bracket() {
        assert_eq!(p("{ \"a\": 1, }"), json!({"a": 1}));
        assert_eq!(p("{ \"a\": [1, 2, ] }"), json!({"a": [1, 2]}));
        assert_eq!(p("{\n  \"a\": 1,\n}"), json!({"a": 1}));
    }

    #[test]
    fn keeps_commas_that_separate_things() {
        assert_eq!(p("{ \"a\": 1, \"b\": 2 }"), json!({"a": 1, "b": 2}));
    }

    #[test]
    fn leaves_a_comma_inside_a_string_alone() {
        assert_eq!(
            strip_trailing_commas(r#"{ "s": "a, }" }"#),
            r#"{ "s": "a, }" }"#
        );
    }

    // parseJsonc
    #[test]
    fn handles_a_realistic_commented_settings_file() {
        let text = r#"
// Your settings go here; this overrides the defaults.
{
  // The terminal font. Any monospace family.
  "terminal": {
    "fontSize": 20, // points
    "fontFamily": "Iosevka Term", /* must be installed */
  },
  "startingDir": "/Users/me/Code",
}"#;
        assert_eq!(
            p(text),
            json!({"terminal": {"fontSize": 20, "fontFamily": "Iosevka Term"}, "startingDir": "/Users/me/Code"})
        );
    }

    // Callers already handle a bad config file; this must not need a new path.
    #[test]
    fn fails_the_way_a_strict_parser_fails() {
        assert!(parse_jsonc("{ broken").is_err());
    }

    #[test]
    fn handles_an_empty_file_the_way_json_does() {
        assert!(parse_jsonc("").is_err());
        assert_eq!(p("{}"), json!({}));
        assert_eq!(p("// only a comment\n{}"), json!({}));
    }

    // patchJsonText: the app writes to the same file you edit, and
    // reserialising would delete every comment in it.
    #[test]
    fn keeps_comments_and_formatting_untouched() {
        let before = "// my settings\n{\n  // the scheme I like\n  \"theme\": \"afterglow\",\n\n  // where new cards start\n  \"startingDir\": \"/Users/me/Code\"\n}\n";
        let after = patch_json_text(before, "theme", &json!("Dracula")).unwrap();
        assert!(after.contains("// the scheme I like"));
        assert!(after.contains("// where new cards start"));
        assert!(after.contains("\"theme\": \"Dracula\""));
        assert_eq!(
            p(&after),
            json!({"theme": "Dracula", "startingDir": "/Users/me/Code"})
        );
    }

    // A `"theme"` written inside a comment must not be mistaken for the setting.
    #[test]
    fn does_not_patch_a_key_that_only_appears_in_a_comment() {
        let before = "{\n  // \"theme\": \"old-one-i-turned-off\"\n  \"startingDir\": \"/x\"\n}";
        let after = patch_json_text(before, "theme", &json!("new")).unwrap();
        assert!(after.contains("// \"theme\": \"old-one-i-turned-off\""));
        assert_eq!(p(&after), json!({"startingDir": "/x", "theme": "new"}));
    }

    #[test]
    fn adds_a_key_that_is_not_there_yet() {
        let after = patch_json_text("{\n  \"a\": 1\n}", "theme", &json!("x")).unwrap();
        assert_eq!(p(&after), json!({"a": 1, "theme": "x"}));
    }

    #[test]
    fn adds_a_key_to_an_empty_object() {
        let after = patch_json_text("{\n}", "theme", &json!("x")).unwrap();
        assert_eq!(p(&after), json!({"theme": "x"}));
    }

    #[test]
    fn reaches_one_level_of_nesting() {
        let before = "{\n  \"ui\": {\n    // the counter\n    \"showFps\": true\n  }\n}";
        let after = patch_json_text(before, "ui.showFps", &json!(false)).unwrap();
        assert!(after.contains("// the counter"));
        assert_eq!(p(&after), json!({"ui": {"showFps": false}}));
    }

    // A missing setting is written FLAT, whatever else the file holds; a
    // flat key already there is the one replaced.
    #[test]
    fn adds_a_missing_key_flat_and_replaces_a_flat_one() {
        let a = patch_json_text(
            "{\n  \"ui\": {\n    \"a\": 1\n  }\n}",
            "ui.showFps",
            &json!(false),
        )
        .unwrap();
        assert_eq!(p(&a), json!({"ui": {"a": 1}, "ui.showFps": false}));
        let b = patch_json_text("{\n}", "ui.showFps", &json!(false)).unwrap();
        assert_eq!(p(&b), json!({"ui.showFps": false}));
        let c = patch_json_text("{ \"ui.showFps\": true }", "ui.showFps", &json!(false)).unwrap();
        assert_eq!(p(&c), json!({"ui.showFps": false}));
    }

    // Writing the value that is already there must touch no file.
    #[test]
    fn returns_none_when_nothing_would_change() {
        assert_eq!(
            patch_json_text("{ \"theme\": \"x\" }", "theme", &json!("x")),
            None
        );
        assert_eq!(
            patch_json_text(
                "{ \"ui\": { \"showFps\": true } }",
                "ui.showFps",
                &json!(true)
            ),
            None
        );
    }

    #[test]
    fn handles_every_value_type() {
        assert_eq!(
            p(&patch_json_text("{ \"a\": 1 }", "a", &Value::Null).unwrap()),
            json!({"a": null})
        );
        assert_eq!(
            p(&patch_json_text("{ \"a\": 1 }", "a", &json!(2.5)).unwrap()),
            json!({"a": 2.5})
        );
        assert_eq!(
            p(&patch_json_text("{ \"a\": 1 }", "a", &json!(false)).unwrap()),
            json!({"a": false})
        );
    }

    // A key only matches at its own level, not one nested inside another object.
    #[test]
    fn does_not_patch_a_same_named_key_one_level_deeper() {
        let before = "{\n  \"ui\": { \"theme\": \"inner\" }\n}";
        let after = patch_json_text(before, "theme", &json!("outer")).unwrap();
        assert_eq!(
            p(&after),
            json!({"ui": {"theme": "inner"}, "theme": "outer"})
        );
    }

    #[test]
    fn leaves_a_file_it_cannot_make_sense_of_alone() {
        assert_eq!(patch_json_text("not json at all", "a", &json!(1)), None);
    }

    // A trailing comma is legal JSONC and people leave one: the new key must not
    // add a second (#246: `"ui.cardOpacity": 0.9,,` broke the whole file).
    #[test]
    fn inserting_after_a_trailing_comma_adds_no_second_one() {
        let text = "{\n  \"a\": 1,\n  \"b\": 0.9,\n}\n";
        let out = patch_json_text(text, "ui.windowColor", &json!("lime")).unwrap();
        assert!(!out.contains(",,"), "{out}");
        assert_eq!(p(&out), json!({"a": 1, "b": 0.9, "ui.windowColor": "lime"}));
    }

    // The comma belongs before a comment on the last line, not inside it.
    #[test]
    fn the_comma_goes_before_a_comment_on_the_last_line() {
        let text = "{\n  \"a\": 1 // keep this\n}\n";
        let out = patch_json_text(text, "b", &json!(2)).unwrap();
        assert!(out.contains("// keep this"), "{out}");
        assert_eq!(p(&out), json!({"a": 1, "b": 2}));
        let text = "{\n  \"a\": 1, // keep this\n}\n";
        let out = patch_json_text(text, "b", &json!(2)).unwrap();
        assert_eq!(p(&out), json!({"a": 1, "b": 2}));
        assert!(!out.contains(",,"), "{out}");
    }

    #[test]
    fn inserting_into_an_empty_or_plain_object_still_works() {
        assert_eq!(
            p(&patch_json_text("{\n}\n", "a", &json!(1)).unwrap()),
            json!({"a": 1})
        );
        let out = patch_json_text("{\n  \"a\": 1\n}\n", "b", &json!(2)).unwrap();
        assert_eq!(p(&out), json!({"a": 1, "b": 2}));
    }
}
