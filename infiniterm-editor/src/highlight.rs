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
