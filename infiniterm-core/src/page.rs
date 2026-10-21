//! The Page card's document (#42): Markdown parsed into blocks of styled
//! spans (`parse`), and blocks laid out into wrapped lines for a width in
//! cells (`layout`). Pure, so the shape of a page is tested here; the ui's
//! `page_body.rs` paints the lines and handles the scroll and the links.
//!
//! A Page is a read-only rendered document that never takes the keyboard,
//! made for the welcome card (the read-only editor it was first had to
//! either lock, which trapped newcomers, or not, which left the docs with no
//! keys at all, #41). Monospace on purpose: the page is drawn in the
//! terminal font like everything else on a card, so wrapping is counting
//! cells and a code block lines up.
//!
//! Parsed with pulldown-cmark, without its extensions: headings, emphasis,
//! inline code, code blocks, lists, quotes, rules and links are what the
//! welcome card and the docs use. Anything else (tables, HTML) arrives as
//! its text.
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
    /// Where the span links to, as written in the document.
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Heading(u8),
    Paragraph,
    /// A list item: its nesting depth (0 at the top), its marker ("•" or
    /// "3."), and which top-level list it belongs to, so two lists in a row
    /// keep a blank line between them.
    Item {
        depth: usize,
        marker: String,
        list: usize,
    },
    Code,
    Quote,
    Rule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub spans: Vec<Span>,
}

/// What a laid-out line is, for the painter's colour and weight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineKind {
    Heading(u8),
    Body,
    Code,
    Quote,
    Rule,
    Blank,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    /// Cells before the first span.
    pub indent: usize,
    pub spans: Vec<Span>,
}

impl Line {
    /// The link under cell `col`, if any.
    pub fn link_at(&self, col: usize) -> Option<&str> {
        let mut at = self.indent;
        for s in &self.spans {
            let w = s.text.chars().count();
            if col >= at && col < at + w {
                return s.link.as_deref();
            }
            at += w;
        }
        None
    }
}

/// The document as blocks.
pub fn parse(md: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = vec![];
    let mut style = Style::default();
    let mut link: Option<String> = None;
    // One entry per open list: the next number, or None for bullets.
    let mut lists: Vec<Option<u64>> = vec![];
    let mut in_item = false;
    let mut top_lists = 0usize;
    let mut quote = false;
    let push_text = |blocks: &mut Vec<Block>, text: &str, style: Style, link: &Option<String>| {
        if let Some(b) = blocks.last_mut() {
            b.spans.push(Span {
                text: text.to_string(),
                style,
                link: link.clone(),
            });
        }
    };
    for event in Parser::new(md) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => blocks.push(Block {
                kind: BlockKind::Heading(heading_level(level)),
                spans: vec![],
            }),
            Event::Start(Tag::Paragraph) => {
                // A list item's text is its own block; a quote's is a quote.
                if !in_item {
                    blocks.push(Block {
                        kind: if quote {
                            BlockKind::Quote
                        } else {
                            BlockKind::Paragraph
                        },
                        spans: vec![],
                    });
                }
            }
            Event::Start(Tag::List(start)) => {
                if lists.is_empty() {
                    top_lists += 1;
                }
                lists.push(start)
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                let depth = lists.len().saturating_sub(1);
                let marker = match lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}.");
                        *n += 1;
                        m
                    }
                    _ => "•".to_string(),
                };
                blocks.push(Block {
                    kind: BlockKind::Item {
                        depth,
                        marker,
                        list: top_lists,
                    },
                    spans: vec![],
                });
                in_item = true;
            }
            Event::End(TagEnd::Item) => in_item = false,
            Event::Start(Tag::CodeBlock(_)) => blocks.push(Block {
                kind: BlockKind::Code,
                spans: vec![],
            }),
            Event::Start(Tag::BlockQuote(_)) => quote = true,
            Event::End(TagEnd::BlockQuote(_)) => quote = false,
            Event::Start(Tag::Emphasis) => style.italic = true,
            Event::End(TagEnd::Emphasis) => style.italic = false,
            Event::Start(Tag::Strong) => style.bold = true,
            Event::End(TagEnd::Strong) => style.bold = false,
            Event::Start(Tag::Link { dest_url, .. }) => link = Some(dest_url.to_string()),
            Event::End(TagEnd::Link) => link = None,
            Event::Rule => blocks.push(Block {
                kind: BlockKind::Rule,
                spans: vec![],
            }),
            Event::Code(t) => push_text(
                &mut blocks,
                &t,
                Style {
                    code: true,
                    ..style
                },
                &link,
            ),
            Event::Text(t) => push_text(&mut blocks, &t, style, &link),
            Event::SoftBreak => push_text(&mut blocks, " ", style, &link),
            Event::HardBreak => push_text(&mut blocks, "\n", style, &link),
            _ => {}
        }
    }
    blocks
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Cells a list level indents by.
const LIST_INDENT: usize = 2;
/// Cells a quote indents by (its bar is drawn in them).
const QUOTE_INDENT: usize = 2;

