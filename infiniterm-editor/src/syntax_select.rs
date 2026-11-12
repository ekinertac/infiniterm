//! Expand the selection by syntax node (#778): from the caret to the word,
//! the call, the statement, the block, the function, one press at a time,
//! by walking up the tree-sitter parse the highlighter already depends on.
//! Shrinking back is the caller's: it keeps the selections it expanded
//! from, since a parent node does not say which child it came from.
//!
//! Called by `infiniterm-ui/src/editor_body.rs` for `editor.selection.expand`
//! (`EditorAction::ExpandSelection`). Parses on the press only, never per
//! frame. Pure: text and a char range in, a char range out. A language with
//! no grammar has nothing to expand to.
use crate::language::Language;
use std::ops::Range;
use tree_sitter::Parser;

/// Char offset to byte offset in `text`, clamped to its end.
fn to_byte(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(b, _)| b)
}

fn to_char(text: &str, bytes: usize) -> usize {
    text[..bytes.min(text.len())].chars().count()
}

/// The smallest named node that holds `sel` and is larger than it, as a
/// char range. An empty selection grows to the node the caret is in (the
/// word, usually). `None` when the selection is already the whole parse or
/// the language has no grammar.
pub fn expand(lang: Language, text: &str, sel: Range<usize>) -> Option<Range<usize>> {
    let grammar = lang.grammar()?;
    let mut parser = Parser::new();
    parser.set_language(&grammar).ok()?;
    let tree = parser.parse(text, None)?;
    let (a, b) = (to_byte(text, sel.start), to_byte(text, sel.end));
    let mut node = tree.root_node().named_descendant_for_byte_range(a, b)?;
    loop {
        let r = node.byte_range();
        // Larger, and not just the same text with whitespace round it.
        let grown = (r.start < a || r.end > b) && text[r.clone()].trim() != text[a..b].trim();
        if grown && r.start <= a && r.end >= b {
            return Some(to_char(text, r.start)..to_char(text, r.end));
        }
        node = node.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The selected text after expanding `sel` once.
    fn grow(lang: Language, src: &str, sel: Range<usize>) -> Option<String> {
        expand(lang, src, sel).map(|r| src.chars().skip(r.start).take(r.len()).collect())
    }

    #[test]
    fn grows_from_the_caret_outward_one_node_at_a_time() {
        let src = "fn main() {\n    let x = foo(a, b);\n}\n";
        let caret = src.find("foo").unwrap() + 1;
        let word = expand(Language::Rust, src, caret..caret).unwrap();
        assert_eq!(&src[word.clone()], "foo");
        let call = expand(Language::Rust, src, word).unwrap();
        assert_eq!(&src[call.clone()], "foo(a, b)");
        let stmt = grow(Language::Rust, src, call).unwrap();
        assert_eq!(stmt, "let x = foo(a, b);");
    }

    #[test]
    fn the_whole_file_has_nothing_left_to_grow_to() {
        let src = "fn a() {}";
        assert_eq!(expand(Language::Rust, src, 0..src.len()), None);
    }

    #[test]
    fn char_offsets_survive_multibyte_text() {
        // The caret on the second name, past a string of two-byte letters:
        // byte and char offsets differ from there on.
        let src = "let s = \"ğüş\"; let tt = 1;";
        let at = src[..src.find("tt").unwrap()].chars().count() + 1;
        assert_eq!(grow(Language::JavaScript, src, at..at).as_deref(), Some("tt"));
    }
}
