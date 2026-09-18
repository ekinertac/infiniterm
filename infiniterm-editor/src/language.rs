//! Which grammar reads a file, from its extension or name. Port of the
//! reference's `languageFor` and its `ALIASES` table: the shell family and
//! dotfiles go to Bash, `Cargo.lock` is TOML, JSON files use the JavaScript
//! grammar because the config files are JSONC and a JSON grammar stops at
//! the first `//`, and a miss is plain text, which is honest where a wrong
//! grammar is not. The badge name is what a reader calls the file (`json`,
//! not `javascript`), and the comment token is what Cmd+/ inserts.
//!
//! Grammars are compiled in, the twenty the mapping asked for first; a
//! file of another kind is plain text until its grammar is added here.
use tree_sitter_highlight::HighlightConfiguration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    JavaScript,
    TypeScript,
    Tsx,
    Python,
    Json,
    Toml,
    Yaml,
    Bash,
    Css,
    Html,
    Go,
    C,
    Markdown,
    Svelte,
}

/// The capture names the theme knows (`editor_theme.rs`); anything a
/// grammar's query names outside this list paints as plain text.
pub const CAPTURES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "embedded",
    "function",
    "function.builtin",
    "function.method",
    "keyword",
    "label",
    "number",
    "operator",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "string",
    "string.special",
    "tag",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
    "text.title",
    "text.literal",
    "text.uri",
    "text.emphasis",
    "text.strong",
];

impl Language {
    /// From the file's path; `None` is plain text.
    pub fn for_path(path: &str) -> Option<Language> {
        let name = path.rsplit('/').next().unwrap_or(path);
        let ext = name.rsplit('.').next().filter(|e| *e != name).unwrap_or("");
        let ext = ext.to_ascii_lowercase();
        if name == "Cargo.lock" || name == "Pipfile" {
            return Some(Language::Toml);
        }
        if name == "yarn.lock" {
            return Some(Language::Yaml);
        }
        if name == "Dockerfile" || name.starts_with("Dockerfile.") {
            return Some(Language::Bash);
        }
        if name.starts_with('.')
            && matches!(
                name.trim_start_matches('.').split('.').next().unwrap_or(""),
                "zshrc" | "zshenv" | "zprofile" | "bashrc" | "bash_profile" | "profile" | "env"
            )
        {
            return Some(Language::Bash);
        }
        Some(match ext.as_str() {
            "rs" => Language::Rust,
            "js" | "mjs" | "cjs" | "jsx" => Language::JavaScript,
            "json" | "jsonc" | "json5" => Language::Json,
            "ts" | "mts" | "cts" => Language::TypeScript,
            "tsx" => Language::Tsx,
            "py" | "pyi" => Language::Python,
            "toml" => Language::Toml,
            "yml" | "yaml" => Language::Yaml,
            "sh" | "bash" | "zsh" | "fish" | "env" | "envrc" => Language::Bash,
            "css" => Language::Css,
            "html" | "htm" | "xml" | "plist" | "svg" => Language::Html,
            "go" => Language::Go,
            "c" | "h" => Language::C,
            "md" | "markdown" => Language::Markdown,
            "svelte" => Language::Svelte,
            _ => return None,
        })
    }

