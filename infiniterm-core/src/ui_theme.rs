//! A ui theme: `ui-themes/<name>.json`, how infiniterm LOOKS (#511).
//!
//! Where it fits: `Model::apply_ui_theme_text` runs `parse` on the file that
//! `ui.theme` in settings.json names (and on every edit of it), keeps the
//! result in `Model::ui_theme`, and overlays its look keys on `config.ui`
//! (`Ui::take_look_from`), so every reader of `config.ui.cardRadius` and the
//! like is unchanged. The ui resolves the bar tokens against the live
//! `Chrome` colours each frame, so "canvas" follows the canvas colour.
//! Related: `config_files.rs` (the folder and its watcher), `look_migration.rs`
//! (moves the look keys out of an old settings.json), `settings_doc.rs`
//! (the reference text, from the docs the keys already had), `jsonc.rs`.
//!
//! One file per look, so a look is something you copy, switch and share.
//! Chrome only, never the terminal palette: that is `theme` in settings.json
//! and the 522 iTerm2 schemes. Keys are FLAT and dotted like settings.json.
//! The moved keys keep their `ui.` names (`ui.cardRadius`), so moving one is
//! moving a line and the docs they already had stay true. An unknown key or a
//! bad value rejects the whole file and the look in force stays, the rule
//! settings.json follows since #246.
use crate::jsonc::parse_jsonc;
use serde_json::{Map, Value};

/// The look every install has: the file `ui-themes/default.json` when it
/// exists, else the built-in values with nothing overridden.
pub const DEFAULT_NAME: &str = "default";

/// The folder inside the config folder.
pub const DIR_NAME: &str = "ui-themes";

/// A colour the file names: a literal, or a token the ui resolves each frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColorSpec {
    /// `#rrggbb` or `#rgb`, kept as written; the ui turns it into a colour.
    Hex(String),
    /// The canvas colour behind the cards: a flat bar that blends into it.
    Canvas,
    /// The card ground (the terminal theme's background).
    Card,
    /// What the bars wear without a theme.
    Bar,
    /// Nothing drawn: the window's own background shows through.
    Transparent,
}

/// A bar's edge: a colour, or none at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BorderSpec {
    None,
    Color(ColorSpec),
}

/// One bar's overrides; `None` means "as before ui themes existed".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BarTheme {
    pub background: Option<ColorSpec>,
    pub border: Option<BorderSpec>,
}

/// The workspace tabs' look (#544). Every field is `None` until the file sets
/// it, and `None` is the tab as it was drawn before.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabTheme {
    /// An idle tab's fill (drawn as nothing today).
    pub background: Option<ColorSpec>,
    /// The chosen tab's fill.
    pub active_background: Option<ColorSpec>,
    /// The chosen tab's fill as a gradient (two stops, may carry alpha): it
    /// replaces `active_background`.
    pub gradient: Option<[String; 2]>,
    /// Multiplies the alpha of every tab fill, 0.1 to 1.
    pub opacity: Option<f64>,
    /// An idle tab's outline.
    pub border: Option<BorderSpec>,
    /// The chosen tab's outline.
    pub active_border: Option<BorderSpec>,
    /// The line between two neighbouring idle tabs.
    pub separator: Option<BorderSpec>,
    /// Corner radius in pixels, 0 to 40.
    pub radius: Option<f64>,
    /// Show the focused card's program as an icon on the tab. Off unless set.
    pub icons: Option<bool>,
    /// What the icon is made of (`tab_icon::IconStyle`); unset is a glyph.
    pub icon_style: Option<crate::tab_icon::IconStyle>,
    /// Program name to a glyph or a logo file name (`tab_icon::choose`).
    pub icon_map: std::collections::BTreeMap<String, String>,
    /// The icon's size in pixels, scaled with the interface size; unset keeps
    /// the built-in size (a little over the label's).
    pub icon_size: Option<f64>,
}

/// The card header's keys (#544); see `card_header.rs`. Unset is no header.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CardTheme {
    /// A header row inside terminal, transcript and page cards.
    pub header: Option<bool>,
    /// Which buttons it carries, in order; none unless listed.
    pub header_buttons: Vec<crate::card_header::HeaderButton>,
    /// Show the buttons only on the focused card. Off unless set.
    pub header_buttons_active_only: Option<bool>,
    /// The program's icon at the header's left, from the tab icon keys
    /// (`tab.iconStyle`, `tab.iconMap`).
    pub header_icons: Option<bool>,
    /// The header icon's size in pixels at 100% zoom (it scales with the
    /// card); unset keeps the built-in size.
    pub header_icon_size: Option<f64>,
}

impl CardTheme {
    pub fn header_on(&self) -> bool {
        self.header == Some(true)
    }

