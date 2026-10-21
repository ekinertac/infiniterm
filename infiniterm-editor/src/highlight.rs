//! Syntax spans from tree-sitter: the whole buffer parsed and walked with
//! the grammar's highlight query, giving byte ranges tagged with a capture
//! name. Built on demand when the text version changes and cached by the
//! body. A parse of a few thousand lines is milliseconds, which is why
//! there is no incremental tree yet: the ceiling is a file that takes
//! longer than a frame to parse, and the fix then is a parse off the ui
//! thread, not a smarter one on it.
use crate::language::{Language, CAPTURES};
use std::collections::HashMap;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    /// Byte range into the text.
    pub start: usize,
    pub end: usize,
    /// A capture name from `CAPTURES`.
    pub capture: &'static str,
}

/// One per body: the compiled queries for the languages it has shown.
#[derive(Default)]
pub struct Highlighting {
    configs: HashMap<Language, HighlightConfiguration>,
    highlighter: Option<Highlighter>,
}

impl Highlighting {
    pub fn spans(&mut self, language: Language, text: &str) -> Vec<Span> {
        let highlighter = self.highlighter.get_or_insert_with(Highlighter::new);
        let config = match self.configs.entry(language) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => match language.highlight_config() {
                Some(c) => e.insert(c),
                None => return vec![],
            },
        };
        let Ok(events) = highlighter.highlight(config, text.as_bytes(), None, None, |_| None)
        else {
            return vec![];
        };
        let mut out = vec![];
        let mut stack: Vec<&'static str> = vec![];
        for event in events.flatten() {
            match event {
                HighlightEvent::HighlightStart(h) => stack.push(CAPTURES[h.0]),
                HighlightEvent::HighlightEnd => {
                    stack.pop();
                }
                HighlightEvent::Source { start, end } => {
                    if let Some(capture) = stack.last() {
                        out.push(Span {
                            start,
                            end,
                            capture,
                        });
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn captures(language: Language, text: &str) -> Vec<(&'static str, String)> {
        let mut h = Highlighting::default();
        h.spans(language, text)
            .into_iter()
            .map(|s| (s.capture, text[s.start..s.end].to_string()))
            .collect()
    }

    #[test]
    fn rust_keywords_strings_and_comments_are_captured() {
        let c = captures(Language::Rust, "// hi\nfn main() { let s = \"x\"; }");
        assert!(c.contains(&("comment", "// hi".into())));
        assert!(c.contains(&("keyword", "fn".into())));
        assert!(
            c.iter().any(|(k, t)| *k == "string" && t.contains("\"x\""))
                || c.contains(&("string", "\"x\"".into()))
        );
    }

    #[test]
    fn jsonc_comments_survive_the_javascript_grammar() {
        let c = captures(Language::Json, "{\n  // note\n  \"a\": 1\n}");
        assert!(c.contains(&("comment", "// note".into())));
        assert!(c.iter().any(|(k, _)| *k == "number"));
    }

    #[test]
    fn a_broken_file_still_yields_spans() {
        let c = captures(Language::Python, "def (:\n  return 1 # c");
        assert!(c.iter().any(|(k, _)| *k == "comment"));
    }
}
#[cfg(test)]
mod every_grammar {
    use super::*;
    // Each grammar against a sample of what it is for, in one sequence
    // through one `Highlighting`: TypeScript once came back empty, because
    // its query only adds to JavaScript's and nothing had joined the two.
    #[test]
    fn every_grammar_yields_spans_on_a_sample() {
        let samples = [
            (Language::Rust, "fn main() { let s = \"x\"; } // c"),
            (Language::JavaScript, "function f() { return 'x'; } // c"),
            (
                Language::TypeScript,
                "function f(): string { return 'x'; } // c",
            ),
            (Language::Tsx, "const a = <div>{'x'}</div>; // c"),
            (Language::Python, "def f():\n    return 'x'  # c"),
            (Language::Json, "{\n  \"a\": 1\n}"),
            (Language::Toml, "[a]\nb = \"x\" # c"),
            (Language::Yaml, "a: 1 # c\nb: \"x\""),
            (Language::Bash, "echo \"x\" # c\nif true; then :; fi"),
            (Language::Css, "a { color: red; } /* c */"),
            (Language::Html, "<div class=\"a\">x</div><!-- c -->"),
            (
                Language::Go,
                "package main\nfunc main() { s := \"x\" } // c",
            ),
            (Language::C, "int main() { return 1; } // c"),
            (Language::Markdown, "# Title\n\nsome *text* and `code`\n"),
            (
                Language::Svelte,
                "<script>let a = 1;</script>\n<div>{a}</div>",
            ),
            (
                Language::Sql,
                "SELECT id, name FROM users WHERE id = 1; -- c",
            ),
            #[cfg(not(windows))]
            (Language::Scss, "$c: red;\n.a { .b { color: $c; } } // c"),
        ];
        let mut h = Highlighting::default();
        let mut empty = vec![];
        for (l, text) in samples {
            if h.spans(l, text).is_empty() {
                empty.push(l);
            }
        }
        assert!(empty.is_empty(), "no spans for {empty:?}");
    }
}
