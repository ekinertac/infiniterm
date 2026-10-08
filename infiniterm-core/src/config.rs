//! The settings schema: every value, its default, and how a file is read
//! over it. Port of config.ts and its tests.
//!
//! The user's file holds only OVERRIDES (`~/.config/infiniterm/settings.json`)
//! and `settings.default.json` beside it documents the rest (`settings_doc.rs`
//! renders that one from `DEFAULT_CONFIG`). That split is why there is no
//! backfill: a new setting reaches an existing install through a file nobody
//! edits.
//!
//! Keys are FLAT and dotted, VS Code's shape: `"terminal.decoyCommand": "top"`
//! is the whole override, one line copied from the defaults file. The nested
//! shape the files had first (`"terminal": { "decoyCommand": ... }`) is still
//! read, because every install from before 2026-09-18 has one: `flatten`
//! turns it into dotted keys before anything looks, and a key given both
//! ways takes the flat one. Nothing writes the nested shape any more.
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

use crate::viewport::FIT_PADDING;

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
    /// with the window; `tmux` and `daemon` both keep it running when the
    /// app quits, reachable again from any terminal (`tmux attach -t
    /// infiniterm`, `ift attach <id>`).
    ///
    /// pty is the DEFAULT here (Task 7 flips it to `daemon` once the
    /// falsification scenario in `tools/drive/daemon.sh` has run). tmux
    /// works for shells and for programs that repaint a whole screen, but a
    /// program that redraws INLINE, moving the cursor up and erasing a line
    /// rather than clearing, needs our grid's scroll position to match
    /// exactly what it believes. Two emulators track one program under
    /// tmux, with a replayed history in between, and when they disagree by
    /// a row that kind of redraw lands wrong and never heals, because it
    /// never clears. Claude Code is one. `daemon` (`iftd`, one per card) has
    /// no second emulator in the path at all: it never parses VT, so a
    /// reattach replays the exact bytes our own parser would have seen live.
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
    /// MiB of raw output the `daemon` backend keeps per card, replayed on
    /// reattach. Ignored by `pty` and `tmux`. 4 is the design's starting
    /// guess, not a measured floor; Ink's repaints are large and repetitive
    /// enough that a Claude card may need more, which is the whole reason
    /// this is a setting and not a constant.
    pub session_buffer: f64,
    /// What `card.mask` runs over a card when someone is reading your
    /// screen: a program that looks like work and says nothing. Empty
    /// falls back to the default.
    pub decoy_command: String,
    /// A mouse selection goes to the clipboard as it is made (iTerm2's
    /// "copy to pasteboard on selection").
    pub copy_on_select: bool,
    /// Lines per wheel step, as a multiple of the normal rate.
    pub scroll_multiplier: f64,
    /// Space between a card's edge and its text, in points at 100%.
    pub padding: f64,
    /// `KEY=value` pairs added to every new card's environment.
    pub env: Vec<String>,
}

/// How the background picture fills the window (`ui.backgroundImageFit`,
/// #97): `cover` fills it and crops, `contain` shows all of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundFit {
    Cover,
    Contain,
}

/// A card label's corner (`ui.cardLabelPosition`). Written "top right";
/// "right top" reads the same (`label_corner`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LabelCorner {
    #[serde(rename = "top right")]
    TopRight,
    #[serde(rename = "top left")]
    TopLeft,
    #[serde(rename = "bottom left")]
    BottomLeft,
    #[serde(rename = "bottom right")]
    BottomRight,
}

impl LabelCorner {
    pub fn left(self) -> bool {
        matches!(self, LabelCorner::TopLeft | LabelCorner::BottomLeft)
    }
    pub fn bottom(self) -> bool {
        matches!(self, LabelCorner::BottomLeft | LabelCorner::BottomRight)
    }
}