/// The blocks as lines of at most `cols` cells: words wrapped greedily, a
/// list item's continuation lines under its text, code blocks never
/// wrapped (a long line runs off and is clipped, the way a terminal shows
/// it), a blank line between blocks except between items of one list.
pub fn layout(blocks: &[Block], cols: usize) -> Vec<Line> {
    let cols = cols.max(8);
    let mut out: Vec<Line> = vec![];
    let mut prev_list: Option<usize> = None;
    for b in blocks {
        let list = match &b.kind {
            BlockKind::Item { list, .. } => Some(*list),
            _ => None,
        };
        if !out.is_empty() && !(list.is_some() && list == prev_list) {
            out.push(Line {
                kind: LineKind::Blank,
                indent: 0,
                spans: vec![],
            });
        }
        prev_list = list;
        match &b.kind {
            BlockKind::Rule => out.push(Line {
                kind: LineKind::Rule,
                indent: 0,
                spans: vec![],
            }),
            BlockKind::Code => {
                let text: String = b.spans.iter().map(|s| s.text.as_str()).collect();
                for l in text.trim_end_matches('\n').split('\n') {
                    out.push(Line {
                        kind: LineKind::Code,
                        indent: LIST_INDENT,
                        spans: vec![Span {
                            text: l.to_string(),
                            style: Style {
                                code: true,
                                ..Style::default()
                            },
                            link: None,
                        }],
                    });
                }
            }
            BlockKind::Item { depth, marker, .. } => {
                let indent = depth * LIST_INDENT;
                let hang = marker.chars().count() + 1;
                let mut lines = wrap(&b.spans, cols.saturating_sub(indent + hang));
                for (i, l) in lines.iter_mut().enumerate() {
                    l.indent = indent + hang;
                    if i == 0 {
                        l.indent = indent;
                        l.spans.insert(
                            0,
                            Span {
                                text: format!("{marker} "),
                                style: Style::default(),
                                link: None,
                            },
                        );
                    }
                }
                out.extend(lines);
            }
            BlockKind::Heading(level) => {
                for mut l in wrap(&b.spans, cols) {
                    l.kind = LineKind::Heading(*level);
                    out.push(l);
                }
            }
            BlockKind::Quote => {
                for mut l in wrap(&b.spans, cols.saturating_sub(QUOTE_INDENT)) {
                    l.kind = LineKind::Quote;
                    l.indent = QUOTE_INDENT;
                    out.push(l);
                }
            }
            BlockKind::Paragraph => out.extend(wrap(&b.spans, cols)),
        }
    }
    out
}

