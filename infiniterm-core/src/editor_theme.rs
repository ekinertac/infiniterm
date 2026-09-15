//! The editor's colours, from the terminal theme. Port of editorTheme.ts
//! and its tests.
//!
//! An editor card is a terminal card in every way that shows, and colour
//! shows most: the same background and foreground, syntax painted with the
//! same sixteen ANSI colours a terminal's own highlighters use, so a `.ts`
//! file and `bat foo.ts` in the card beside it are the same theme. The
//! mapping is the conventional one (keywords magenta, strings green,
//! comments the dim grey, numbers yellow, names blue) that bat, delta and
//! vim's default converge on.
//!
//! Where the reference emitted Lezer tag names and CSS (`color-mix`,
//! `var(--card-bg)`), this emits tree-sitter capture names and resolved
//! hex colours; the chrome fallbacks are passed in as `Chrome` so the
//! editor element and the card frame read one palette. Pure: theme in,
//! rules and colours out.
use crate::itermcolors::Theme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxRule {
    /// A tree-sitter highlight capture name.
    pub tag: &'static str,
    pub color: String,
    pub italic: bool,
}

/// Which ANSI slot paints which kind of token, in fallback order.
const ROLES: [(&str, [&str; 2]); 7] = [
    ("keyword", ["magenta", "brightMagenta"]),
    ("string", ["green", "brightGreen"]),
    ("comment", ["brightBlack", "white"]),
    ("number", ["yellow", "brightYellow"]),
    ("name", ["blue", "brightBlue"]),
    ("type", ["cyan", "brightCyan"]),
    ("operator", ["red", "brightRed"]),
];

/// Capture names grouped by the role that paints them.
fn tags(role: &str) -> &'static [&'static str] {
    match role {
        "keyword" => &[
            "keyword",
            "keyword.control",
            "keyword.function",
            "keyword.import",
            "keyword.operator",
        ],
        "string" => &["string", "string.special", "string.regexp", "string.escape"],
        "comment" => &[
            "comment",
            "comment.line",
            "comment.block",
            "comment.documentation",
            "attribute",
        ],
        "number" => &[
            "number",
            "number.integer",
            "number.float",
            "boolean",
            "constant",
            "constant.builtin",
        ],
        "name" => &[
            "function",
            "function.method",
            "function.call",
            "property",
            "variable.definition",
            "label",
        ],
        "type" => &["type", "namespace", "tag", "attribute.name", "text.title"],
        "operator" => &[
            "operator",
            "punctuation",
            "punctuation.bracket",
            "text.reference",
            "text.uri",
        ],
        _ => &[],
    }
}

fn pick<'a>(theme: &'a Theme, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .filter_map(|k| theme.get(*k))
        .map(String::as_str)
        .find(|c| !c.is_empty())
}

/// Syntax rules for a theme, one per capture, only for roles the theme has
/// a colour for. With no theme there are no rules and the editor is plain
/// text in the foreground colour, which is honest.
pub fn syntax_rules(theme: Option<&Theme>) -> Vec<SyntaxRule> {
    let Some(theme) = theme else { return vec![] };
    let mut rules = vec![];
    for (role, keys) in ROLES {
        let Some(color) = pick(theme, &keys) else {
            continue;
        };
        for tag in tags(role) {
            rules.push(SyntaxRule {
                tag,
                color: color.to_string(),
                italic: role == "comment",
            });
        }
    }
    rules
}

/// The chrome palette the editor falls back to without a theme: the same
/// three values the card frame paints with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chrome {
    pub card_bg: String,
    pub text: String,
    pub text_faint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorColors {
    pub background: String,
    pub foreground: String,
    pub cursor: String,
    pub selection: String,
    pub selection_text: String,
    pub gutter: String,
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.strip_prefix('#')?;
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some([v(0)?, v(2)?, v(4)?]),
        3 => {
            let d = |i: usize| u8::from_str_radix(&h[i..i + 1].repeat(2), 16).ok();
            Some([d(0)?, d(1)?, d(2)?])
        }
        _ => None,
    }
}

/// `color-mix(in srgb, a (1-t), b t)` for two hex colours; `a` unchanged
/// when either is not hex.
pub fn mix_hex(a: &str, b: &str, t: f64) -> String {
    match (parse_hex(a), parse_hex(b)) {
        (Some(x), Some(y)) => {
            let c = |i: usize| (x[i] as f64 * (1. - t) + y[i] as f64 * t).round() as u8;
            format!("#{:02x}{:02x}{:02x}", c(0), c(1), c(2))
        }
        _ => a.to_string(),
    }
}