/// `ui.cardLabelPosition`, either word first, any case; anything else keeps
/// the fallback.
fn label_corner(value: Option<&Value>, fallback: LabelCorner) -> LabelCorner {
    let Some(s) = value.and_then(Value::as_str) else {
        return fallback;
    };
    let words: Vec<String> = s.split_whitespace().map(str::to_lowercase).collect();
    let has = |w: &str| words.iter().any(|x| x == w);
    if words.len() != 2 {
        return fallback;
    }
    match (has("top"), has("bottom"), has("left"), has("right")) {
        (true, false, false, true) => LabelCorner::TopRight,
        (true, false, true, false) => LabelCorner::TopLeft,
        (false, true, true, false) => LabelCorner::BottomLeft,
        (false, true, false, true) => LabelCorner::BottomRight,
        _ => fallback,
    }
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
    Daemon,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cards {
    /// Size of a new card, in 25px grid cells; 0 (the default) sizes it
    /// from the window (`cards::auto_size`).
    pub width: f64,
    pub height: f64,
    /// The shape of a card sized from the window: a ratio like "16:9"
    /// (the default) or "window" for the window's own shape
    /// (`cards::parse_shape`).
    pub shape: String,
    /// Whether a card carved out of another (a split) keeps its
    /// directory. A plain new card always starts at `starting_dir`.
    pub inherit_directory: bool,
    /// The space between cards, in pixels (`cards.gap`). Every placement,
    /// split, tidy and growth works from it; 25 is the grid.
    pub gap: f64,
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
    /// How much the whole canvas is dimmed while another app is in front.
    pub unfocused_dim: f64,
    /// Alpha of the window's own fill: canvas, title bar, status bar (#84).
    pub window_opacity: f64,
    /// Blur what is behind a see-through window; nothing at opacity 1.
    pub window_blur: bool,
    /// Bundled picture names or paths, in the order they rotate; empty for
    /// none (#97, a list since #210). One string in the file is a list of one.
    pub background_image: Vec<String>,
    pub background_image_fit: BackgroundFit,
    /// Seconds a picture stays before the next one fades in.
    pub background_image_interval: f64,
    /// Seconds the crossfade takes; 0 changes pictures at once.
    pub background_image_fade: f64,
    /// Ctrl+Tab and the status bar's card count look at the current
    /// workspace only (#101).
    pub workspace_isolation: bool,
    /// A colour for the title bar and the Dock icon: a name from the picker or
    /// a hex code, empty for none (#160). A remote instance wears its host's.
    pub window_color: String,
    /// Alpha of a card's background fill, text untouched (#98).
    pub card_opacity: f64,
    /// Rounded card corners, in pixels at 100% zoom; 0 keeps them square (#235).
    pub card_radius: f64,
    /// Whether the canvas draws its grid (#96).
    /// Canvas scale reading mode zooms to (#313): 1.5 is 150%.
    pub read_zoom: f64,
    pub show_grid: bool,
    /// Screen pixels left around a card or cluster when it is fitted (#87).
    pub fit_padding: f64,
    /// Whether a fit may zoom past 100% to fill the window.
    pub fit_magnify: bool,
    /// Whether Cmd+1 on a card that was split frames the whole slot (the
    /// card and the pieces it was split from), as it always did. Off: it fits
    /// the card itself, split or not.
    pub fit_split_slot: bool,
    /// Chrome font sizes in SCREEN pixels, before the UI-scale multiplier.
    /// Three values because they are read at three distances: a card label
    /// has to survive 10% zoom, a group name sits above a block, the status
    /// bar is always at arm's length.
    pub card_label_size: f64,
    /// Which corner of a card its label sits in (#71).
    pub card_label_position: LabelCorner,
    /// No label on the card a maximised view fills: the status bar names it.
    pub hide_label_when_maximised: bool,
    pub group_label_size: f64,
    pub status_bar_size: f64,
    /// Whether the canvas animates at all. Reduced motion still wins.
    pub animations: bool,
    /// The frame counter in the status bar. On by default because the point
    /// of it is to be there when something stutters; it holds a permanent
    /// frame loop while on.
    pub show_fps: bool,
    /// What the green button and `app.fullscreen` do: macOS's own full
    /// screen, or the window over the whole screen, notch strip
    /// included (`fullscreen.rs`).
    pub fullscreen: FullscreenMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FullscreenMode {
    /// macOS's full screen: its own Space, content below the notch.
    Native,
    /// The window covers the whole screen, beside the notch too.
    Cover,
}

/// Page zoom limits for browser cards, the range Safari allows.
pub const BROWSER_ZOOM_MIN: f64 = 0.3;
pub const BROWSER_ZOOM_MAX: f64 = 3.;

pub fn default_config() -> Config {
    Config {
        // Bundled with the app (assets/themes), so a fresh install has
        // colours rather than the fallback palette. Violite since
        // 2026-09-30: Ekin's own, chosen over Catppuccin Mocha for a first
        // launch.
        theme: Some("Violite".into()),
        starting_dir: String::new(),
        terminal: Terminal {
            shell: String::new(),
            backend: TerminalBackend::Daemon,
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
            session_buffer: 4.,
            copy_on_select: false,
            scroll_multiplier: 1.,
            // The inset cards have had since the first day
            // (terminal_body.rs `PAD`).
            padding: 6.,
            env: vec![],
            // Live, dense, unreadable at a glance, on every Mac. Not
            // system.log: unified logging left it a line every ten minutes.
            // `command` bypasses zsh's own builtin `log` (a login shell
            // always checks builtins before PATH), which otherwise takes
            // the args meant for /usr/bin/log and answers "too many
            // arguments": the decoy showed an error instead of the log.
            decoy_command: "command log stream --style compact".into(),
        },
        cards: Cards {
            // From the window: 69 by 80 cells, portrait, was the default
            // until 2026-09-24 and suited one 32-inch 4K screen; on a
            // laptop it is a tall sliver. 0 means "size it from the window".
            width: 0.,
            height: 0.,
            shape: "16:9".into(),
            inherit_directory: true,
            gap: crate::cards::GUTTER,
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
            unfocused_dim: 0.4,
            window_opacity: 1.,
            window_blur: false,
            background_image: Vec::new(),
            background_image_fit: BackgroundFit::Cover,
            background_image_interval: 300.,
            background_image_fade: 2.,
            workspace_isolation: false,
            window_color: String::new(),
            card_opacity: 1.,
            card_radius: 0.,
            read_zoom: 1.5,
            show_grid: true,
            fit_padding: FIT_PADDING,
            fit_magnify: false,
            fit_split_slot: true,
            card_label_size: 15.,
            card_label_position: LabelCorner::TopRight,
            hide_label_when_maximised: false,
            group_label_size: 15.,
            status_bar_size: 11.,
            animations: true,
            // A development number; the release does not show it by
            // default (issue #16).
            show_fps: false,
            // Native full screen on a notched MacBook leaves the strip
            // beside the camera black; cover puts the title bar there.
            fullscreen: FullscreenMode::Cover,
        },
    }
}

/// A card size in cells: 0 is "from the window" and kept; anything else is
/// whole cells between `min` and 400.
fn cells(value: Option<&Value>, fallback: f64, min: f64) -> f64 {
    match value.and_then(Value::as_f64) {
        Some(0.) => 0.,
        _ => num(value, fallback, min, 400.).round(),
    }
}

fn num(value: Option<&Value>, fallback: f64, min: f64, max: f64) -> f64 {
    match value.and_then(Value::as_f64) {
        Some(n) if n.is_finite() => n.clamp(min, max),
        _ => fallback,
    }
}

/// A setting that is one string or a list of them (`ui.backgroundImage`):
/// the trimmed, non-empty strings, in order. Anything else is an empty list.
fn string_or_list(value: Option<&Value>) -> Vec<String> {
    let one = |v: &Value| v.as_str().map(|s| s.trim().to_string());
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(one)
            .filter(|s| !s.is_empty())
            .collect(),
        Some(v) => one(v).filter(|s| !s.is_empty()).into_iter().collect(),
        None => Vec::new(),
    }
}

/// `terminal.env`: the `KEY=value` strings with a key before the `=`;
/// anything else in the list is skipped rather than failing the file.
fn env_pairs(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter(|s| s.split_once('=').is_some_and(|(k, _)| !k.trim().is_empty()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
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

/// Every setting in `raw` by its dotted name, whichever shape wrote it.
/// Nested objects are walked into dotted keys; a key that already holds a
/// dot is taken as it is and, entered last, wins over the same setting
/// spelled nested. Arrays are values (`browser.engines`), never walked.
pub fn flatten(raw: &serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
    fn walk(prefix: &str, value: &Value, out: &mut serde_json::Map<String, Value>) {
        match value.as_object() {
            Some(map) => {
                for (k, v) in map {
                    let key = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    walk(&key, v, out);
                }
            }
            None => {
                out.insert(prefix.to_string(), value.clone());
            }
        }
    }
    let mut out = serde_json::Map::new();
    for (k, v) in raw.iter().filter(|(k, _)| !k.contains('.')) {
        walk(k, v, &mut out);
    }
    for (k, v) in raw.iter().filter(|(k, _)| k.contains('.')) {
        out.insert(k.clone(), v.clone());
    }
    out
}

/// The settings under `prefix.` in a flattened file, by their short name,
/// so a section's fields read as `t.get("shell")`.
fn group(flat: &serde_json::Map<String, Value>, prefix: &str) -> serde_json::Map<String, Value> {
    let head = format!("{prefix}.");
    flat.iter()
        .filter_map(|(k, v)| {
            k.strip_prefix(&head)
                .map(|rest| (rest.to_string(), v.clone()))
        })
        .collect()
}

/// Merges a parsed file over the defaults, clamping anything out of range.
pub fn merge_config(raw: &Value) -> Config {
    let d = default_config();
    let Some(raw) = raw.as_object() else { return d };
    let flat = flatten(raw);
    let r = &flat;
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
                    ("daemon", TerminalBackend::Daemon),
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
            session_buffer: num(t.get("sessionBuffer"), d.terminal.session_buffer, 1., 64.),
            decoy_command: match trimmed(t.get("decoyCommand"), &d.terminal.decoy_command) {
                s if s.is_empty() => d.terminal.decoy_command.clone(),
                s => s,
            },
            copy_on_select: bool_(t.get("copyOnSelect"), d.terminal.copy_on_select),
            scroll_multiplier: num(
                t.get("scrollMultiplier"),
                d.terminal.scroll_multiplier,
                0.1,
                10.,
            ),
            padding: num(t.get("padding"), d.terminal.padding, 0., 60.),
            env: env_pairs(t.get("env")),
        },
        cards: Cards {
            // Floors that keep a card usable: below about 40 columns a
            // terminal stops being one; the ceiling is what placement can
            // still fit.
            // 0 stays 0: "from the window", not a card 12 cells wide.
            width: cells(c.get("width"), d.cards.width, 12.),
            height: cells(c.get("height"), d.cards.height, 8.),
            // A value that is neither "window" nor a ratio keeps the default.
            shape: match trimmed(c.get("shape"), &d.cards.shape) {
                s if crate::cards::shape_is_valid(&s) => s,
                _ => d.cards.shape.clone(),
            },
            inherit_directory: bool_(c.get("inheritDirectory"), d.cards.inherit_directory),
            // 0 packs cards edge to edge; above 200 a gap is a gulf and the
            // placement maths has no use for it.
            gap: num(c.get("gap"), d.cards.gap, 0., 200.),
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
            unfocused_dim: num(u.get("unfocusedDim"), d.ui.unfocused_dim, 0., 1.),
            window_opacity: num(u.get("windowOpacity"), d.ui.window_opacity, 0.1, 1.),
            window_blur: bool_(u.get("windowBlur"), d.ui.window_blur),
            background_image: string_or_list(u.get("backgroundImage")),
            background_image_interval: num(
                u.get("backgroundImageInterval"),
                d.ui.background_image_interval,
                5.,
                86_400.,
            ),
            background_image_fade: num(
                u.get("backgroundImageFade"),
                d.ui.background_image_fade,
                0.,
                30.,
            ),
            background_image_fit: match u.get("backgroundImageFit").and_then(Value::as_str) {
                Some(s) if s.eq_ignore_ascii_case("contain") => BackgroundFit::Contain,
                _ => BackgroundFit::Cover,
            },
            workspace_isolation: bool_(u.get("workspaceIsolation"), d.ui.workspace_isolation),
            window_color: u
                .get("windowColor")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
            card_opacity: num(u.get("cardOpacity"), d.ui.card_opacity, 0.1, 1.),
            card_radius: num(u.get("cardRadius"), d.ui.card_radius, 0., 40.),
            read_zoom: num(u.get("readZoom"), d.ui.read_zoom, 1.1, 4.),
            show_grid: bool_(u.get("showGrid"), d.ui.show_grid),
            fit_padding: num(u.get("fitPadding"), d.ui.fit_padding, 0., 500.),
            fit_magnify: bool_(u.get("fitMagnify"), d.ui.fit_magnify),
            fit_split_slot: bool_(u.get("fitSplitSlot"), d.ui.fit_split_slot),
            card_label_size: num(u.get("cardLabelSize"), d.ui.card_label_size, 6., 64.),
            card_label_position: label_corner(u.get("cardLabelPosition"), d.ui.card_label_position),
            hide_label_when_maximised: bool_(
                u.get("hideLabelWhenMaximised"),
                d.ui.hide_label_when_maximised,
            ),
            group_label_size: num(u.get("groupLabelSize"), d.ui.group_label_size, 6., 64.),
            status_bar_size: num(u.get("statusBarSize"), d.ui.status_bar_size, 6., 32.),
            animations: bool_(u.get("animations"), d.ui.animations),
            show_fps: bool_(u.get("showFps"), d.ui.show_fps),
            fullscreen: one(
                u.get("fullscreen"),
                d.ui.fullscreen,
                &[
                    ("native", FullscreenMode::Native),
                    ("cover", FullscreenMode::Cover),
                ],
            ),
        },
    }
}

#[cfg(test)]
mod tests {

    // Either word first, any case; nonsense keeps the default (#71).
    #[test]
    fn the_label_corner_reads_either_way_round() {
        let c = |v: &str| {
            merge_config(&serde_json::json!({ "ui.cardLabelPosition": v }))
                .ui
                .card_label_position
        };
        assert_eq!(c("bottom left"), LabelCorner::BottomLeft);
        assert_eq!(c("Left Bottom"), LabelCorner::BottomLeft);
        assert_eq!(c("right top"), LabelCorner::TopRight);
        assert_eq!(c("top top"), LabelCorner::TopRight);
        assert_eq!(c("middle"), LabelCorner::TopRight);
        assert_eq!(
            default_config().ui.card_label_position,
            LabelCorner::TopRight
        );
    }

    // The four settings added for people coming from other terminals.
    #[test]
    fn copy_on_select_scroll_padding_and_env_are_read_and_clamped() {
        let c = merge_config(&serde_json::json!({
            "terminal.copyOnSelect": true,
            "terminal.scrollMultiplier": 50,
            "terminal.padding": 12,
            "terminal.env": ["EDITOR=ift", "no equals sign", "=novalue", 7, "LANG=en_US.UTF-8"],
        }));
        assert!(c.terminal.copy_on_select);
        assert_eq!(c.terminal.scroll_multiplier, 10., "clamped");
        assert_eq!(c.terminal.padding, 12.);
        assert_eq!(c.terminal.env, ["EDITOR=ift", "LANG=en_US.UTF-8"]);
        let d = default_config();
        assert!(!d.terminal.copy_on_select);
        assert_eq!((d.terminal.scroll_multiplier, d.terminal.padding), (1., 6.));
        assert!(d.terminal.env.is_empty());
    }

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
    fn the_backend_setting_names_three_things() {
        for (text, want) in [
            ("pty", TerminalBackend::Pty),
            ("tmux", TerminalBackend::Tmux),
            ("daemon", TerminalBackend::Daemon),
        ] {
            let c = m(json!({"terminal": {"backend": text}}));
            assert_eq!(c.terminal.backend, want, "{text}");
        }
        // A typo falls back to THE DEFAULT, whatever that currently is,
        // rather than breaking the file. Asked of `default_config` rather
        // than spelled out, so flipping the default cannot leave this test
        // asserting yesterday's answer.
        assert_eq!(
            m(json!({"terminal": {"backend": "nonsense"}}))
                .terminal
                .backend,
            default_config().terminal.backend
        );
    }

    #[test]
    fn full_screen_is_native_or_cover_and_cover_by_default() {
        assert_eq!(default_config().ui.fullscreen, FullscreenMode::Cover);
        assert_eq!(
            m(json!({"ui.fullscreen": "native"})).ui.fullscreen,
            FullscreenMode::Native
        );
        assert_eq!(
            m(json!({"ui": {"fullscreen": "sideways"}})).ui.fullscreen,
            FullscreenMode::Cover
        );
    }

    // A wash over the whole canvas while another app is in front. Clamped
    // like inactiveDim: 1 is a blackout and anything past it is a typo.
    #[test]
    fn the_unfocused_dim_is_a_fraction() {
        assert_eq!(
            m(json!({"ui": {"unfocusedDim": 0.7}})).ui.unfocused_dim,
            0.7
        );
        assert_eq!(m(json!({"ui": {"unfocusedDim": 0}})).ui.unfocused_dim, 0.);
        assert_eq!(m(json!({"ui": {"unfocusedDim": 5}})).ui.unfocused_dim, 1.);
    }

    #[test]
    fn a_partial_file_only_overrides_what_it_sets() {
        let c = m(json!({"terminal": {"fontSize": 14}}));
        let d = default_config();
        assert_eq!(c.terminal.font_size, 14.);
        assert_eq!(c.terminal.font_family, d.terminal.font_family);
        assert_eq!(c.ui.inactive_dim, d.ui.inactive_dim);
        assert_eq!(c.ui.unfocused_dim, d.ui.unfocused_dim);
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

    // The file's shape is the user's business: flat dotted keys are what
    // the defaults file shows, the nested shape is what older files have,
    // and a setting written both ways takes the flat one.
    #[test]
    fn flat_and_nested_keys_read_alike_and_flat_wins() {
        let flat = m(json!({"terminal.fontSize": 18, "ui.showFps": true}));
        assert_eq!(flat.terminal.font_size, 18.);
        assert!(flat.ui.show_fps);
        let nested = m(json!({"terminal": {"fontSize": 18}, "ui": {"showFps": true}}));
        assert_eq!(nested, flat);
        let both = m(json!({"terminal": {"fontSize": 18}, "terminal.fontSize": 20}));
        assert_eq!(both.terminal.font_size, 20.);
        // An array is a value, not a group to walk into.
        let engines = m(json!({"browser.engines": [
            {"keyword": "g", "name": "G", "searchUrl": "https://g/?q=%s"}
        ]}));
        assert_eq!(engines.browser.engines.len(), 1);
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
        assert_eq!(
            (c.width, c.height, c.inherit_directory),
            (0., 0., true),
            "from the window"
        );
        assert_eq!(m(json!({"cards": {"width": 0}})).cards.width, 0.);
        assert_eq!(
            m(json!({"cards": {"width": 69, "height": 80}}))
                .cards
                .height,
            80.
        );
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
    fn window_opacity_defaults_opaque_and_clamps() {
        assert_eq!(m(json!({})).ui.window_opacity, 1.);
        assert!(!m(json!({})).ui.window_blur);
        assert_eq!(m(json!({"ui.windowOpacity": 0.6})).ui.window_opacity, 0.6);
        assert_eq!(m(json!({"ui.windowOpacity": 0})).ui.window_opacity, 0.1);
        assert_eq!(m(json!({"ui.windowOpacity": 3})).ui.window_opacity, 1.);
        assert!(m(json!({"ui.windowBlur": true})).ui.window_blur);
    }

    #[test]
    fn the_background_image_settings_default_to_none_and_cover() {
        let ui = m(json!({})).ui;
        assert!(ui.background_image.is_empty());
        assert_eq!(ui.background_image_fit, BackgroundFit::Cover);
        let ui = m(json!({"ui.backgroundImage": " dusk ", "ui.backgroundImageFit": "Contain"})).ui;
        assert_eq!(ui.background_image, ["dusk"]);
        assert_eq!(ui.background_image_fit, BackgroundFit::Contain);
        assert_eq!(
            m(json!({"ui.backgroundImageFit": "stretch"}))
                .ui
                .background_image_fit,
            BackgroundFit::Cover
        );
    }

    #[test]
    fn the_background_image_is_a_string_or_a_list_with_a_timer_and_a_fade() {
        let ui = m(json!({"ui.backgroundImage": ["dusk", " ", "~/a.png", 7, "ember "]})).ui;
        assert_eq!(ui.background_image, ["dusk", "~/a.png", "ember"]);
        assert_eq!(ui.background_image_interval, 300.);
        assert_eq!(ui.background_image_fade, 2.);
        let ui = m(json!({
            "ui.backgroundImageInterval": 1,
            "ui.backgroundImageFade": 99
        }))
        .ui;
        assert_eq!(ui.background_image_interval, 5.);
        assert_eq!(ui.background_image_fade, 30.);
        assert_eq!(
            m(json!({"ui.backgroundImageFade": 0}))
                .ui
                .background_image_fade,
            0.
        );
        assert!(m(json!({"ui.backgroundImage": 3}))
            .ui
            .background_image
            .is_empty());
    }

    #[test]
    fn workspace_isolation_is_off_unless_asked_for() {
        assert!(!m(json!({})).ui.workspace_isolation);
        assert!(
            m(json!({"ui.workspaceIsolation": true}))
                .ui
                .workspace_isolation
        );
    }

    #[test]
    fn the_window_colour_is_none_unless_set() {
        assert_eq!(m(json!({})).ui.window_color, "");
        assert_eq!(
            m(json!({"ui.windowColor": " blue "})).ui.window_color,
            "blue"
        );
        assert_eq!(
            m(json!({"ui.windowColor": 5})).ui.window_color,
            "",
            "not a string"
        );
    }

    #[test]
    fn card_radius_defaults_to_square_and_clamps() {
        assert_eq!(m(json!({})).ui.card_radius, 0.);
        assert_eq!(m(json!({"ui.cardRadius": 12})).ui.card_radius, 12.);
        assert_eq!(m(json!({"ui.cardRadius": 500})).ui.card_radius, 40.);
        assert_eq!(m(json!({"ui.cardRadius": -3})).ui.card_radius, 0.);
        assert_eq!(m(json!({"ui.cardRadius": "big"})).ui.card_radius, 0.);
    }

    #[test]
    fn card_opacity_defaults_opaque_and_clamps() {
        assert_eq!(m(json!({})).ui.card_opacity, 1.);
        assert_eq!(m(json!({"ui.cardOpacity": 0.7})).ui.card_opacity, 0.7);
        assert_eq!(m(json!({"ui.cardOpacity": 0})).ui.card_opacity, 0.1);
        assert_eq!(m(json!({"ui.cardOpacity": 2})).ui.card_opacity, 1.);
    }

    #[test]
    fn the_grid_is_on_unless_turned_off() {
        assert_eq!(m(json!({})).ui.read_zoom, 1.5);
        assert_eq!(m(json!({"ui.readZoom": 2})).ui.read_zoom, 2.);
        assert_eq!(m(json!({"ui.readZoom": 9})).ui.read_zoom, 4.);
        assert_eq!(m(json!({"ui.readZoom": 1})).ui.read_zoom, 1.1);
        assert!(m(json!({})).ui.show_grid);
        assert!(!m(json!({"ui.showGrid": false})).ui.show_grid);
        assert!(m(json!({"ui.showGrid": "no"})).ui.show_grid);
    }

    #[test]
    fn a_boolean_setting_ignores_a_non_boolean() {
        assert!(m(json!({"ui": {"animations": "yes"}})).ui.animations);
        assert!(!m(json!({"ui": {"animations": false}})).ui.animations);
    }
}
