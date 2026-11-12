//! The named things in a source file, for Go to Symbol (#602): functions,
//! types, classes, methods, headings, found by walking the tree-sitter parse
//! the highlighter already depends on, no language server. Each language has
//! a short table of the node kinds that declare something and how that kind
//! is called; the name is the node's `name` field, with the few shapes that
//! keep it elsewhere (a Rust `impl`, a C declarator, a markdown heading, a
//! TOML table) handled by hand.
//!
//! Called by `infiniterm-ui/src/editors.rs` when the model asks
//! (`EditorAction::GoToSymbol`); the rows go to `Model::open_symbols`. Pure:
//! text in, rows out. A language with no table here (HTML, CSS, JSON, YAML,
//! SQL, Svelte: declarations are not what you look for in them) has none.

use crate::language::Language;
use tree_sitter::{Node, Parser};

/// One symbol: what it is called, what kind of thing it is, where it starts
/// and ends (1-based lines, the end inclusive: sticky scroll pins a header
/// while its body is on screen, `decor::sticky_headers`) and how deep it
/// sits inside others.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: &'static str,
    pub line: usize,
    pub end_line: usize,
    pub depth: usize,
}

/// The node kinds that declare a symbol in `lang`, with how each is called.
fn kinds(lang: Language) -> &'static [(&'static str, &'static str)] {
    match lang {
        Language::Rust => &[
            ("function_item", "fn"),
            ("struct_item", "struct"),
            ("enum_item", "enum"),
            ("union_item", "union"),
            ("trait_item", "trait"),
            ("impl_item", "impl"),
            ("mod_item", "mod"),
            ("type_item", "type"),
            ("const_item", "const"),
            ("static_item", "static"),
            ("macro_definition", "macro"),
        ],
        Language::JavaScript => &[
            ("function_declaration", "function"),
            ("generator_function_declaration", "function"),
            ("class_declaration", "class"),
            ("method_definition", "method"),
            ("variable_declarator", "var"),
        ],
        Language::TypeScript | Language::Tsx => &[
            ("function_declaration", "function"),
            ("generator_function_declaration", "function"),
            ("class_declaration", "class"),
            ("abstract_class_declaration", "class"),
            ("method_definition", "method"),
            ("interface_declaration", "interface"),
            ("type_alias_declaration", "type"),
            ("enum_declaration", "enum"),
            ("variable_declarator", "var"),
        ],
        Language::Python => &[
            ("function_definition", "def"),
            ("class_definition", "class"),
        ],
        Language::Go => &[
            ("function_declaration", "func"),
            ("method_declaration", "method"),
            ("type_spec", "type"),
        ],
        Language::C => &[
            ("function_definition", "fn"),
            ("struct_specifier", "struct"),
            ("enum_specifier", "enum"),
            ("union_specifier", "union"),
        ],
        Language::Bash => &[("function_definition", "function")],
        Language::Markdown => &[("atx_heading", "heading"), ("setext_heading", "heading")],
        Language::Toml => &[("table", "table"), ("table_array_element", "table")],
        _ => &[],
    }
}

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

/// The identifier a C declarator wraps: `int *f(int a)` is `f`.
fn c_declarator_name<'a>(mut node: Node<'a>, src: &'a str) -> Option<&'a str> {
    while let Some(next) = node.child_by_field_name("declarator") {
        node = next;
    }
    matches!(node.kind(), "identifier" | "field_identifier").then(|| text(node, src))
}