    /// The buttons a card wears: the listed ones, or none on a card that is
    /// not the focused one while `card.headerButtonsActiveOnly` is on.
    pub fn buttons_for(&self, focused: bool) -> &[crate::card_header::HeaderButton] {
        if self.header_buttons_active_only == Some(true) && !focused {
            &[]
        } else {
            &self.header_buttons
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiTheme {
    /// The look keys the file set, as written (`"ui.cardRadius": 8`). Range
    /// clamping and defaults are `config::merge_config`'s, the same code
    /// settings.json used for these keys, so a value means what it always did.
    pub look: Map<String, Value>,
    pub titlebar: BarTheme,
    pub statusbar: BarTheme,
    pub tab: TabTheme,
    pub card: CardTheme,
}

/// What kind of JSON value a look key holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Number,
    Bool,
    Text,
    TextOrList,
    /// A list of exactly two gradient stops, or an empty list (off).
    ColorPair,
    /// One of a fixed set of words (`ui.icon`); anything else rejects the file.
    Choice(&'static [&'static str]),
}

/// A gradient stop: `#rgb`, `#rrggbb` or `#rrggbbaa`. Unlike a bar colour it
/// may carry its own alpha, which the card opacity then multiplies (#544).
pub fn is_gradient_stop(s: &str) -> bool {
    let Some(h) = s.strip_prefix('#') else {
        return false;
    };
    matches!(h.len(), 3 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit())
}

/// Every look key the file accepts, with its value kind. These are the keys
/// that left settings.json in #511. The `settings_doc` rule applies here too:
/// a key someone can write is a key in this list, in `Ui::take_look_from`
/// and in the docs; a test holds each of the three.
pub const LOOK_KEYS: [(&str, Kind); 21] = [
    ("ui.icon", Kind::Choice(&crate::app_icon::NAMES)),
    ("ui.windowOpacity", Kind::Number),
    ("ui.windowBlur", Kind::Bool),
    ("ui.windowColor", Kind::Text),
    ("ui.backgroundImage", Kind::TextOrList),
    ("ui.backgroundImageInterval", Kind::Number),
    ("ui.backgroundImageFade", Kind::Number),
    ("ui.backgroundImageFit", Kind::Text),
    ("ui.cardRadius", Kind::Number),
    ("ui.cardOpacity", Kind::Number),
    ("ui.cardGradient", Kind::ColorPair),
    ("ui.cardGradientAngle", Kind::Number),
    ("ui.focusRingWidth", Kind::Number),
    ("ui.showGrid", Kind::Bool),
    ("ui.inactiveDim", Kind::Number),
    ("ui.unfocusedDim", Kind::Number),
    ("ui.fontFamily", Kind::Text),
    ("ui.fontSize", Kind::Number),
    ("ui.fontWeight", Kind::Text),
    ("ui.fontWeightBold", Kind::Text),
    ("ui.hideLabelWhenFitted", Kind::Bool),
];

/// The bar keys (#425) and the tab keys (#544), with their docs. They exist
/// only in a ui theme.
pub const BAR_KEYS: [(&str, &[&str], &str); 21] = [
    (
        "titlebar.background",
        &[
            "The title bar's ground: #rgb, #rrggbb, \"canvas\" (the canvas behind the cards),",
            "\"card\" (the terminal theme's background), \"bar\" (the default) or \"transparent\".",
        ],
        "\"canvas\"",
    ),
    (
        "titlebar.border",
        &["The title bar's edge: a colour as above, or \"none\"."],
        "\"none\"",
    ),
    (
        "statusbar.background",
        &["The status bar's ground. Same values as titlebar.background."],
        "\"canvas\"",
    ),
    (
        "statusbar.border",
        &["The status bar's edge: a colour, or \"none\"."],
        "\"none\"",
    ),
    (
        "tab.background",
        &[
            "A workspace tab's fill while it is not the chosen one: #rgb, #rrggbb or #rrggbbaa,",
            "or \"canvas\", \"card\", \"bar\", \"transparent\". Unset, idle tabs have no fill.",
        ],
        "\"#ffffff14\"",
    ),
    (
        "tab.activeBackground",
        &["The chosen workspace tab's fill. Same values as tab.background."],
        "\"#ffffff29\"",
    ),
    (
        "tab.gradient",
        &[
            "The chosen workspace tab's fill as a gradient: two colours, top to bottom, either",
            "may carry an alpha. It replaces tab.activeBackground. [] keeps the flat fill.",
        ],
        "[\"#5a4a42\", \"#2b2320\"]",
    ),
    (
        "tab.opacity",
        &[
            "Multiplies the alpha of every tab fill, 0.1 to 1, so tabs stay translucent over",
            "ui.windowBlur. Default 1.",
        ],
        "1",
    ),
    (
        "tab.border",
        &["A workspace tab's one pixel outline while it is not the chosen one: a colour, or \"none\"."],
        "\"none\"",
    ),
    (
        "tab.activeBorder",
        &["The chosen workspace tab's outline: a colour, or \"none\"."],
        "\"none\"",
    ),
    (
        "tab.separator",
        &[
            "A thin line between two neighbouring idle workspace tabs: a colour, or \"none\".",
            "No line touches the chosen tab.",
        ],
        "\"none\"",
    ),
    (
        "tab.radius",
        &["Round the corners of workspace tabs, in pixels, 0 to 40. Unset keeps the small default."],
        "6",
    ),
    (
        "tab.icons",
        &[
            "Show an icon on each workspace tab for the program running in that workspace's",
            "focused card (the first card, when none is focused). Off by default.",
        ],
        "false",
    ),
    (
        "tab.iconStyle",
        &[
            "\"glyph\" draws a Nerd Font glyph. \"image\" draws a logo from ui-themes/<name>/icons/",
            "(<program>.svg or .png, default.svg or .png for the rest) and a glyph for a",
            "program with none. <name> is this theme's file name without .json.",
        ],
        "\"glyph\"",
    ),
    (
        "tab.iconMap",
        &[
            "Program name to icon: a glyph (any text) or a file name in the theme's folder",
            "ending in .svg or .png. The key \"default\" covers programs not listed. Built-in",
            "glyphs cover common shells, runtimes and coding agents.",
        ],
        "{\"bun\": \"bun.svg\", \"default\": \"\\uf120\"}",
    ),
    (
        "tab.iconSize",
        &[
            "The workspace tab icon's size in pixels, 6 to 64: a glyph's font size or a logo's",
            "side. It grows and shrinks with the interface size (Cmd+Shift+= and Cmd+Shift+-).",
            "Unset keeps the built-in size, a little larger than the tab's label.",
        ],
        "16",
    ),
    (
        "card.header",
        &[
            "A header row inside terminal, transcript and page cards, above the body: the",
            "program's icon (card.headerIcons), the card's name and card.headerButtons. The body",
            "gets the rest, so a terminal has fewer rows. Editor, diff and browser cards keep",
            "their own tab strip. Off, the name is a chip in the card's corner as before.",
        ],
        "false",
    ),
    (
        "card.headerButtons",
        &[
            "The header's buttons, left to right: \"split-right\", \"split-down\", \"maximise\",",
            "\"close\". Each runs the command of that name (card.split.right, card.split.down,",
            "card.maximize.toggle, card.close). None unless listed.",
        ],
        "[\"split-right\", \"split-down\", \"maximise\", \"close\"]",
    ),
    (
        "card.headerButtonsActiveOnly",
        &[
            "Show the header's buttons only on the focused card. The other cards' headers show",
            "just the icon and the name, and nothing is clickable where the buttons would be.",
            "Default false.",
        ],
        "false",
    ),
    (
        "card.headerIcons",
        &[
            "Show the program's icon at the header's left. It uses tab.iconStyle and",
            "tab.iconMap, so the card and its workspace tab agree. Default false.",
        ],
        "false",
    ),
    (
        "card.headerIconSize",
        &[
            "The header icon's size in pixels at 100% zoom, 6 to 64: a glyph's font size or a",
            "logo's side. It scales with the card, like the header. Unset keeps the built-in",
            "size, about half the header's height.",
        ],
        "16",
    ),
];

/// Whether a dotted key belongs to a ui theme and not to settings.json.
pub fn is_look_key(key: &str) -> bool {
    LOOK_KEYS.iter().any(|(k, _)| *k == key)
}

/// A settings value with the look keys taken out, and their dotted names.
/// Both spellings go: `"ui.cardRadius": 8` and `"ui": {"cardRadius": 8}`.
/// settings.json no longer accepts these keys, but an old file that still
/// holds them (a host older than this build, sent to a viewer; a key typed
/// by hand) must not fail validation, so they are dropped, not refused.
pub fn strip_look(raw: &Value) -> (Value, Vec<String>) {
    let Some(map) = raw.as_object() else {
        return (raw.clone(), vec![]);
    };
    let mut out = Map::new();
    let mut dropped = vec![];
    for (k, v) in map {
        if is_look_key(k) {
            dropped.push(k.clone());
        } else if k == "ui" {
            match v.as_object() {
                Some(inner) => {
                    let mut kept = Map::new();
                    for (ik, iv) in inner {
                        let full = format!("ui.{ik}");
                        if is_look_key(&full) {
                            dropped.push(full);
                        } else {
                            kept.insert(ik.clone(), iv.clone());
                        }
                    }
                    out.insert(k.clone(), Value::Object(kept));
                }
                None => {
                    out.insert(k.clone(), v.clone());
                }
            }
        } else {
            out.insert(k.clone(), v.clone());
        }
    }
    (Value::Object(out), dropped)
}

/// Whether `name` can be a ui theme's file name: one plain path component, so
/// a `ui.theme` of `../../x` cannot reach outside the folder.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
        && name.trim() == name
}

