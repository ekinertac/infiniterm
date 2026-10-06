//! The `when` clause of a keybinding (#269): a small expression over a fixed
//! set of context keys, evaluated each time a chord is looked up. VS Code's
//! idea and, where the concept matches, its names (`editorTextFocus`,
//! `suggestWidgetVisible`, `findWidgetVisible`), so a snippet copied from
//! there mostly reads the same.
//!
//! Called by `keymap.rs` (parsing a binding's `when` and the terminal safety
//! check) and `model/` (`Model::key_context`, `conditional_for`). Related:
//! `model/register.rs::resolve_chord`, `infiniterm-ui/src/input.rs` (bare
//! keys). Pure, no model: the model builds a `Context` and asks.
//!
//! Grammar: `||` binds loosest, then `&&`, then `!`, then `==` / `!=`;
//! parentheses group; an operand is a context key, a string in single or
//! double quotes, or `true` / `false`. A key alone is its truth: a boolean
//! as it is, a string when it is not empty and not "none". An unknown key is
//! an error at parse time, never a silently false clause, because a typo that
//! disables a binding without a word is the worst way to fail.

use std::fmt;

/// Everything a `when` can ask about, filled by `Model::key_context`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    /// "terminal", "editor", "browser", "diff", "transcript", "page" or
    /// "none" (no card focused).
    pub card_kind: &'static str,
    /// The focused editor or browser card holds the keyboard (`locked`).
    pub card_locked: bool,
    /// "none", "palette", "prompt", "omnibox", "shortcuts", "find" or
    /// "switcher".
    pub overlay: &'static str,
    /// An empty slot (a phantom) is what is focused.
    pub phantom_focus: bool,
    /// More than one card is selected.
    pub multi_selection: bool,
    /// The editor's completion popup is open.
    pub suggest_widget_visible: bool,
    /// The editor's find panel, or a card's find bar, is open.
    pub find_widget_visible: bool,
    /// The editor has a selection.
    pub editor_has_selection: bool,
}

impl Context {
    /// A focused terminal with nothing open: where a bare key must never be
    /// taken. `keymap.rs` refuses a binding on a bare key whose `when` holds
    /// here.
    pub fn terminal_focused() -> Context {
        Context {
            card_kind: "terminal",
            card_locked: false,
            overlay: "none",
            phantom_focus: false,
            multi_selection: false,
            suggest_widget_visible: false,
            find_widget_visible: false,
            editor_has_selection: false,
        }
    }
}

/// The names a `when` may use and what each means: the source of the error
/// message for an unknown one and of the docs.
pub const KEYS: &[(&str, &str)] = &[
    (
        "cardKind",
        "the focused card's kind: terminal, editor, browser, diff, transcript, page or none",
    ),
    ("terminalFocus", "a terminal card is focused"),
    (
        "editorFocus",
        "an editor card is focused (arrowed to or locked)",
    ),
    (
        "editorTextFocus",
        "an editor card is focused and locked: the keyboard is the file's",
    ),
    ("browserFocus", "a browser card is focused"),
    (
        "cardLocked",
        "the focused editor or browser card holds the keyboard",
    ),
    (
        "overlay",
        "what is open over the canvas: none, palette, prompt, omnibox, shortcuts, find or switcher",
    ),
    (
        "phantomFocus",
        "an empty slot is focused (the phantom the arrows show)",
    ),
    ("multiSelection", "more than one card is selected"),
    (
        "suggestWidgetVisible",
        "the editor's completion popup is open",
    ),
    (
        "findWidgetVisible",
        "a find bar or the editor's find panel is open",
    ),
    ("editorHasSelection", "the editor has text selected"),
];

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Eq(Box<Expr>, Box<Expr>),
    Ne(Box<Expr>, Box<Expr>),
    Key(String),
    Str(String),
    Bool(bool),
}

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Bool(bool),
    Str(String),
}

/// A parsed `when`, with the text it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct When {
    expr: Expr,
    source: String,
}

impl fmt::Display for When {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.source)
    }
}

