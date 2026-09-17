//! The settings schema: every value, its default, and how a file is read
//! over it. Port of config.ts and its tests.
//!
//! The user's file holds only OVERRIDES (`~/.config/infiniterm/settings.json`)
//! and `settings.default.json` beside it documents the rest (`settings_doc.rs`
//! renders that one from `DEFAULT_CONFIG`). That split is why there is no
//! backfill: a new setting reaches an existing install through a file nobody
//! edits.
//!
//! Every value has a default and a partial file falls back rather than
//! failing; out-of-range numbers CLAMP rather than reject, because a font
//! size of 2000 is a typo and throwing away every valid setting beside it is
//! the worse answer. The defaults live in three places in the reference
//! (config, first-paint tokens, terminal fallbacks) and must agree; in the
//! port this struct is the one source and the UI reads it.
//!
//! Field order is the order the defaults file is rendered in, so it is kept
//! as the reference has it.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Theme name, matching a file in the themes directory (without
    /// extension). `None` means the default theme, not "no theme".
    pub theme: Option<String>,
    /// Where a new card's shell starts when there is no card to inherit
    /// from. Empty means the home directory; the root of the chain, not an
    /// override.
    pub starting_dir: String,
    pub terminal: Terminal,
    pub cards: Cards,
    pub canvas: Canvas,
    pub editor: Editor,
    pub browser: Browser,
    pub ui: Ui,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Terminal {
    /// Program to run in a new card. Empty uses the login shell.
    pub shell: String,
    /// Where a card's shell lives. `pty` is a plain local shell that dies
    /// with the window; `tmux` keeps it running when the app quits and lets
    /// `tmux attach -t infiniterm` reach it from any terminal.
    ///
    /// pty is the DEFAULT. tmux works for shells and for programs that
    /// repaint a whole screen, but a program that redraws INLINE, moving
    /// the cursor up and erasing a line rather than clearing, needs our
    /// grid's scroll position to match exactly what it believes. Two
    /// emulators track one program under tmux, with a replayed history in
    /// between, and when they disagree by a row that kind of redraw lands
    /// wrong and never heals, because it never clears. Claude Code is one.
    pub backend: TerminalBackend,
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    pub font_family: String,
    pub font_size: f64,
    pub font_weight: String,
    pub font_weight_bold: String,
    pub letter_spacing: f64,
    pub line_height: f64,
    pub scrollback: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalBackend {
    Tmux,
    Pty,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cards {
    /// Size of a new card, in 25px grid cells.
    pub width: f64,
    pub height: f64,
    /// Whether a new card opens in the active card's directory.
    pub inherit_directory: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Canvas {
    /// Multiplier on scroll-to-zoom.
    pub zoom_sensitivity: f64,
    /// Whether the canvas glides after a pan drag.
    pub momentum: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Editor {
    /// A wash behind the line the cursor is on.
    pub highlight_line: bool,
    /// Selected text's background and colour. Empty means the theme's yellow
    /// on the theme's background: a highlighter pen, visible on any scheme.
    pub selection_color: String,
    pub selection_text_color: String,
    pub wrap: Wrap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Wrap {
    Prose,
    Always,
    Never,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Browser {
    /// Page zoom a browser card opens at: 1 is actual size.
    pub zoom: f64,
    /// Where the omnibox sends a search; `%s` is the encoded query.
    pub search_engine: String,
    /// Asks Google to complete what is typed in the omnibox. OFF by
    /// default: it is the only thing in this app that talks to anything but
    /// the page you asked for.
    pub suggestions: bool,
    /// EXTRA tab-to-search sites, added to the nine built in. Empty in the
    /// defaults file, so nobody has to scroll past a list they did not
    /// write; `Config::engines` is what the omnibox actually reads.
    pub engines: Vec<crate::omni::engines::Engine>,
}

impl Config {
    /// The tab-to-search registry: the user's entries first, so adding a
    /// keyword that is already built in overrides it rather than being
    /// shadowed by it.
    pub fn engines(&self) -> Vec<crate::omni::engines::Engine> {
        let mut all = self.browser.engines.clone();
        all.extend(crate::omni::engines::default_engines());
        all
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ui {
    /// How much of an inactive card's text is dimmed away, 0 to 1.
    pub inactive_dim: f64,
    /// Chrome font sizes in SCREEN pixels, before the UI-scale multiplier.
    /// Three values because they are read at three distances: a card label
    /// has to survive 10% zoom, a group name sits above a block, the status
    /// bar is always at arm's length.
    pub card_label_size: f64,
    pub group_label_size: f64,
    pub status_bar_size: f64,
    /// Whether the canvas animates at all. Reduced motion still wins.
    pub animations: bool,
    /// The frame counter in the status bar. On by default because the point
    /// of it is to be there when something stutters; it holds a permanent
    /// frame loop while on.
    pub show_fps: bool,
    /// The big centred name over each card below 60% zoom, where cards are
    /// live but too small to read. Off means only the corner label.
    pub mid_zoom_label: bool,
}

/// Page zoom limits for browser cards, the range Safari allows.
pub const BROWSER_ZOOM_MIN: f64 = 0.3;
pub const BROWSER_ZOOM_MAX: f64 = 3.;

pub fn default_config() -> Config {
    Config {
        // Bundled with the app, so a fresh install has colours rather than
        // the fallback palette.
        theme: Some("Catppuccin Mocha".into()),
        starting_dir: String::new(),
        terminal: Terminal {
            shell: String::new(),
            backend: TerminalBackend::Pty,
            cursor_style: CursorStyle::Block,
            cursor_blink: true,
            // What every Mac has. A Nerd Font is one you install, and a
            // default that renders tofu on a fresh machine is worse than one
            // that renders plain ASCII.
            font_family: "ui-monospace, Menlo, monospace".into(),
            font_size: 14.,
            font_weight: "normal".into(),
            font_weight_bold: "bold".into(),
            letter_spacing: 0.,
            line_height: 1.2,
            scrollback: 10_000.,
        },
        cards: Cards {
            width: 69.,
            height: 80.,
            inherit_directory: true,
        },
        canvas: Canvas {
            zoom_sensitivity: 1.,
            momentum: true,
        },
        editor: Editor {
            highlight_line: true,
            selection_color: String::new(),
            selection_text_color: String::new(),
            wrap: Wrap::Prose,
        },
        browser: Browser {
            zoom: 1.,
            search_engine: crate::omni::address::SEARCH_TEMPLATE.into(),
            suggestions: false,
            engines: vec![],
        },
        ui: Ui {
            inactive_dim: 0.45,
            card_label_size: 15.,
            group_label_size: 15.,
            status_bar_size: 11.,
            animations: true,
            show_fps: true,
            mid_zoom_label: true,
        },
    }
}

fn num(value: Option<&Value>, fallback: f64, min: f64, max: f64) -> f64 {
    match value.and_then(Value::as_f64) {
        Some(n) if n.is_finite() => n.clamp(min, max),
        _ => fallback,
    }
}

fn bool_(value: Option<&Value>, fallback: bool) -> bool {
    value.and_then(Value::as_bool).unwrap_or(fallback)
}

/// A non-blank string, or the default.
fn str_(value: Option<&Value>, fallback: &str) -> String {
    match value.and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => fallback.to_string(),
    }
}

/// A string trimmed, or the default; blank is allowed (it means "unset").
fn trimmed(value: Option<&Value>, fallback: &str) -> String {
    value
        .and_then(Value::as_str)
        .map_or_else(|| fallback.to_string(), |s| s.trim().to_string())
}

/// One of a fixed set, or the default. A typo falls back rather than breaking.
fn one<T: Copy>(value: Option<&Value>, fallback: T, allowed: &[(&str, T)]) -> T {
    value
        .and_then(Value::as_str)
        .and_then(|s| allowed.iter().find(|(name, _)| *name == s))
        .map_or(fallback, |(_, v)| *v)
}

/// The group `key` of `raw` as an object, or an empty one.
fn group(raw: &serde_json::Map<String, Value>, key: &str) -> serde_json::Map<String, Value> {
    raw.get(key)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// Merges a parsed file over the defaults, clamping anything out of range.
pub fn merge_config(raw: &Value) -> Config {
    let d = default_config();
    let Some(r) = raw.as_object() else { return d };
    let t = group(r, "terminal");
    let u = group(r, "ui");
    let c = group(r, "cards");
    let v = group(r, "canvas");
    let e = group(r, "editor");
    let b = group(r, "browser");

    Config {
        theme: match r.get("theme").and_then(Value::as_str) {
            Some(s) if !s.trim().is_empty() => Some(s.to_string()),
            _ => d.theme.clone(),
        },
        starting_dir: trimmed(r.get("startingDir"), &d.starting_dir),
        terminal: Terminal {
            shell: trimmed(t.get("shell"), &d.terminal.shell),
            backend: one(
                t.get("backend"),
                d.terminal.backend,
                &[
                    ("tmux", TerminalBackend::Tmux),
                    ("pty", TerminalBackend::Pty),
                ],
            ),
            cursor_style: one(
                t.get("cursorStyle"),
                d.terminal.cursor_style,
                &[
                    ("block", CursorStyle::Block),
                    ("bar", CursorStyle::Bar),
                    ("underline", CursorStyle::Underline),
                ],
            ),
            cursor_blink: bool_(t.get("cursorBlink"), d.terminal.cursor_blink),
            font_family: str_(t.get("fontFamily"), &d.terminal.font_family),
            font_size: num(t.get("fontSize"), d.terminal.font_size, 6., 96.),
            font_weight: str_(t.get("fontWeight"), &d.terminal.font_weight),
            font_weight_bold: str_(t.get("fontWeightBold"), &d.terminal.font_weight_bold),
            letter_spacing: num(t.get("letterSpacing"), d.terminal.letter_spacing, -10., 10.),
            line_height: num(t.get("lineHeight"), d.terminal.line_height, 0.8, 3.),
            scrollback: num(t.get("scrollback"), d.terminal.scrollback, 0., 1_000_000.),
        },
        cards: Cards {
            // Floors that keep a card usable: below about 40 columns a
            // terminal stops being one; the ceiling is what placement can
            // still fit.
            width: num(c.get("width"), d.cards.width, 12., 400.).round(),
            height: num(c.get("height"), d.cards.height, 8., 400.).round(),
            inherit_directory: bool_(c.get("inheritDirectory"), d.cards.inherit_directory),
        },
        canvas: Canvas {
            zoom_sensitivity: num(v.get("zoomSensitivity"), d.canvas.zoom_sensitivity, 0.1, 5.),
            momentum: bool_(v.get("momentum"), d.canvas.momentum),
        },
        editor: Editor {
            highlight_line: bool_(e.get("highlightLine"), d.editor.highlight_line),
            selection_color: str_(e.get("selectionColor"), &d.editor.selection_color),
            selection_text_color: str_(e.get("selectionTextColor"), &d.editor.selection_text_color),
            wrap: one(
                e.get("wrap"),
                d.editor.wrap,
                &[
                    ("prose", Wrap::Prose),
                    ("always", Wrap::Always),
                    ("never", Wrap::Never),
                ],
            ),
        },
        browser: Browser {
            zoom: num(
                b.get("zoom"),
                d.browser.zoom,
                BROWSER_ZOOM_MIN,
                BROWSER_ZOOM_MAX,
            ),
            search_engine: match b.get("searchEngine").and_then(Value::as_str) {
                // A template with no %s cannot carry a query, and a URL that
                // silently drops what was typed is worse than the default.
                Some(s) if s.contains("%s") => s.to_string(),
                _ => d.browser.search_engine.clone(),
            },
            suggestions: bool_(b.get("suggestions"), d.browser.suggestions),
            engines: b
                .get("engines")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|v| {
                            Some(crate::omni::engines::Engine {
                                keyword: v.get("keyword")?.as_str()?.to_string(),
                                name: v.get("name")?.as_str()?.to_string(),
                                search_url: v
                                    .get("searchUrl")
                                    .and_then(Value::as_str)
                                    .filter(|u| u.contains("%s"))?
                                    .to_string(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        },
        ui: Ui {
            inactive_dim: num(u.get("inactiveDim"), d.ui.inactive_dim, 0., 1.),
            card_label_size: num(u.get("cardLabelSize"), d.ui.card_label_size, 6., 64.),
            group_label_size: num(u.get("groupLabelSize"), d.ui.group_label_size, 6., 64.),
            status_bar_size: num(u.get("statusBarSize"), d.ui.status_bar_size, 6., 32.),
            animations: bool_(u.get("animations"), d.ui.animations),
            show_fps: bool_(u.get("showFps"), d.ui.show_fps),
            mid_zoom_label: bool_(u.get("midZoomLabel"), d.ui.mid_zoom_label),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_browser_group_carries_the_omnibox_settings() {
        let d = default_config();
        assert_eq!(
            d.browser.search_engine,
            crate::omni::address::SEARCH_TEMPLATE
        );
        assert!(!d.browser.suggestions, "the network call is opt in");
        // The defaults file shows an empty list; the built-ins are in code.
        assert!(d.browser.engines.is_empty());
        assert_eq!(
            d.engines().len(),
            crate::omni::engines::DEFAULT_ENGINES.len()
        );

        // A template without %s cannot carry a query.
        let c = merge_config(&serde_json::json!({"browser": {"searchEngine": "https://x/?q="}}));
        assert_eq!(
            c.browser.search_engine,
            crate::omni::address::SEARCH_TEMPLATE
        );

        let c = merge_config(&serde_json::json!({"browser": {"suggestions": true}}));
        assert!(c.browser.suggestions);

        // A malformed engine is dropped and the rest survive; a user entry
        // comes before the built-ins, so it can override one.
        let c = merge_config(&serde_json::json!({"browser": {"engines": [
            {"keyword": "github.com", "name": "Mine", "searchUrl": "https://x.example/s?q=%s"},
            {"keyword": "broken"}
        ]}}));
        assert_eq!(c.browser.engines.len(), 1);
        assert_eq!(
            crate::omni::engines::resolve("github.com", &c.engines())
                .unwrap()
                .name,
            "Mine"
        );
    }
    use serde_json::json;

    fn m(v: Value) -> Config {
        merge_config(&v)
    }

    #[test]
    fn an_empty_object_gives_the_defaults() {
        assert_eq!(m(json!({})), default_config());
    }

    #[test]
    fn a_partial_file_only_overrides_what_it_sets() {
        let c = m(json!({"terminal": {"fontSize": 14}}));
        let d = default_config();
        assert_eq!(c.terminal.font_size, 14.);
        assert_eq!(c.terminal.font_family, d.terminal.font_family);
        assert_eq!(c.ui.inactive_dim, d.ui.inactive_dim);
    }

    // A font size of 2000 is a typo; throwing away every other valid setting
    // beside it would be a worse answer than clamping.
    #[test]
    fn out_of_range_numbers_clamp_instead_of_rejecting_the_file() {
        assert_eq!(
            m(json!({"terminal": {"fontSize": 2000}}))
                .terminal
                .font_size,
            96.
        );
        assert_eq!(
            m(json!({"terminal": {"fontSize": 0}})).terminal.font_size,
            6.
        );
        assert_eq!(m(json!({"ui": {"inactiveDim": 5}})).ui.inactive_dim, 1.);
        assert!(m(json!({})).ui.mid_zoom_label);
        assert!(!m(json!({"ui": {"midZoomLabel": false}})).ui.mid_zoom_label);
    }

    #[test]
    fn wrong_types_fall_back_rather_than_propagating() {
        let c = m(json!({"terminal": {"fontSize": "huge", "lineHeight": null}, "ui": 7}));
        let d = default_config();
        assert_eq!(c.terminal.font_size, d.terminal.font_size);
        assert_eq!(c.terminal.line_height, d.terminal.line_height);
        assert_eq!(c.ui.inactive_dim, d.ui.inactive_dim);
    }

    // JSON cannot carry NaN or Infinity, so a Value never holds one; the
    // clamp's finite check covers the case if a caller builds a Value by hand.
    #[test]
    fn nan_and_infinity_are_not_numbers_worth_honouring() {
        let d = default_config();
        assert_eq!(
            num(Some(&Value::Null), d.terminal.font_size, 6., 96.),
            d.terminal.font_size
        );
        assert_eq!(
            num(
                serde_json::Number::from_f64(f64::NAN)
                    .map(Value::Number)
                    .as_ref(),
                14.,
                6.,
                96.
            ),
            14.
        );
        assert_eq!(
            num(
                serde_json::Number::from_f64(f64::INFINITY)
                    .map(Value::Number)
                    .as_ref(),
                14.,
                6.,
                96.
            ),
            14.
        );
    }

    #[test]
    fn an_empty_theme_name_means_the_default_theme_not_a_theme_called_empty() {
        let d = default_config();
        assert_eq!(m(json!({"theme": "   "})).theme, d.theme);
        assert_eq!(m(json!({"theme": null})).theme, d.theme);
        assert_eq!(
            m(json!({"theme": "Dracula"})).theme.as_deref(),
            Some("Dracula")
        );
    }

    #[test]
    fn a_non_object_file_gives_the_defaults() {
        assert_eq!(m(json!([1, 2])), default_config());
        assert_eq!(m(Value::Null), default_config());
        assert_eq!(m(json!("nope")), default_config());
    }

    // Three sizes, not one: read at three distances.
    #[test]
    fn chrome_font_sizes_default_and_clamp_independently() {
        let u = m(json!({})).ui;
        assert_eq!(
            (u.card_label_size, u.group_label_size, u.status_bar_size),
            (15., 15., 11.)
        );
        let c = m(json!({"ui": {"cardLabelSize": 22, "groupLabelSize": 999, "statusBarSize": 0}}));
        assert_eq!(c.ui.card_label_size, 22.);
        assert_eq!(c.ui.group_label_size, 64.);
        assert_eq!(c.ui.status_bar_size, 6.);
    }

    #[test]
    fn a_non_numeric_chrome_size_falls_back_rather_than_reaching_the_renderer() {
        assert_eq!(
            m(json!({"ui": {"cardLabelSize": "big"}}))
                .ui
                .card_label_size,
            15.
        );
    }

    // The root of the inheritance chain, not an override.
    #[test]
    fn starting_dir_defaults_to_empty_meaning_the_home_directory() {
        assert_eq!(m(json!({})).starting_dir, "");
        assert_eq!(
            m(json!({"startingDir": "/Users/me/Code"})).starting_dir,
            "/Users/me/Code"
        );
        assert_eq!(m(json!({"startingDir": "  /tmp  "})).starting_dir, "/tmp");
        assert_eq!(m(json!({"startingDir": 42})).starting_dir, "");
    }

    #[test]
    fn the_new_terminal_settings_default_and_validate() {
        let t = m(json!({})).terminal;
        assert_eq!(t.shell, "");
        assert_eq!(t.cursor_style, CursorStyle::Block);
        assert!(t.cursor_blink);
        assert_eq!(
            m(json!({"terminal": {"cursorStyle": "bar"}}))
                .terminal
                .cursor_style,
            CursorStyle::Bar
        );
    }

    // A typo falls back rather than handing the terminal something it does not understand.
    #[test]
    fn an_unknown_cursor_style_falls_back_to_the_default() {
        assert_eq!(
            m(json!({"terminal": {"cursorStyle": "squiggle"}}))
                .terminal
                .cursor_style,
            CursorStyle::Block
        );
    }

    #[test]
    fn card_size_is_whole_cells_and_clamped_to_something_usable() {
        let c = m(json!({})).cards;
        assert_eq!((c.width, c.height, c.inherit_directory), (69., 80., true));
        assert_eq!(m(json!({"cards": {"width": 40.7}})).cards.width, 41.);
        assert_eq!(m(json!({"cards": {"width": 1}})).cards.width, 12.);
        assert_eq!(m(json!({"cards": {"height": 9999}})).cards.height, 400.);
    }

    #[test]
    fn canvas_settings_default_and_clamp() {
        assert_eq!(
            m(json!({})).canvas,
            Canvas {
                zoom_sensitivity: 1.,
                momentum: true
            }
        );
        assert_eq!(
            m(json!({"canvas": {"zoomSensitivity": 99}}))
                .canvas
                .zoom_sensitivity,
            5.
        );
        assert!(!m(json!({"canvas": {"momentum": false}})).canvas.momentum);
    }

    #[test]
    fn a_boolean_setting_ignores_a_non_boolean() {
        assert!(m(json!({"ui": {"animations": "yes"}})).ui.animations);
        assert!(!m(json!({"ui": {"animations": false}})).ui.animations);
    }
}