/// The picker's rows from the file stems in the folder: `default` always
/// (it works with no file), first; the rest A to Z without regard to case.
pub fn names_from_stems(stems: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut rest: Vec<String> = stems
        .into_iter()
        .filter(|s| s != DEFAULT_NAME && valid_name(s))
        .collect();
    rest.sort_by_key(|s| s.to_lowercase());
    rest.dedup();
    std::iter::once(DEFAULT_NAME.to_string())
        .chain(rest)
        .collect()
}

fn is_hex(s: &str) -> bool {
    let Some(h) = s.strip_prefix('#') else {
        return false;
    };
    matches!(h.len(), 3 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit())
}

fn color(key: &str, v: &Value) -> Result<ColorSpec, String> {
    let s = v
        .as_str()
        .ok_or_else(|| format!("\"{key}\" must be a string"))?;
    match s {
        "canvas" => Ok(ColorSpec::Canvas),
        "card" => Ok(ColorSpec::Card),
        "bar" => Ok(ColorSpec::Bar),
        "transparent" => Ok(ColorSpec::Transparent),
        _ if is_hex(s) => Ok(ColorSpec::Hex(s.to_string())),
        _ => Err(format!(
            "\"{key}\": \"{s}\" is not #rgb, #rrggbb, #rrggbbaa, \"canvas\", \"card\", \"bar\" or \"transparent\""
        )),
    }
}