/// The name of `node`, or `None` when it is not a symbol after all (an
/// anonymous struct, a variable that holds no function).
fn name_of(lang: Language, node: Node, src: &str) -> Option<String> {
    let kind = node.kind();
    let name = match (lang, kind) {
        // `impl Trait for Type` and `impl Type`: the type it is for.
        (Language::Rust, "impl_item") => {
            let ty = node.child_by_field_name("type").map(|n| text(n, src))?;
            match node.child_by_field_name("trait").map(|n| text(n, src)) {
                Some(tr) => format!("{tr} for {ty}"),
                None => ty.to_string(),
            }
        }
        (Language::C, "function_definition") => c_declarator_name(node, src)?.to_string(),
        // A `const f = () => {}` is a function; `const x = 1` is not.
        (Language::JavaScript | Language::TypeScript | Language::Tsx, "variable_declarator") => {
            let value = node.child_by_field_name("value")?;
            if !matches!(
                value.kind(),
                "arrow_function" | "function_expression" | "function" | "generator_function"
            ) {
                return None;
            }
            text(node.child_by_field_name("name")?, src).to_string()
        }
        (Language::Markdown, "atx_heading" | "setext_heading") => {
            let mut cursor = node.walk();
            let inline = node
                .named_children(&mut cursor)
                .find(|c| c.kind() == "inline" || c.kind() == "paragraph")?;
            let t = text(inline, src).trim();
            // The level is part of what a heading is.
            let level = match node.kind() {
                "setext_heading" => 1,
                _ => node
                    .named_children(&mut node.walk())
                    .find_map(|c| {
                        let k = c.kind();
                        k.strip_prefix("atx_h")
                            .and_then(|r| r.strip_suffix("_marker"))
                            .and_then(|n| n.parse::<usize>().ok())
                    })
                    .unwrap_or(1),
            };
            format!("{} {t}", "#".repeat(level))
        }
        (Language::Toml, "table" | "table_array_element") => {
            let key = node.named_child(0)?;
            text(key, src).trim().to_string()
        }
        _ => text(node.child_by_field_name("name")?, src).to_string(),
    };
    let name = name.trim();
    (!name.is_empty()).then(|| name.lines().next().unwrap_or(name).to_string())
}

/// The node's last line, 1-based. A node that takes its line's newline
/// ends at column 0 of the NEXT line, which is not part of it.
fn end_line(node: Node) -> usize {
    let end = node.end_position();
    if end.column == 0 && end.row > node.start_position().row {
        end.row
    } else {
        end.row + 1
    }
}

fn walk(lang: Language, node: Node, src: &str, depth: usize, out: &mut Vec<Symbol>) {
    let table = kinds(lang);
    let found = table
        .iter()
        .find(|(k, _)| *k == node.kind())
        .and_then(|(_, label)| name_of(lang, node, src).map(|name| (*label, name)));
    let child_depth = match found {
        Some((kind, name)) => {
            // A heading's depth is its level; the tree nests sections, not
            // the headings inside one another.
            let own_depth = if lang == Language::Markdown {
                name.chars()
                    .take_while(|c| *c == '#')
                    .count()
                    .saturating_sub(1)
            } else {
                depth
            };
            out.push(Symbol {
                name,
                kind: if lang == Language::Markdown { "" } else { kind },
                line: node.start_position().row + 1,
                end_line: end_line(node),
                depth: own_depth,
            });
            depth + 1
        }
        None => depth,
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(lang, child, src, child_depth, out);
    }
}