impl When {
    pub fn parse(source: &str) -> Result<When, String> {
        let tokens = tokenize(source)?;
        let mut p = Parser {
            tokens: &tokens,
            at: 0,
        };
        let expr = p.or()?;
        if p.at != tokens.len() {
            return Err(format!("unexpected `{}`", p.tokens[p.at].text()));
        }
        Ok(When {
            expr,
            source: source.trim().to_string(),
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn eval(&self, ctx: &Context) -> bool {
        truthy(&value(&self.expr, ctx))
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Str(s) => !s.is_empty() && s != "none",
    }
}

fn lookup(key: &str, c: &Context) -> Value {
    match key {
        "cardKind" => Value::Str(c.card_kind.into()),
        "overlay" => Value::Str(c.overlay.into()),
        "terminalFocus" => Value::Bool(c.card_kind == "terminal"),
        "editorFocus" => Value::Bool(c.card_kind == "editor"),
        "editorTextFocus" => Value::Bool(c.card_kind == "editor" && c.card_locked),
        "browserFocus" => Value::Bool(c.card_kind == "browser"),
        "cardLocked" => Value::Bool(c.card_locked),
        "phantomFocus" => Value::Bool(c.phantom_focus),
        "multiSelection" => Value::Bool(c.multi_selection),
        "suggestWidgetVisible" => Value::Bool(c.suggest_widget_visible),
        "findWidgetVisible" => Value::Bool(c.find_widget_visible),
        "editorHasSelection" => Value::Bool(c.editor_has_selection),
        _ => Value::Bool(false), // unreachable: parse refuses an unknown key
    }
}

fn value(e: &Expr, c: &Context) -> Value {
    match e {
        Expr::Or(a, b) => Value::Bool(truthy(&value(a, c)) || truthy(&value(b, c))),
        Expr::And(a, b) => Value::Bool(truthy(&value(a, c)) && truthy(&value(b, c))),
        Expr::Not(a) => Value::Bool(!truthy(&value(a, c))),
        Expr::Eq(a, b) => Value::Bool(value(a, c) == value(b, c)),
        Expr::Ne(a, b) => Value::Bool(value(a, c) != value(b, c)),
        Expr::Key(k) => lookup(k, c),
        Expr::Str(s) => Value::Str(s.clone()),
        Expr::Bool(b) => Value::Bool(*b),
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Ident(String),
    Str(String),
    Or,
    And,
    Not,
    Eq,
    Ne,
    Open,
    Close,
}

impl Token {
    fn text(&self) -> String {
        match self {
            Token::Ident(s) => s.clone(),
            Token::Str(s) => format!("'{s}'"),
            Token::Or => "||".into(),
            Token::And => "&&".into(),
            Token::Not => "!".into(),
            Token::Eq => "==".into(),
            Token::Ne => "!=".into(),
            Token::Open => "(".into(),
            Token::Close => ")".into(),
        }
    }
}

fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            c if c.is_whitespace() => i += 1,
            '(' => {
                out.push(Token::Open);
                i += 1;
            }
            ')' => {
                out.push(Token::Close);
                i += 1;
            }
            '&' if next == Some('&') => {
                out.push(Token::And);
                i += 2;
            }
            '|' if next == Some('|') => {
                out.push(Token::Or);
                i += 2;
            }
            '=' if next == Some('=') => {
                out.push(Token::Eq);
                i += 2;
            }
            '!' if next == Some('=') => {
                out.push(Token::Ne);
                i += 2;
            }
            '!' => {
                out.push(Token::Not);
                i += 1;
            }
            '\'' | '"' => {
                let quote = c;
                let mut j = i + 1;
                let mut s = String::new();
                loop {
                    match chars.get(j) {
                        Some(&q) if q == quote => break,
                        Some(&ch) => s.push(ch),
                        None => return Err("a string is not closed".into()),
                    }
                    j += 1;
                }
                out.push(Token::Str(s));
                i = j + 1;
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                out.push(Token::Ident(chars[start..i].iter().collect()));
            }
            c => return Err(format!("unexpected `{c}`")),
        }
    }
    if out.is_empty() {
        return Err("a when must not be empty".into());
    }
    Ok(out)
}

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn eat(&mut self, t: &Token) -> bool {
        if self.peek() == Some(t) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut left = self.and()?;
        while self.eat(&Token::Or) {
            left = Expr::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut left = self.not()?;
        while self.eat(&Token::And) {
            left = Expr::And(Box::new(left), Box::new(self.not()?));
        }
        Ok(left)
    }

    fn not(&mut self) -> Result<Expr, String> {
        if self.eat(&Token::Not) {
            return Ok(Expr::Not(Box::new(self.not()?)));
        }
        self.cmp()
    }

    fn cmp(&mut self) -> Result<Expr, String> {
        let left = self.atom()?;
        if self.eat(&Token::Eq) {
            return Ok(Expr::Eq(Box::new(left), Box::new(self.atom()?)));
        }
        if self.eat(&Token::Ne) {
            return Ok(Expr::Ne(Box::new(left), Box::new(self.atom()?)));
        }
        Ok(left)
    }

    fn atom(&mut self) -> Result<Expr, String> {
        match self.tokens.get(self.at).cloned() {
            Some(Token::Open) => {
                self.at += 1;
                let inner = self.or()?;
                if !self.eat(&Token::Close) {
                    return Err("a `(` is not closed".into());
                }
                Ok(inner)
            }
            Some(Token::Str(s)) => {
                self.at += 1;
                Ok(Expr::Str(s))
            }
            Some(Token::Ident(name)) => {
                self.at += 1;
                match name.as_str() {
                    "true" => Ok(Expr::Bool(true)),
                    "false" => Ok(Expr::Bool(false)),
                    _ if KEYS.iter().any(|(k, _)| *k == name) => Ok(Expr::Key(name)),
                    _ => Err(format!(
                        "unknown context key `{name}` (known: {})",
                        KEYS.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")
                    )),
                }
            }
            Some(t) => Err(format!("unexpected `{}`", t.text())),
            None => Err("the clause ends too soon".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_locked() -> Context {
        Context {
            card_kind: "editor",
            card_locked: true,
            ..Context::terminal_focused()
        }
    }

    fn holds(src: &str, c: &Context) -> bool {
        When::parse(src).unwrap().eval(c)
    }

    #[test]
    fn a_key_alone_is_its_truth_and_the_derived_ones_follow_the_card() {
        let (e, t) = (editor_locked(), Context::terminal_focused());
        assert!(holds("editorTextFocus", &e) && !holds("editorTextFocus", &t));
        assert!(holds("editorFocus && cardLocked", &e));
        assert!(holds("terminalFocus", &t) && !holds("terminalFocus", &e));
        assert!(!holds("browserFocus", &e));
        // a string key is true unless it is empty or "none"
        assert!(holds("cardKind", &e));
        assert!(!holds("overlay", &e));
    }

    #[test]
    fn operators_group_and_bind_as_written() {
        let e = editor_locked();
        assert!(holds("!suggestWidgetVisible", &e));
        assert!(holds("editorTextFocus && !suggestWidgetVisible", &e));
        assert!(!holds("editorTextFocus && suggestWidgetVisible", &e));
        // && binds tighter than ||
        assert!(holds(
            "suggestWidgetVisible && findWidgetVisible || editorTextFocus",
            &e
        ));
        assert!(!holds(
            "suggestWidgetVisible && (findWidgetVisible || editorTextFocus)",
            &e
        ));
        assert!(holds("!(suggestWidgetVisible || findWidgetVisible)", &e));
    }

    #[test]
    fn strings_compare_with_either_quote_and_literals_are_booleans() {
        let e = editor_locked();
        assert!(holds("cardKind == 'editor'", &e));
        assert!(holds("cardKind == \"editor\"", &e));
        assert!(holds("cardKind != 'browser'", &e));
        assert!(holds("overlay == 'none'", &e));
        assert!(!holds("cardKind == 'browser'", &e));
        assert!(holds("true", &e) && !holds("false", &e));
        assert!(holds("editorTextFocus == true", &e));
    }

    #[test]
    fn mistakes_are_errors_that_say_what_is_wrong() {
        let err = |s: &str| When::parse(s).unwrap_err();
        assert!(err("editorFocuss").contains("unknown context key `editorFocuss`"));
        assert!(
            err("editorFocuss").contains("editorTextFocus"),
            "lists the known ones"
        );
        assert!(err("").contains("empty"));
        assert!(err("a &&").contains("unknown") || err("a &&").contains("too soon"));
        assert!(err("(editorFocus").contains("not closed"));
        assert!(err("editorFocus)").contains("unexpected"));
        assert!(err("'open").contains("not closed"));
        assert!(err("editorFocus & cardLocked").contains("unexpected"));
        assert!(err("editorFocus = true").contains("unexpected"));
    }

    #[test]
    fn a_focused_terminal_context_has_nothing_open() {
        let t = Context::terminal_focused();
        for key in [
            "editorFocus",
            "browserFocus",
            "cardLocked",
            "phantomFocus",
            "multiSelection",
            "suggestWidgetVisible",
            "findWidgetVisible",
            "editorHasSelection",
        ] {
            assert!(!holds(key, &t), "{key}");
        }
        assert!(holds("terminalFocus && overlay == 'none'", &t));
    }

    #[test]
    fn the_source_text_is_kept_trimmed() {
        assert_eq!(
            When::parse("  editorFocus  ").unwrap().source(),
            "editorFocus"
        );
    }
}