fn border(key: &str, v: &Value) -> Result<BorderSpec, String> {
    if v.as_str() == Some("none") {
        return Ok(BorderSpec::None);
    }
    color(key, v)
        .map(BorderSpec::Color)
        .map_err(|e| format!("{e} (or \"none\")"))
}

/// `tab.gradient`: two stops, or `[]` for none.
fn gradient(key: &str, v: &Value) -> Result<Option<[String; 2]>, String> {
    let stops: Option<Vec<&str>> = v
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect());
    match (v.as_array().map(Vec::len), stops.as_deref()) {
        (Some(0), _) => Ok(None),
        (Some(2), Some([a, b])) if is_gradient_stop(a) && is_gradient_stop(b) => {
            Ok(Some([a.to_string(), b.to_string()]))
        }
        _ => Err(format!(
            "\"{key}\" must be [] or two colours like [\"#5a4a42\", \"#2b2320\"] (#rgb, #rrggbb or #rrggbbaa)"
        )),
    }
}

/// `tab.iconMap`: program name to a glyph or a logo file name. A value that
/// names a file must be a plain file name (`tab_icon::safe_file`).
fn icon_map(key: &str, v: &Value) -> Result<std::collections::BTreeMap<String, String>, String> {
    let obj = v
        .as_object()
        .ok_or_else(|| format!("\"{key}\" must be an object of \"program\": \"icon\""))?;
    let mut map = std::collections::BTreeMap::new();
    for (program, icon) in obj {
        let icon = icon
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("\"{key}\": \"{program}\" must be a glyph or a file name"))?;
        let looks_like_file = icon.rsplit_once('.').is_some_and(|(_, e)| {
            crate::tab_icon::IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str())
        });
        if looks_like_file && !crate::tab_icon::safe_file(icon) {
            return Err(format!(
                "\"{key}\": \"{icon}\" must be a plain file name in the theme's folder"
            ));
        }
        map.insert(crate::tab_icon::program_key(program), icon.to_string());
    }
    Ok(map)
}