/// Every symbol of `text`, in document order, or none for a language with no
/// table or a parse that fails.
pub fn symbols(lang: Language, text: &str) -> Vec<Symbol> {
    if kinds(lang).is_empty() {
        return vec![];
    }
    let Some(grammar) = lang.grammar() else {
        return vec![];
    };
    let mut parser = Parser::new();
    if parser.set_language(&grammar).is_err() {
        return vec![];
    }
    let Some(tree) = parser.parse(text, None) else {
        return vec![];
    };
    let mut out = vec![];
    walk(lang, tree.root_node(), text, 0, &mut out);
    if lang == Language::Markdown {
        // The node is the heading line alone; its section runs to the line
        // before the next heading at the same level or above.
        let last = text.lines().count().max(1);
        for i in 0..out.len() {
            out[i].end_line = out[i + 1..]
                .iter()
                .find(|s| s.depth <= out[i].depth)
                .map_or(last, |s| s.line - 1);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(lang: Language, src: &str) -> Vec<(String, &'static str, usize, usize)> {
        symbols(lang, src)
            .into_iter()
            .map(|s| (s.name, s.kind, s.line, s.depth))
            .collect()
    }

    #[test]
    fn rust_items_nest_and_impls_name_their_type() {
        let src = "mod m {\n    pub fn a() {}\n}\nstruct S;\nimpl S {\n    fn b(&self) {}\n}\nimpl Drop for S {\n    fn drop(&mut self) {}\n}\n";
        let got = names(Language::Rust, src);
        let want = |n: &str, k: &'static str, l, d| (n.to_string(), k, l, d);
        assert_eq!(
            got,
            vec![
                want("m", "mod", 1, 0),
                want("a", "fn", 2, 1),
                want("S", "struct", 4, 0),
                want("S", "impl", 5, 0),
                want("b", "fn", 6, 1),
                want("Drop for S", "impl", 8, 0),
                want("drop", "fn", 9, 1),
            ]
        );
    }

    #[test]
    fn python_defs_and_classes() {
        let src = "def top():\n    pass\n\nclass K:\n    def m(self):\n        pass\n";
        let got: Vec<_> = names(Language::Python, src)
            .into_iter()
            .map(|(n, k, l, d)| format!("{k} {n} {l} {d}"))
            .collect();
        assert_eq!(got, ["def top 1 0", "class K 4 0", "def m 5 1"]);
    }

    #[test]
    fn javascript_functions_classes_and_arrow_constants_but_not_plain_values() {
        let src = "function f() {}\nconst g = () => 1;\nconst n = 5;\nclass C {\n  m() {}\n}\n";
        let got: Vec<_> = names(Language::JavaScript, src)
            .into_iter()
            .map(|(n, k, ..)| format!("{k} {n}"))
            .collect();
        assert_eq!(got, ["function f", "var g", "class C", "method m"]);
    }

    #[test]
    fn typescript_adds_interfaces_types_and_enums() {
        let src = "interface I { a: number }\ntype T = string;\nenum E { A }\n";
        let got: Vec<_> = names(Language::TypeScript, src)
            .into_iter()
            .map(|(n, k, ..)| format!("{k} {n}"))
            .collect();
        assert_eq!(got, ["interface I", "type T", "enum E"]);
    }

    #[test]
    fn go_and_c_find_their_functions() {
        let go = "package p\nfunc F() {}\ntype T struct{}\nfunc (t T) M() {}\n";
        let got: Vec<_> = names(Language::Go, go)
            .into_iter()
            .map(|(n, k, ..)| format!("{k} {n}"))
            .collect();
        assert_eq!(got, ["func F", "type T", "method M"]);
        let c = "int add(int a, int b) { return a + b; }\nstatic char *name(void) { return 0; }\n";
        let got: Vec<_> = names(Language::C, c).into_iter().map(|(n, ..)| n).collect();
        assert_eq!(got, ["add", "name"]);
    }

    #[test]
    fn markdown_headings_carry_their_level() {
        let src = "# Title\n\ntext\n\n## Part\n\nmore\n\n### Deep\n";
        let got: Vec<_> = names(Language::Markdown, src)
            .into_iter()
            .map(|(n, _, l, d)| format!("{n}@{l}/{d}"))
            .collect();
        assert_eq!(got, ["# Title@1/0", "## Part@5/1", "### Deep@9/2"]);
    }

    #[test]
    fn toml_tables_and_shell_functions() {
        let toml = "[package]\nname = \"x\"\n\n[[bin]]\nname = \"y\"\n";
        let got: Vec<_> = names(Language::Toml, toml)
            .into_iter()
            .map(|(n, ..)| n)
            .collect();
        assert_eq!(got, ["package", "bin"]);
        let sh = "build() {\n  echo hi\n}\nfunction clean {\n  true\n}\n";
        let got: Vec<_> = names(Language::Bash, sh)
            .into_iter()
            .map(|(n, ..)| n)
            .collect();
        assert_eq!(got, ["build", "clean"]);
    }

    #[test]
    fn a_language_without_a_table_has_no_symbols() {
        assert!(symbols(Language::Json, "{\"a\": 1}").is_empty());
        assert!(symbols(Language::Rust, "").is_empty());
    }

    #[test]
    fn a_symbol_ends_on_its_own_last_line() {
        let ends = |lang, src| -> Vec<(usize, usize)> {
            symbols(lang, src)
                .into_iter()
                .map(|s| (s.line, s.end_line))
                .collect()
        };
        let rust = "impl S {\n    fn a() {\n        x();\n    }\n}\n";
        assert_eq!(ends(Language::Rust, rust), vec![(1, 5), (2, 4)]);
        // A section takes its trailing newline; the next heading is not in it.
        let md = "# A\ntext\n## B\nmore\n# C\n";
        assert_eq!(
            ends(Language::Markdown, md),
            vec![(1, 4), (3, 4), (5, 5)]
        );
    }
}
