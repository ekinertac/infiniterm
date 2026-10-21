//! What an editor card's status bar says: the file on the left, and on the
//! right where the caret is and what the file is (language, indentation,
//! encoding, line endings), the strip every editor people come from has.
//!
//! Pure: the gpui side (`infiniterm-ui/src/editor_body.rs`) gathers the
//! facts into a `Status`, lays out what `fields` returns, and lets the text
//! be selected and copied. The right side is a list of fields rather than
//! one joined string, so the painter can space them and a double-click can
//! select one field whole.
use crate::language::Language;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The file's absolute path; `None` for an untitled buffer.
    pub path: Option<String>,
    /// 1-based, as every editor counts them.
    pub line: usize,
    pub col: usize,
    pub language: Option<Language>,
    /// Characters selected across every cursor.
    pub selected: usize,
    pub cursors: usize,
    /// The indent the editor inserts: `INDENT` in `buffer.rs`.
    pub indent: Indent,
    /// The file on disk had Windows line endings.
    pub crlf: bool,
    pub read_only: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Indent {
    #[default]
    Spaces2,
    Spaces4,
    Tabs,
}

impl Language {
    /// The name people call it by, for the status bar.
    pub fn display_name(self) -> &'static str {
        match self {
            Language::Rust => "Rust",
            Language::JavaScript => "JavaScript",
            Language::TypeScript => "TypeScript",
            Language::Tsx => "TSX",
            Language::Python => "Python",
            Language::Json => "JSON",
            Language::Toml => "TOML",
            Language::Yaml => "YAML",
            Language::Bash => "Shell",
            Language::Css => "CSS",
            Language::Html => "HTML",
            Language::Go => "Go",
            Language::C => "C",
            Language::Markdown => "Markdown",
            Language::Svelte => "Svelte",
            Language::Sql => "SQL",
            Language::Scss => "SCSS",
        }
    }
}

/// The left side: the path, home shortened to `~`, then whether it is
/// read-only. `home` is the user's home directory without a trailing slash.
pub fn left(s: &Status, home: &str) -> String {
    let mut out = match &s.path {
        Some(p) if !home.is_empty() && (p == home || p.starts_with(&format!("{home}/"))) => {
            format!("~{}", &p[home.len()..])
        }
        Some(p) => p.clone(),
        None => "untitled".to_string(),
    };
    if s.read_only {
        out.push_str("  read-only");
    }
    out
}

/// The right side, field by field, left to right.
pub fn right(s: &Status) -> Vec<String> {
    let mut out = vec![format!("Ln {}, Col {}", s.line, s.col)];
    if s.selected > 0 {
        out.push(format!("{} selected", s.selected));
    }
    if s.cursors > 1 {
        out.push(format!("{} cursors", s.cursors));
    }
    out.push(
        match s.indent {
            Indent::Spaces2 => "Spaces: 2",
            Indent::Spaces4 => "Spaces: 4",
            Indent::Tabs => "Tabs",
        }
        .to_string(),
    );
    out.push("UTF-8".to_string());
    out.push(if s.crlf { "CRLF" } else { "LF" }.to_string());
    out.push(
        s.language
            .map(Language::display_name)
            .unwrap_or("Plain Text")
            .to_string(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> Status {
        Status {
            path: Some("/Users/me/Code/app/src/main.rs".into()),
            line: 12,
            col: 4,
            language: Some(Language::Rust),
            cursors: 1,
            ..Default::default()
        }
    }

    #[test]
    fn the_path_shortens_home_to_a_tilde() {
        assert_eq!(left(&status(), "/Users/me"), "~/Code/app/src/main.rs");
        // A sibling of home is not home.
        let mut s = status();
        s.path = Some("/Users/meg/x.rs".into());
        assert_eq!(left(&s, "/Users/me"), "/Users/meg/x.rs");
    }

    #[test]
    fn untitled_and_read_only_say_so() {
        let mut s = status();
        s.path = None;
        assert_eq!(left(&s, "/Users/me"), "untitled");
        s.path = Some("/Users/me/a.md".into());
        s.read_only = true;
        assert_eq!(left(&s, "/Users/me"), "~/a.md  read-only");
    }

    #[test]
    fn the_right_side_is_position_then_what_the_file_is() {
        assert_eq!(
            right(&status()),
            ["Ln 12, Col 4", "Spaces: 2", "UTF-8", "LF", "Rust"]
        );
    }

    #[test]
    fn a_selection_and_extra_cursors_appear_only_when_there() {
        let mut s = status();
        s.selected = 34;
        s.cursors = 3;
        assert_eq!(
            &right(&s)[..3],
            ["Ln 12, Col 4", "34 selected", "3 cursors"]
        );
    }

    #[test]
    fn plain_text_and_crlf() {
        let mut s = status();
        s.language = None;
        s.crlf = true;
        let r = right(&s);
        assert!(r.contains(&"CRLF".to_string()));
        assert_eq!(r.last().map(String::as_str), Some("Plain Text"));
    }
}