/// `card.headerButtons`: a list of button names (`card_header::HeaderButton`).
fn header_buttons(key: &str, v: &Value) -> Result<Vec<crate::card_header::HeaderButton>, String> {
    let names: Option<Vec<String>> = v.as_array().and_then(|a| {
        a.iter()
            .map(|n| n.as_str().map(String::from))
            .collect::<Option<Vec<_>>>()
    });
    names
        .and_then(|n| crate::card_header::parse_buttons(&n))
        .ok_or_else(|| {
            format!(
                "\"{key}\" must be a list of {}",
                crate::card_header::HeaderButton::NAMES
                    .iter()
                    .map(|n| format!("\"{n}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

/// A number clamped to `min..=max`, like `merge_config` does for the look keys.
fn number(key: &str, v: &Value, min: f64, max: f64) -> Result<f64, String> {
    v.as_f64()
        .map(|n| n.clamp(min, max))
        .ok_or_else(|| format!("\"{key}\" must be a number"))
}

fn check_kind(key: &str, kind: Kind, v: &Value) -> Result<(), String> {
    if let Kind::Choice(words) = kind {
        if v.as_str().is_some_and(|s| words.contains(&s)) {
            return Ok(());
        }
        let list: Vec<String> = words.iter().map(|w| format!("\"{w}\"")).collect();
        return Err(format!("\"{key}\" must be {}", list.join(" or ")));
    }
    let (ok, want) = match kind {
        Kind::Number => (v.is_number(), "a number"),
        Kind::Bool => (v.is_boolean(), "true or false"),
        Kind::Text => (v.is_string(), "text"),
        Kind::TextOrList => (
            v.is_string() || v.as_array().is_some_and(|a| a.iter().all(Value::is_string)),
            "text or a list of text",
        ),
        Kind::ColorPair => (
            v.as_array().is_some_and(|a| {
                a.is_empty()
                    || (a.len() == 2 && a.iter().all(|s| s.as_str().is_some_and(is_gradient_stop)))
            }),
            "[] or two colours like [\"#3a2c27\", \"#1f1a18\"] (#rgb, #rrggbb or #rrggbbaa)",
        ),
        Kind::Choice(_) => unreachable!("handled above"),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("\"{key}\" must be {want}"))
    }
}

/// Parses a ui theme's text. An empty file is the empty theme (the built-in
/// look); anything that is not a JSON object, an unknown key or a bad value
/// is an error.
pub fn parse(text: &str) -> Result<UiTheme, String> {
    if text.trim().is_empty() {
        return Ok(UiTheme::default());
    }
    let raw = parse_jsonc(text).map_err(|e| e.to_string())?;
    let obj = raw
        .as_object()
        .ok_or("a ui theme must be a JSON object of \"key\": value")?;
    let mut theme = UiTheme::default();
    for (key, v) in obj {
        match key.as_str() {
            "titlebar.background" => theme.titlebar.background = Some(color(key, v)?),
            "titlebar.border" => theme.titlebar.border = Some(border(key, v)?),
            "statusbar.background" => theme.statusbar.background = Some(color(key, v)?),
            "statusbar.border" => theme.statusbar.border = Some(border(key, v)?),
            "tab.background" => theme.tab.background = Some(color(key, v)?),
            "tab.activeBackground" => theme.tab.active_background = Some(color(key, v)?),
            "tab.gradient" => theme.tab.gradient = gradient(key, v)?,
            "tab.opacity" => theme.tab.opacity = Some(number(key, v, 0.1, 1.)?),
            "tab.border" => theme.tab.border = Some(border(key, v)?),
            "tab.activeBorder" => theme.tab.active_border = Some(border(key, v)?),
            "tab.separator" => theme.tab.separator = Some(border(key, v)?),
            "tab.radius" => theme.tab.radius = Some(number(key, v, 0., 40.)?),
            "tab.icons" => {
                theme.tab.icons = Some(v.as_bool().ok_or("\"tab.icons\" must be true or false")?)
            }
            "tab.iconStyle" => {
                theme.tab.icon_style = Some(
                    v.as_str()
                        .and_then(crate::tab_icon::IconStyle::parse)
                        .ok_or("\"tab.iconStyle\" must be \"glyph\" or \"image\"")?,
                )
            }
            "tab.iconMap" => theme.tab.icon_map = icon_map(key, v)?,
            "tab.iconSize" => {
                theme.tab.icon_size = Some(number(
                    key,
                    v,
                    crate::tab_icon::ICON_SIZE_MIN,
                    crate::tab_icon::ICON_SIZE_MAX,
                )?)
            }
            "card.headerIconSize" => {
                theme.card.header_icon_size = Some(number(
                    key,
                    v,
                    crate::tab_icon::ICON_SIZE_MIN,
                    crate::tab_icon::ICON_SIZE_MAX,
                )?)
            }
            "card.header" => {
                theme.card.header =
                    Some(v.as_bool().ok_or("\"card.header\" must be true or false")?)
            }
            "card.headerIcons" => {
                theme.card.header_icons = Some(
                    v.as_bool()
                        .ok_or("\"card.headerIcons\" must be true or false")?,
                )
            }
            "card.headerButtons" => theme.card.header_buttons = header_buttons(key, v)?,
            "card.headerButtonsActiveOnly" => {
                theme.card.header_buttons_active_only = Some(
                    v.as_bool()
                        .ok_or("\"card.headerButtonsActiveOnly\" must be true or false")?,
                )
            }
            _ => match LOOK_KEYS.iter().find(|(k, _)| *k == key) {
                Some((_, kind)) => {
                    check_kind(key, *kind, v)?;
                    theme.look.insert(key.clone(), v.clone());
                }
                None => return Err(format!("unknown key \"{key}\"")),
            },
        }
    }
    Ok(theme)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{default_config, merge_config};
    use serde_json::json;

    #[test]
    fn empty_text_is_the_built_in_look() {
        assert_eq!(parse("").unwrap(), UiTheme::default());
        assert_eq!(parse("  \n").unwrap(), UiTheme::default());
        assert_eq!(parse("// only a note\n{}").unwrap(), UiTheme::default());
    }

    #[test]
    fn parses_the_bars_flat_with_comments_and_trailing_commas() {
        let t = parse(
            r##"{
              // flat
              "titlebar.background": "canvas",
              "titlebar.border": "none",
              "statusbar.background": "#101216",
              "statusbar.border": "card",
            }"##,
        )
        .unwrap();
        assert_eq!(t.titlebar.background, Some(ColorSpec::Canvas));
        assert_eq!(t.titlebar.border, Some(BorderSpec::None));
        assert_eq!(
            t.statusbar.background,
            Some(ColorSpec::Hex("#101216".into()))
        );
        assert_eq!(t.statusbar.border, Some(BorderSpec::Color(ColorSpec::Card)));
        assert!(t.look.is_empty());
    }

    #[test]
    fn short_hex_and_every_token_are_accepted() {
        for v in ["#abc", "#A1B2C3", "canvas", "card", "bar", "transparent"] {
            assert!(
                parse(&format!(r#"{{"titlebar.background": "{v}"}}"#)).is_ok(),
                "{v}"
            );
        }
    }

    #[test]
    fn a_background_cannot_be_none() {
        // "none" is a border word; a background that vanishes is "transparent".
        assert!(parse(r#"{"titlebar.background": "none"}"#).is_err());
    }

    #[test]
    fn bad_bar_values_name_the_key() {
        for bad in [
            r##"{"titlebar.background": "#12"}"##,
            r##"{"titlebar.background": "#12345g"}"##,
            r#"{"titlebar.background": 5}"#,
            r#"{"statusbar.border": true}"#,
            r#"{"statusbar.border": "thick"}"#,
        ] {
            let e = parse(bad).unwrap_err();
            assert!(e.contains("titlebar") || e.contains("statusbar"), "{e}");
        }
    }

    #[test]
    fn look_keys_keep_their_values_as_written() {
        let t = parse(
            r#"{
              "ui.cardRadius": 12,
              "ui.showGrid": false,
              "ui.windowColor": "teal",
              "ui.backgroundImage": ["dusk", "~/a.png"],
              "ui.hideLabelWhenFitted": true,
            }"#,
        )
        .unwrap();
        assert_eq!(t.look["ui.cardRadius"], json!(12));
        assert_eq!(t.look["ui.showGrid"], json!(false));
        assert_eq!(t.look["ui.backgroundImage"], json!(["dusk", "~/a.png"]));
        assert_eq!(t.look.len(), 5);
    }

    #[test]
    fn a_look_value_of_the_wrong_kind_rejects_the_file_naming_the_key() {
        for (bad, key) in [
            (r#"{"ui.cardRadius": "big"}"#, "ui.cardRadius"),
            (r#"{"ui.showGrid": "no"}"#, "ui.showGrid"),
            (r#"{"ui.windowColor": 5}"#, "ui.windowColor"),
            (r#"{"ui.backgroundImage": [1]}"#, "ui.backgroundImage"),
            (r#"{"ui.backgroundImage": 3}"#, "ui.backgroundImage"),
        ] {
            let e = parse(bad).unwrap_err();
            assert!(e.contains(key), "{e}");
        }
    }

    #[test]
    fn tab_keys_parse_and_an_unset_tab_is_the_default_look() {
        assert_eq!(parse("{}").unwrap().tab, TabTheme::default());
        let t = parse(
            r##"{
              "tab.background": "#ffffff14",
              "tab.activeBackground": "canvas",
              "tab.gradient": ["#5a4a42", "#2b2320cc"],
              "tab.opacity": 0.8,
              "tab.border": "none",
              "tab.activeBorder": "#fff",
              "tab.separator": "#ffffff22",
              "tab.radius": 9,
            }"##,
        )
        .unwrap()
        .tab;
        assert_eq!(t.background, Some(ColorSpec::Hex("#ffffff14".into())));
        assert_eq!(t.active_background, Some(ColorSpec::Canvas));
        assert_eq!(t.gradient, Some(["#5a4a42".into(), "#2b2320cc".into()]));
        assert_eq!(t.opacity, Some(0.8));
        assert_eq!(t.border, Some(BorderSpec::None));
        assert_eq!(
            t.active_border,
            Some(BorderSpec::Color(ColorSpec::Hex("#fff".into())))
        );
        assert_eq!(
            t.separator,
            Some(BorderSpec::Color(ColorSpec::Hex("#ffffff22".into())))
        );
        assert_eq!(t.radius, Some(9.));
        assert_eq!(parse(r#"{"tab.gradient": []}"#).unwrap().tab.gradient, None);
    }

    #[test]
    fn tab_icon_keys_parse_and_are_off_unless_set() {
        let none = parse("{}").unwrap().tab;
        assert_eq!((none.icons, none.icon_style), (None, None));
        assert!(none.icon_map.is_empty());
        let t = parse(
            r#"{
              "tab.icons": true,
              "tab.iconStyle": "image",
              "tab.iconMap": {"Bun": "bun.svg", "-fish": "F", "default": "default.png"},
            }"#,
        )
        .unwrap()
        .tab;
        assert_eq!(t.icons, Some(true));
        assert_eq!(t.icon_style, Some(crate::tab_icon::IconStyle::Image));
        assert_eq!(t.icon_map["bun"], "bun.svg");
        assert_eq!(t.icon_map["fish"], "F", "keyed like a process name");
        assert_eq!(t.icon_map["default"], "default.png");
    }

    #[test]
    fn card_header_keys_parse_and_the_header_is_off_unless_set() {
        let none = parse("{}").unwrap().card;
        assert_eq!(none, CardTheme::default());
        assert!(!none.header_on());
        let c = parse(
            r#"{
              "card.header": true,
              "card.headerIcons": true,
              "card.headerButtons": ["close", "split-right"],
            }"#,
        )
        .unwrap()
        .card;
        assert!(c.header_on());
        assert_eq!(c.header_icons, Some(true));
        use crate::card_header::HeaderButton::*;
        assert_eq!(c.header_buttons, vec![Close, SplitRight]);
        assert!(!parse(r#"{"card.header": false}"#).unwrap().card.header_on());
    }

    #[test]
    fn the_buttons_can_be_limited_to_the_focused_card() {
        use crate::card_header::HeaderButton::*;
        let all = parse(r#"{"card.headerButtons": ["split-right", "close"]}"#)
            .unwrap()
            .card;
        assert_eq!(all.buttons_for(false), &[SplitRight, Close]);
        assert_eq!(all.buttons_for(true), &[SplitRight, Close]);
        let only =
            parse(r#"{"card.headerButtons": ["close"], "card.headerButtonsActiveOnly": true}"#)
                .unwrap()
                .card;
        assert_eq!(only.buttons_for(true), &[Close]);
        assert!(only.buttons_for(false).is_empty());
        let off =
            parse(r#"{"card.headerButtons": ["close"], "card.headerButtonsActiveOnly": false}"#)
                .unwrap()
                .card;
        assert_eq!(off.buttons_for(false), &[Close]);
    }

    #[test]
    fn bad_card_header_values_name_the_key() {
        for (bad, key) in [
            (
                r#"{"card.headerButtonsActiveOnly": "yes"}"#,
                "card.headerButtonsActiveOnly",
            ),
            (r#"{"card.header": "yes"}"#, "card.header"),
            (r#"{"card.headerIcons": 1}"#, "card.headerIcons"),
            (r#"{"card.headerButtons": "close"}"#, "card.headerButtons"),
            (
                r#"{"card.headerButtons": ["minimise"]}"#,
                "card.headerButtons",
            ),
            (r#"{"card.headerButtons": [1]}"#, "card.headerButtons"),
        ] {
            let e = parse(bad).unwrap_err();
            assert!(e.contains(key), "{e}");
        }
    }

    #[test]
    fn bad_tab_icon_values_name_the_key() {
        for (bad, key) in [
            (r#"{"tab.icons": "yes"}"#, "tab.icons"),
            (r#"{"tab.iconStyle": "svg"}"#, "tab.iconStyle"),
            (r#"{"tab.iconStyle": 1}"#, "tab.iconStyle"),
            (r#"{"tab.iconMap": ["bun"]}"#, "tab.iconMap"),
            (r#"{"tab.iconMap": {"bun": 3}}"#, "tab.iconMap"),
            (r#"{"tab.iconMap": {"bun": ""}}"#, "tab.iconMap"),
            (r#"{"tab.iconMap": {"bun": "../bun.svg"}}"#, "tab.iconMap"),
            (r#"{"tab.iconMap": {"bun": "a/b.png"}}"#, "tab.iconMap"),
        ] {
            let e = parse(bad).unwrap_err();
            assert!(e.contains(key), "{e}");
        }
    }

    #[test]
    fn icon_sizes_are_unset_by_default_clamp_and_name_the_key_when_bad() {
        let none = parse("{}").unwrap();
        assert_eq!(
            (none.tab.icon_size, none.card.header_icon_size),
            (None, None)
        );
        let t = parse(r#"{"tab.iconSize": 14, "card.headerIconSize": 18.5}"#).unwrap();
        assert_eq!(t.tab.icon_size, Some(14.));
        assert_eq!(t.card.header_icon_size, Some(18.5));
        let t = parse(r#"{"tab.iconSize": 1, "card.headerIconSize": 500}"#).unwrap();
        assert_eq!(t.tab.icon_size, Some(crate::tab_icon::ICON_SIZE_MIN));
        assert_eq!(
            t.card.header_icon_size,
            Some(crate::tab_icon::ICON_SIZE_MAX)
        );
        for (bad, key) in [
            (r#"{"tab.iconSize": "big"}"#, "tab.iconSize"),
            (r#"{"card.headerIconSize": true}"#, "card.headerIconSize"),
        ] {
            let e = parse(bad).unwrap_err();
            assert!(e.contains(key), "{e}");
        }
    }

    #[test]
    fn tab_numbers_clamp_and_bad_tab_values_name_the_key() {
        let t = parse(r#"{"tab.opacity": 0, "tab.radius": 99}"#)
            .unwrap()
            .tab;
        assert_eq!((t.opacity, t.radius), (Some(0.1), Some(40.)));
        for (bad, key) in [
            (r#"{"tab.background": "none"}"#, "tab.background"),
            (r#"{"tab.activeBackground": 3}"#, "tab.activeBackground"),
            (r##"{"tab.gradient": ["#fff"]}"##, "tab.gradient"),
            (r##"{"tab.gradient": ["#fff", "red"]}"##, "tab.gradient"),
            (r##"{"tab.gradient": "#fff"}"##, "tab.gradient"),
            (r#"{"tab.opacity": "half"}"#, "tab.opacity"),
            (r#"{"tab.radius": true}"#, "tab.radius"),
            (r#"{"tab.separator": "thin"}"#, "tab.separator"),
            (r#"{"tab.shadow": 1}"#, "tab.shadow"),
        ] {
            let e = parse(bad).unwrap_err();
            assert!(e.contains(key), "{e}");
        }
    }

    #[test]
    fn a_card_gradient_is_two_stops_or_off() {
        let t = parse(
            r##"{"ui.cardGradient": ["#3a2c27", "#1f1a18cc"], "ui.cardGradientAngle": 170}"##,
        )
        .unwrap();
        assert_eq!(t.look["ui.cardGradient"], json!(["#3a2c27", "#1f1a18cc"]));
        assert!(parse(r#"{"ui.cardGradient": []}"#).is_ok());
        for bad in [
            r##"{"ui.cardGradient": ["#fff"]}"##,
            r##"{"ui.cardGradient": ["#fff", "#000", "#123"]}"##,
            r##"{"ui.cardGradient": ["#fff", "red"]}"##,
            r##"{"ui.cardGradient": ["#fff", "#12345"]}"##,
            r##"{"ui.cardGradient": "#fff"}"##,
            r##"{"ui.cardGradientAngle": "up"}"##,
        ] {
            let e = parse(bad).unwrap_err();
            assert!(e.contains("ui.cardGradient"), "{e}");
        }
    }

    #[test]
    fn ui_icon_takes_default_or_filled_and_nothing_else() {
        assert_eq!(
            parse(r#"{"ui.icon": "filled"}"#).unwrap().look["ui.icon"],
            json!("filled")
        );
        assert!(parse(r#"{"ui.icon": "default"}"#).is_ok());
        for bad in [r#""outline""#, r#""Filled""#, "1", "true", r#""""#] {
            let e = parse(&format!(r#"{{"ui.icon": {bad}}}"#)).unwrap_err();
            assert!(
                e.contains("ui.icon") && e.contains("\"default\" or \"filled\""),
                "{e}"
            );
        }
    }

    #[test]
    fn a_typed_ui_icon_in_settings_json_is_dropped_with_its_name() {
        let (rest, dropped) = strip_look(&json!({"ui.icon": "filled", "ui": {"icon": "filled"}}));
        assert_eq!(dropped, ["ui.icon", "ui.icon"]);
        assert_eq!(rest, json!({"ui": {}}));
    }

    // A behaviour key is not a look key: it stays in settings.json.
    #[test]
    fn a_behaviour_key_or_a_typo_rejects_the_file() {
        for key in ["ui.panStep", "ui.animations", "titlebar.colour", "theme"] {
            let e = parse(&format!(r#"{{"{key}": 1}}"#)).unwrap_err();
            assert!(e.contains(key), "{e}");
        }
    }

    #[test]
    fn not_an_object_or_not_json_is_an_error() {
        assert!(parse("[1]").is_err());
        assert!(parse("\"canvas\"").is_err());
        assert!(parse("{").is_err());
    }

    // The three lists that name a look key must agree.
    #[test]
    fn every_look_key_is_a_ui_field_and_is_documented() {
        let ui = serde_json::to_value(default_config().ui).unwrap();
        for (key, _) in LOOK_KEYS {
            let short = key.strip_prefix("ui.").unwrap();
            assert!(ui.get(short).is_some(), "{key} is not a Ui field");
            assert!(
                !crate::settings_doc::doc(key).is_empty(),
                "{key} has no doc"
            );
        }
    }

    #[test]
    fn every_look_key_is_copied_by_take_look_from() {
        for (key, kind) in LOOK_KEYS {
            let sample = match (key, kind) {
                ("ui.backgroundImageFit", _) => json!("contain"),
                ("ui.showGrid", _) => json!(false),
                ("ui.icon", _) => json!("filled"),
                (_, Kind::Bool) => json!(true),
                (_, Kind::Number) => json!(0.5),
                (_, Kind::Text) => json!("x"),
                (_, Kind::TextOrList) => json!(["dusk"]),
                (_, Kind::ColorPair) => json!(["#112233", "#445566ff"]),
                (_, Kind::Choice(_)) => unreachable!("named above"),
            };
            let look = merge_config(&json!({ key: sample })).ui;
            let mut ui = default_config().ui;
            ui.take_look_from(&look);
            assert_ne!(ui, default_config().ui, "{key} is not copied");
        }
    }

    #[test]
    fn take_look_from_leaves_the_behaviour_alone() {
        let look = merge_config(&json!({"ui.panStep": 99, "ui.cardRadius": 9})).ui;
        let mut ui = default_config().ui;
        ui.take_look_from(&look);
        assert_eq!(ui.card_radius, 9.);
        assert_eq!(ui.pan_step, default_config().ui.pan_step);
    }

    #[test]
    fn strip_look_drops_both_spellings_and_keeps_the_rest() {
        let raw = json!({
            "terminal.fontSize": 16,
            "ui.cardRadius": 8,
            "ui.panStep": 100,
            "ui": {"showGrid": false, "animations": false, "windowOpacity": 0.9},
        });
        let (kept, mut dropped) = strip_look(&raw);
        dropped.sort();
        assert_eq!(
            dropped,
            ["ui.cardRadius", "ui.showGrid", "ui.windowOpacity"]
        );
        assert_eq!(
            kept,
            json!({
                "terminal.fontSize": 16,
                "ui.panStep": 100,
                "ui": {"animations": false},
            })
        );
        assert_eq!(strip_look(&json!([1])), (json!([1]), vec![]));
    }

    #[test]
    fn names_must_be_one_plain_path_component() {
        assert!(valid_name("default"));
        assert!(valid_name("Glass 2"));
        for bad in ["", "..", ".hidden", "a/b", "../x", "a\\b", " pad"] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_picker_lists_default_first_then_a_to_z() {
        let names = names_from_stems(
            ["zen", "Glass", "default", "aurora", "glass", ".x", "a/b"]
                .iter()
                .map(|s| s.to_string()),
        );
        assert_eq!(names, ["default", "aurora", "Glass", "glass", "zen"]);
        assert_eq!(names_from_stems(vec![]), ["default"]);
    }
}