    /// The badge beside the card's name.
    pub fn badge(self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::JavaScript => "javascript",
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
            Language::Python => "python",
            Language::Json => "json",
            Language::Toml => "toml",
            Language::Yaml => "yaml",
            Language::Bash => "shell",
            Language::Css => "css",
            Language::Html => "html",
            Language::Go => "go",
            Language::C => "c",
            Language::Markdown => "markdown",
            Language::Svelte => "svelte",
        }
    }

    /// What Cmd+/ puts in front of a line.
    pub fn comment_token(self) -> &'static str {
        match self {
            Language::Python | Language::Toml | Language::Yaml | Language::Bash => "#",
            Language::Css => "/*",
            Language::Html | Language::Svelte | Language::Markdown => "<!--",
            _ => "//",
        }
    }

    /// Whether the file is a picture the editor shows instead of reading:
    /// the formats gpui decodes (the `image` crate plus resvg). Anything
    /// else with a picture's name is still read as text, which is honest
    /// where a blank pane is not.
    pub fn is_image(path: &str) -> bool {
        let lower = path.to_ascii_lowercase();
        let name = lower.rsplit('/').next().unwrap_or(&lower);
        match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => [
                "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tiff", "tif", "svg",
            ]
            .contains(&ext),
            _ => false,
        }
    }

    /// Whether the file is prose, for `editor.wrap: "prose"`.
    pub fn is_prose(path: &str) -> bool {
        let lower = path.to_ascii_lowercase();
        [".md", ".markdown", ".txt", ".rst", ".adoc"]
            .iter()
            .any(|e| lower.ends_with(e))
    }

    /// The grammar and its highlight query, ready to run. Built once per
    /// language by the caller and kept: a configuration is a compiled
    /// query and costs a few ms.
    pub fn highlight_config(self) -> Option<HighlightConfiguration> {
        let (lang, highlights, injections, locals): (tree_sitter::Language, &str, &str, &str) =
            match self {
                Language::Rust => (
                    tree_sitter_rust::LANGUAGE.into(),
                    tree_sitter_rust::HIGHLIGHTS_QUERY,
                    tree_sitter_rust::INJECTIONS_QUERY,
                    "",
                ),
                Language::JavaScript | Language::Json => (
                    tree_sitter_javascript::LANGUAGE.into(),
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::INJECTIONS_QUERY,
                    tree_sitter_javascript::LOCALS_QUERY,
                ),
                Language::TypeScript => (
                    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                    "",
                    tree_sitter_typescript::LOCALS_QUERY,
                ),
                Language::Tsx => (
                    tree_sitter_typescript::LANGUAGE_TSX.into(),
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                    "",
                    tree_sitter_typescript::LOCALS_QUERY,
                ),
                Language::Python => (
                    tree_sitter_python::LANGUAGE.into(),
                    tree_sitter_python::HIGHLIGHTS_QUERY,
                    "",
                    "",
                ),
                Language::Toml => (
                    tree_sitter_toml_ng::LANGUAGE.into(),
                    tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
                    "",
                    "",
                ),
                Language::Yaml => (
                    tree_sitter_yaml::LANGUAGE.into(),
                    tree_sitter_yaml::HIGHLIGHTS_QUERY,
                    "",
                    "",
                ),
                Language::Bash => (
                    tree_sitter_bash::LANGUAGE.into(),
                    tree_sitter_bash::HIGHLIGHT_QUERY,
                    "",
                    "",
                ),
                Language::Css => (
                    tree_sitter_css::LANGUAGE.into(),
                    tree_sitter_css::HIGHLIGHTS_QUERY,
                    "",
                    "",
                ),
                Language::Html => (
                    tree_sitter_html::LANGUAGE.into(),
                    tree_sitter_html::HIGHLIGHTS_QUERY,
                    tree_sitter_html::INJECTIONS_QUERY,
                    "",
                ),
                Language::Go => (
                    tree_sitter_go::LANGUAGE.into(),
                    tree_sitter_go::HIGHLIGHTS_QUERY,
                    "",
                    "",
                ),
                Language::C => (
                    tree_sitter_c::LANGUAGE.into(),
                    tree_sitter_c::HIGHLIGHT_QUERY,
                    "",
                    "",
                ),
                Language::Markdown => (
                    tree_sitter_md::LANGUAGE.into(),
                    tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
                    tree_sitter_md::INJECTION_QUERY_BLOCK,
                    "",
                ),
                Language::Svelte => (
                    tree_sitter_svelte_ng::LANGUAGE.into(),
                    tree_sitter_svelte_ng::HIGHLIGHTS_QUERY,
                    tree_sitter_svelte_ng::INJECTIONS_QUERY,
                    "",
                ),
            };
        let mut config =
            HighlightConfiguration::new(lang, self.badge(), highlights, injections, locals).ok()?;
        config.configure(CAPTURES);
        Some(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_pick_their_grammar_and_dotfiles_are_shell() {
        assert_eq!(Language::for_path("src/main.rs"), Some(Language::Rust));
        assert_eq!(Language::for_path("~/.zshrc"), Some(Language::Bash));
        assert_eq!(Language::for_path("/x/.env.local"), Some(Language::Bash));
        assert_eq!(Language::for_path("Cargo.lock"), Some(Language::Toml));
        assert_eq!(Language::for_path("settings.json"), Some(Language::Json));
        assert_eq!(Language::for_path("Dockerfile.dev"), Some(Language::Bash));
        assert_eq!(Language::for_path("notes"), None);
        assert_eq!(Language::for_path("a.unknownext"), None);
    }

    #[test]
    fn json_borrows_javascript_but_keeps_its_name() {
        assert_eq!(Language::Json.badge(), "json");
        assert_eq!(Language::Json.comment_token(), "//");
        assert!(Language::is_prose("README.md"));
        assert!(!Language::is_prose("main.rs"));
    }

    // Case-blind, on the last name only: `.png` alone and a directory
    // called `png.d` are not pictures.
    #[test]
    fn a_picture_is_known_by_its_extension() {
        assert!(Language::is_image("/tmp/shots/01.png"));
        assert!(Language::is_image("/tmp/Photo.JPG"));
        assert!(Language::is_image("logo.svg"));
        assert!(!Language::is_image("/tmp/.png"));
        assert!(!Language::is_image("/tmp/png.d/notes"));
        assert!(!Language::is_image("main.rs"));
    }

    #[test]
    fn every_grammar_compiles_its_query() {
        for l in [
            Language::Rust,
            Language::JavaScript,
            Language::TypeScript,
            Language::Tsx,
            Language::Python,
            Language::Json,
            Language::Toml,
            Language::Yaml,
            Language::Bash,
            Language::Css,
            Language::Html,
            Language::Go,
            Language::C,
            Language::Markdown,
            Language::Svelte,
        ] {
            assert!(l.highlight_config().is_some(), "{l:?}");
        }
    }
}