/// Text, cursor and selection are the terminal's; the ground is the
/// terminal's lifted a shade (5%) toward the foreground, so an editor can be
/// told from a terminal across the canvas at any zoom.
pub fn editor_colors(
    theme: Option<&Theme>,
    chrome: &Chrome,
    selection_override: &str,
    selection_text_override: &str,
) -> EditorColors {
    let get = |k: &str| {
        theme
            .and_then(|t| t.get(k))
            .filter(|c| !c.is_empty())
            .map(String::as_str)
    };
    let bg = get("background").unwrap_or(&chrome.card_bg);
    let fg = get("foreground").unwrap_or(&chrome.text);
    let non_blank = |s: &str| Some(s.trim()).filter(|s| !s.is_empty()).map(String::from);
    EditorColors {
        background: mix_hex(bg, fg, 0.05),
        foreground: fg.to_string(),
        cursor: get("cursor").unwrap_or(fg).to_string(),
        // A highlighter pen: the theme's yellow with the background as the
        // text colour. Not the theme's own selection colour, which was
        // chosen against a prompt and is often a shade off the background,
        // invisible on a page of code. The settings override both.
        selection: non_blank(selection_override)
            .or_else(|| {
                get("yellow")
                    .or_else(|| get("brightYellow"))
                    .map(String::from)
            })
            .unwrap_or_else(|| "#ffdc50".to_string()),
        selection_text: non_blank(selection_text_override)
            .or_else(|| get("background").map(String::from))
            .unwrap_or_else(|| "#000000".to_string()),
        gutter: get("brightBlack").unwrap_or(&chrome.text_faint).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        [
            ("background", "#101010"),
            ("foreground", "#e0e0e0"),
            ("magenta", "#c678dd"),
            ("green", "#98c379"),
            ("brightBlack", "#5c6370"),
            ("yellow", "#e5c07b"),
            ("blue", "#61afef"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    fn chrome() -> Chrome {
        Chrome {
            card_bg: "#0e101a".into(),
            text: "#b9c4d2".into(),
            text_faint: "#5a6472".into(),
        }
    }

    fn rule<'a>(rules: &'a [SyntaxRule], tag: &str) -> Option<&'a SyntaxRule> {
        rules.iter().find(|r| r.tag == tag)
    }

    #[test]
    fn paints_keywords_magenta_strings_green_comments_dim_and_italic() {
        let t = theme();
        let rules = syntax_rules(Some(&t));
        assert_eq!(rule(&rules, "keyword").unwrap().color, "#c678dd");
        assert_eq!(rule(&rules, "string").unwrap().color, "#98c379");
        let comment = rule(&rules, "comment").unwrap();
        assert_eq!(comment.color, "#5c6370");
        assert!(comment.italic);
    }

    // A theme without a slot simply leaves those tokens plain.
    #[test]
    fn skips_roles_the_theme_has_no_colour_for() {
        let t = theme();
        let rules = syntax_rules(Some(&t));
        assert!(rule(&rules, "type").is_none()); // no cyan
        assert!(rule(&rules, "operator").is_none()); // no red
    }

    #[test]
    fn falls_back_to_the_bright_slot() {
        let t: Theme = [("brightMagenta".to_string(), "#ff00ff".to_string())]
            .into_iter()
            .collect();
        assert_eq!(
            rule(&syntax_rules(Some(&t)), "keyword").unwrap().color,
            "#ff00ff"
        );
    }

    #[test]
    fn is_empty_without_a_theme() {
        assert!(syntax_rules(None).is_empty());
    }

    #[test]
    fn takes_the_terminal_background_and_foreground() {
        let c = editor_colors(Some(&theme()), &chrome(), "", "");
        // Lifted a shade toward the foreground, so an editor reads as a different surface.
        assert_eq!(c.background, mix_hex("#101010", "#e0e0e0", 0.05));
        assert_eq!(c.background, "#1a1a1a");
        assert_eq!(c.foreground, "#e0e0e0");
        assert_eq!(c.cursor, "#e0e0e0"); // no cursor colour: the foreground
        assert_eq!(c.gutter, "#5c6370");
    }

    #[test]
    fn uses_the_chrome_palette_when_there_is_no_theme() {
        let c = editor_colors(None, &chrome(), "", "");
        assert_eq!(c.background, mix_hex("#0e101a", "#b9c4d2", 0.05));
        assert_eq!(c.foreground, "#b9c4d2");
        assert_eq!(c.gutter, "#5a6472");
    }

    #[test]
    fn selects_with_the_theme_yellow_and_dark_text_unless_the_settings_say_otherwise() {
        let mut t = theme();
        t.insert("selectionBackground".into(), "#222".into());
        let c = editor_colors(Some(&t), &chrome(), "", "");
        assert_eq!(c.selection, "#e5c07b"); // the yellow, not the theme's own selection colour
        assert_eq!(c.selection_text, "#101010");
        let o = editor_colors(Some(&t), &chrome(), "#010203", "#fff");
        assert_eq!(o.selection, "#010203");
        assert_eq!(o.selection_text, "#fff");
    }

    // Native check: the mix matches color-mix(in srgb) per channel and
    // leaves a non-hex colour alone.
    #[test]
    fn mix_hex_blends_per_channel() {
        assert_eq!(mix_hex("#000000", "#ffffff", 0.5), "#808080");
        assert_eq!(mix_hex("#fff", "#000", 1.), "#000000");
        assert_eq!(mix_hex("rgba(1,2,3,0.5)", "#000", 0.5), "rgba(1,2,3,0.5)");
    }
}