/// Spans broken into body lines of at most `width` cells, at spaces where
/// it can (a word longer than the width is cut), and at hard breaks.
fn wrap(spans: &[Span], width: usize) -> Vec<Line> {
    let width = width.max(4);
    let mut lines: Vec<Line> = vec![];
    let mut cur: Vec<Span> = vec![];
    let mut used = 0usize;
    let flush = |lines: &mut Vec<Line>, cur: &mut Vec<Span>, used: &mut usize| {
        // A line never ends in the space that broke it.
        if let Some(last) = cur.last_mut() {
            let t = last.text.trim_end().to_string();
            last.text = t;
        }
        cur.retain(|s| !s.text.is_empty());
        lines.push(Line {
            kind: LineKind::Body,
            indent: 0,
            spans: std::mem::take(cur),
        });
        *used = 0;
    };
    let push = |cur: &mut Vec<Span>, text: &str, s: &Span| match cur.last_mut() {
        Some(last) if last.style == s.style && last.link == s.link => last.text.push_str(text),
        _ => cur.push(Span {
            text: text.to_string(),
            style: s.style,
            link: s.link.clone(),
        }),
    };
    for s in spans {
        for (i, piece) in s.text.split('\n').enumerate() {
            if i > 0 {
                flush(&mut lines, &mut cur, &mut used);
            }
            // Words with their trailing space, so spacing survives a span
            // boundary ("a **b** c").
            for word in piece.split_inclusive(' ') {
                let mut word = word;
                let w = word.trim_end().chars().count();
                if used > 0 && used + w > width {
                    flush(&mut lines, &mut cur, &mut used);
                    if word.trim().is_empty() {
                        continue;
                    }
                }
                // A word wider than the line is cut into line-sized pieces.
                while word.trim_end().chars().count() > width {
                    let cut: String = word.chars().take(width).collect();
                    push(&mut cur, &cut, s);
                    flush(&mut lines, &mut cur, &mut used);
                    word = &word[cut.len()..];
                }
                if used == 0 && word.trim().is_empty() {
                    continue;
                }
                push(&mut cur, word, s);
                used += word.chars().count();
            }
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        flush(&mut lines, &mut cur, &mut used);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(l: &Line) -> String {
        let mut s = " ".repeat(l.indent);
        for sp in &l.spans {
            s.push_str(&sp.text);
        }
        s
    }

    #[test]
    fn headings_emphasis_code_and_links_keep_their_style() {
        let b =
            parse("# Start here\n\nPress **Cmd T** for a `terminal`, see [docs](help/canvas.md).");
        assert_eq!(b[0].kind, BlockKind::Heading(1));
        assert_eq!(b[0].spans[0].text, "Start here");
        let p = &b[1].spans;
        assert!(p.iter().any(|s| s.text == "Cmd T" && s.style.bold));
        assert!(p.iter().any(|s| s.text == "terminal" && s.style.code));
        assert!(p
            .iter()
            .any(|s| s.text == "docs" && s.link.as_deref() == Some("help/canvas.md")));
    }

    #[test]
    fn paragraphs_wrap_at_spaces_with_a_blank_line_between_blocks() {
        let lines = layout(&parse("one two three four five\n\nsix"), 10);
        let t: Vec<String> = lines.iter().map(text).collect();
        assert_eq!(t, ["one two", "three four", "five", "", "six"]);
    }

    #[test]
    fn list_items_hang_under_their_text_and_number_themselves() {
        let lines = layout(
            &parse("1. first item wraps here\n2. second\n\n- a\n  - nested"),
            16,
        );
        let t: Vec<String> = lines.iter().map(text).collect();
        assert_eq!(
            t,
            [
                "1. first item",
                "   wraps here",
                "2. second",
                "",
                "• a",
                "  • nested"
            ]
        );
    }

    #[test]
    fn code_blocks_keep_their_lines_and_are_not_wrapped() {
        let lines = layout(
            &parse("```\nift install-codex-hooks --dry-run\nls\n```"),
            10,
        );
        assert!(lines.iter().all(|l| l.kind == LineKind::Code));
        assert_eq!(text(&lines[0]), "  ift install-codex-hooks --dry-run");
        assert_eq!(text(&lines[1]), "  ls");
    }

    #[test]
    fn spacing_survives_a_style_change_mid_sentence() {
        let lines = layout(&parse("a **bold** word"), 40);
        assert_eq!(text(&lines[0]), "a bold word");
    }

    #[test]
    fn the_link_under_a_cell_is_found() {
        let lines = layout(&parse("see [the docs](d.md) now"), 40);
        let l = &lines[0];
        assert_eq!(l.link_at(4), Some("d.md"));
        assert_eq!(l.link_at(1), None);
    }

    #[test]
    fn a_word_longer_than_the_line_is_cut() {
        let lines = layout(&parse("abcdefghijklmnop"), 8);
        let t: Vec<String> = lines.iter().map(text).collect();
        assert_eq!(t, ["abcdefgh", "ijklmnop"]);
    }
}
