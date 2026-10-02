//! Generating `settings.default.json`: the read-only file that documents
//! every setting, Sublime style. Port of settingsDoc.ts and its tests.
//!
//! The comments live beside the defaults they describe and the file is
//! rendered from `default_config()`, never written by hand: a hand-written
//! defaults file drifts within one release, and a defaults file that lies is
//! worse than none, since it is the thing people copy from. Rewritten on
//! every launch, which is what makes "read-only" true rather than a request.
//!
//! `undocumented` is the guard: a new setting cannot ship without a line
//! saying what it does. `config.rs` owns the values; the settings writer in
//! the backend calls `render_settings_default` at startup.
//!
//! The file is FLAT: one `"terminal.fontSize": 14` per line, VS Code's
//! shape, so an override is one line copied across and nothing to nest. A
//! group's own doc line (`terminal`, `cards`) heads its run of keys.
use crate::config::{default_config, flatten, Config};
use serde_json::Value;

/// What each setting is for, by dotted key. Every leaf in the defaults has
/// an entry; the test keeps that true.
pub const SETTINGS_DOC: &[(&str, &[&str])] = &[
    (
        "theme",
        &[
            "Terminal colour scheme, by file name without the extension, from",
            "~/Library/Application Support/dev.ekinertac.infiniterm/themes/. Five",
            "hundred schemes are put there on first launch; any .itermcolors file",
            "dropped in is picked up too. \"Switch theme\" in the palette writes it here.",
        ],
    ),
    (
        "startingDir",
        &[
            "Where the first card's shell starts. Empty means your home directory.",
            "Cards opened afterwards inherit the directory of the card they were opened",
            "next to, so this is the root of that chain rather than an override.",
        ],
    ),
    (
        "terminal",
        &["The terminals themselves. Changes reach cards that are already open."],
    ),
    (
        "terminal.backend",
        &[
            "Where a card's shell lives: \"pty\", \"tmux\", or \"daemon\".",
            "pty is a plain local shell that dies with the window.",
            "tmux keeps the shell running when infiniterm quits, and the same",
            "sessions are reachable with `tmux attach -t infiniterm` from any",
            "terminal. Each card is one tmux window, but tmux is a second",
            "terminal emulator: a program that redraws INLINE, like Claude Code,",
            "can come out with two lines in one row, because two emulators are",
            "tracking one program and a redraw that never clears never recovers",
            "from a disagreement. Shells and full-screen programs are fine.",
            "daemon, the default, runs one small `iftd` sidecar per card: it parses no",
            "terminal output at all, so a reattach after infiniterm quits replays",
            "the exact bytes our own emulator would have seen live, with no second",
            "emulator to disagree with. `ift sessions` and `ift attach <id>` reach",
            "a daemon card's shell even with infiniterm not running.",
            "Neither tmux nor daemon is the default yet, pending more use.",
        ],
    ),
    (
        "terminal.shell",
        &[
            "Program to run in a new card, with any arguments (\"/bin/zsh -l\",",
            "\"fish --private\"). Empty uses your login shell ($SHELL).",
            "Only affects cards opened afterwards; a running shell is not restarted.",
        ],
    ),
    (
        "terminal.cursorStyle",
        &["\"block\", \"bar\", or \"underline\"."],
    ),
    ("terminal.cursorBlink", &["Whether the cursor blinks."]),
    (
        "terminal.fontFamily",
        &[
            "Any monospace family installed on the system. A Nerd Font is needed for the",
            "powerline segments and icons most shell prompts draw.",
        ],
    ),
    (
        "terminal.fontSize",
        &["In points. The cell size, and so the card grid, follows it."],
    ),
    (
        "terminal.fontWeight",
        &["Weight for normal text: \"normal\", \"bold\", or 100-900."],
    ),
    (
        "terminal.fontWeightBold",
        &["Weight for text the shell asks to be bold."],
    ),
    (
        "terminal.letterSpacing",
        &[
            "Extra pixels between characters; negative tightens. Narrow faces such as",
            "Iosevka read loosely tracked at 0, since xterm rounds the cell width up.",
        ],
    ),
    (
        "terminal.lineHeight",
        &["Multiple of the font size. 1.2 is tighter than the 1.25 a font asks for."],
    ),
    (
        "terminal.copyOnSelect",
        &["Copy a mouse selection to the clipboard as soon as it is made."],
    ),
    (
        "terminal.scrollMultiplier",
        &["Scroll speed: lines per wheel step, times this. 0.1 to 10."],
    ),
    (
        "terminal.padding",
        &["Space between a card's edge and its text, in points at 100% zoom. 0 to 60."],
    ),
    (
        "terminal.env",
        &[
            "Extra environment for every new card, as \"KEY=value\" strings,",
            "e.g. [\"EDITOR=ift\", \"LANG=en_US.UTF-8\"]. Cards already open keep theirs.",
        ],
    ),
    (
        "terminal.scrollback",
        &["Lines kept per card. Costs memory per card, not per app."],
    ),
    (
        "terminal.sessionBuffer",
        &[
            "MiB of raw output the \"daemon\" backend keeps per card, replayed on",
            "reattach. Ignored by \"pty\" and \"tmux\". Turn it down to shrink the",
            "worst case of many cards each holding a full buffer; turn it up if a",
            "card's early output is gone by the time you reattach to it.",
        ],
    ),
    (
        "terminal.decoyCommand",
        &[
            "What Cmd+Shift+H runs over the focused card when somebody is reading",
            "your screen: a program that looks like work and says nothing. The",
            "card underneath keeps running unseen; Enter or Escape brings it back.",
            "\"ping 8.8.8.8\" and \"top\" are the other classics. It runs through a",
            "login shell, not an interactive one, so a PATH set in .zshrc is not",
            "there: give a Homebrew program its full path, cron's rule",
            "(\"/opt/homebrew/bin/htop\").",
        ],
    ),
    (
        "cards",
        &["New cards: how big they are and where they start."],
    ),
    (
        "cards.width",
        &[
            "Width in 25px grid cells, or 0 to size new cards from the window: the canvas at",
            "100% minus a margin, at most 80 cells wide. 69 is 1725px, a full iTerm2 window.",
            "Existing cards keep the size they have.",
        ],
    ),
    (
        "cards.height",
        &["Height in 25px grid cells, or 0 to size from the window. 80 is 2000px."],
    ),
    (
        "cards.shape",
        &[
            "The shape of a card sized from the window: a ratio like \"16:9\" or \"4:3\",",
            "or \"window\" for the window's own shape. 16:9 keeps every card the same",
            "shape on every Mac and after a window resize; \"window\" makes one card fill",
            "the window exactly when fitted. Used only where width or height is 0.",
        ],
    ),
    (
        "cards.inheritDirectory",
        &[
            "Whether a card carved out of another one keeps its directory.",
            "That is a split (Cmd+D, Cmd+Shift+D) and a card opened onto what",
            "the active card is showing. A plain new card (Cmd+T, Cmd+Alt+T)",
            "always starts at startingDir: a new card is a new place to work.",
        ],
    ),
    ("canvas", &["The canvas itself: zooming and panning."]),
    (
        "canvas.zoomSensitivity",
        &["Multiplier on Cmd+scroll zooming. Below 1 is slower, above is faster."],
    ),
    (
        "canvas.momentum",
        &["Whether the canvas keeps gliding after you release a pan drag."],
    ),
    (
        "editor",
        &[
            "Editor cards. Font and colours follow the terminal; these are the extras.",
            "The selection colours apply to terminals too, so both select alike.",
        ],
    ),
    (
        "editor.highlightLine",
        &["A wash behind the line the cursor is on."],
    ),
    (
        "editor.selectionColor",
        &[
            "Behind selected text, as a CSS colour. Empty is the theme's yellow: a",
            "highlighter pen, visible on any scheme.",
        ],
    ),
    (
        "editor.selectionTextColor",
        &["Selected text itself. Empty is the theme's background, dark on the yellow."],
    ),
    (
        "editor.wrap",
        &[
            "Long lines. \"prose\" wraps .md and .txt and leaves code alone; \"always\" and",
            "\"never\" do what they say.",
        ],
    ),
    (
        "browser",
        &["Browser cards. The page stays at its own size whatever the canvas zoom is."],
    ),
    (
        "browser.zoom",
        &[
            "Page zoom a new browser card opens at; 1 is actual size, 0.3 to 3.",
            "The browser zoom commands change it per card while the page is focused.",
        ],
    ),
    (
        "browser.searchEngine",
        &[
            "Where the omnibox (Cmd+L) sends a search; %s is the encoded query.",
            "A template without %s is ignored: it would drop what you typed.",
        ],
    ),
    (
        "browser.suggestions",
        &[
            "Ask Google to complete what you type in the omnibox.",
            "OFF by default: every keystroke goes to Google while it is on.",
            "Nothing else in this app talks to the network on its own.",
        ],
    ),
    (
        "browser.engines",
        &[
            "Extra sites Tab can scope a search to, added to the nine built in",
            "(Google, GitHub, YouTube, Wikipedia, Stack Overflow, MDN, npm,",
            "crates.io, docs.rs). Each is {keyword, name, searchUrl}, where",
            "keyword is what you type and searchUrl is a template with %s.",
            "A keyword that is already built in replaces it.",
        ],
    ),
    (
        "ui",
        &["The app's own chrome: labels, the status bar, dimming."],
    ),
    (
        "ui.inactiveDim",
        &[
            "How much an unfocused card's text is dimmed, 0 to 1. Painted as a scrim over",
            "the text rather than as opacity, which would blur it.",
        ],
    ),
    (
        "ui.maximised",
        &[
            "The focused card always fills the window, like a tabbed terminal with the canvas",
            "behind it: new cards, closes, workspace switches and focus moves keep it so.",
            "Cmd+2 still shows the whole canvas; Cmd+Shift+Enter turns it off for the session.",
        ],
    ),
    (
        "ui.unfocusedDim",
        &[
            "How much the whole canvas is dimmed while another app is in front, 0 to 1.",
            "Says at a glance that keys are going elsewhere. 0 turns it off.",
        ],
    ),
    (
        "ui.cardLabelSize",
        &[
            "Screen pixels, before the Cmd+Shift+= multiplier, at 60% zoom and closer;",
            "1.1x below 60% and 1.45x below 25%, so a name still registers zoomed far out.",
            "Separate from the other sizes for that reason.",
        ],
    ),
    (
        "ui.groupLabelSize",
        &["Screen pixels. A group name sits above a block of cards."],
    ),
    (
        "ui.statusBarSize",
        &["Screen pixels. The status bar is always at arm's length."],
    ),
    (
        "ui.animations",
        &[
            "Zooming, panning and card swaps animate rather than jumping. The system",
            "\"reduce motion\" setting switches them off regardless of this.",
        ],
    ),
    (
        "ui.showFps",
        &[
            "Frame counter in the status bar, shown while frames are drawn back to back",
            "(a zoom, a pan, a drag, fast output); nothing at rest, where the app draws",
            "only when something changes. Orange below 50.",
        ],
    ),
    (
        "ui.fullscreen",
        &[
            "What the green button and Cmd+Ctrl+F do. \"cover\" makes the window cover the",
            "whole screen, the strip beside a MacBook's notch included, with the menu bar",
            "and Dock hidden until the pointer reaches them. \"native\" is macOS's own",
            "full screen: a Space of its own, content starting below the notch.",
        ],
    ),
];

fn doc(key: &str) -> &'static [&'static str] {
    SETTINGS_DOC
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(&[], |(_, lines)| lines)
}

/// Dotted keys in `config` (a settings-shaped value) with no entry in
/// `SETTINGS_DOC`.
pub fn undocumented(config: &Value) -> Vec<String> {
    let mut missing = vec![];
    fn walk(value: &Value, prefix: &str, missing: &mut Vec<String>) {
        if let Some(map) = value.as_object() {
            for (k, v) in map {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                walk(v, &key, missing);
            }
            return;
        }
        if doc(prefix).is_empty() {
            missing.push(prefix.to_string());
        }
    }
    walk(config, "", &mut missing);
    missing
}

const HEADER: &[&str] = &[
    "Every setting infiniterm has, with its default value.",
    "",
    "THIS FILE IS REWRITTEN EVERY LAUNCH. Editing it does nothing.",
    "Put your changes in settings.json beside it, which overrides what is here and",
    "is never touched by an upgrade. Copy a line across and change it: the keys",
    "are flat, so \"terminal.fontSize\": 16 is a whole override.",
    "",
    "Comments and trailing commas are allowed in both files.",
];

/// One JSON value, printed the way JSON.stringify prints it: `14`, not
/// `14.0`. The config holds f64 everywhere and a file people copy from must
/// not suggest a decimal where the reference writes a whole number.
fn scalar(value: &Value) -> String {
    match value.as_f64() {
        Some(n) => n.to_string(),
        None => value.to_string(),
    }
}

/// The settings as flat dotted keys, each under its comment, with a group's
/// own comment heading its first key and a blank line between entries so
/// the file reads as sections rather than a wall.
fn render(value: &Value) -> String {
    let Some(map) = value.as_object() else {
        return scalar(value);
    };
    let flat = flatten(map);
    let mut seen_groups: Vec<String> = vec![];
    let inner: Vec<String> = flat
        .iter()
        .enumerate()
        .map(|(i, (key, v))| {
            let mut comment = String::new();
            if let Some((group, _)) = key.rsplit_once('.') {
                if !seen_groups.iter().any(|g| g == group) {
                    seen_groups.push(group.to_string());
                    for line in doc(group) {
                        comment.push_str(&format!("  // {line}\n"));
                    }
                    if !doc(group).is_empty() {
                        comment.push_str("  //\n");
                    }
                }
            }
            for line in doc(key) {
                comment.push_str(&format!("  // {line}\n"));
            }
            let spacer = if !comment.is_empty() && i > 0 {
                "\n"
            } else {
                ""
            };
            let comma = if i < flat.len() - 1 { "," } else { "" };
            format!("{spacer}{comment}  \"{key}\": {}{comma}", scalar(v))
        })
        .collect();
    format!("{{\n{}\n}}", inner.join("\n"))
}

/// The full text of `settings.default.json`.
pub fn render_settings_default(config: &Config) -> String {
    let header: Vec<String> = HEADER
        .iter()
        .map(|line| {
            if line.is_empty() {
                "//".to_string()
            } else {
                format!("// {line}")
            }
        })
        .collect();
    let value = serde_json::to_value(config).expect("config serialises");
    format!("{}\n{}\n", header.join("\n"), render(&value))
}

/// The defaults file as shipped.
pub fn default_settings_text() -> String {
    render_settings_default(&default_config())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::merge_config;
    use crate::jsonc::parse_jsonc;
    use serde_json::json;

    fn defaults_value() -> Value {
        serde_json::to_value(default_config()).unwrap()
    }

    // The guard that keeps the defaults file honest.
    #[test]
    fn finds_nothing_every_setting_is_documented() {
        assert!(undocumented(&defaults_value()).is_empty());
    }

    #[test]
    fn reports_a_setting_with_no_comment() {
        let mut with_extra = defaults_value();
        with_extra["brandNew"] = json!(1);
        assert_eq!(undocumented(&with_extra), ["brandNew"]);
    }

    #[test]
    fn walks_into_nested_groups() {
        assert_eq!(
            undocumented(&json!({"ui": {"somethingNew": true}})),
            ["ui.somethingNew"]
        );
    }

    // It has to be parseable by the same reader the user's file goes
    // through, and come back as exactly the defaults. Through `merge_config`,
    // not serde: the file is flat and Config is not, and typed, because a
    // Value compares `14` and `14.0` as different numbers.
    #[test]
    fn round_trips_through_the_real_merge() {
        assert_eq!(
            merge_config(&parse_jsonc(&default_settings_text()).unwrap()),
            default_config()
        );
    }

    #[test]
    fn says_it_is_rewritten_and_where_to_put_your_own() {
        let text = default_settings_text();
        assert!(text.contains("REWRITTEN EVERY LAUNCH"));
        assert!(text.contains("settings.json"));
    }

    #[test]
    fn comments_every_setting_nested_ones_included() {
        let text = default_settings_text();
        assert!(text.contains("// Terminal colour scheme"));
        assert!(text.contains("// In points."));
        assert!(text.contains("// Frame counter in the status bar"));
    }

    #[test]
    fn is_rendered_from_the_defaults_so_a_value_cannot_drift_from_the_code() {
        let text = default_settings_text();
        let d = default_config();
        assert!(text.contains(&format!("\"terminal.fontSize\": {}", d.terminal.font_size)));
        assert!(text.contains(&format!("\"ui.showFps\": {}", d.ui.show_fps)));
    }

    // Native check: f64 defaults render as JSON.stringify would, so the file
    // reads `14` and `10000`, never `14.0`.
    #[test]
    fn whole_numbers_render_without_a_decimal_point() {
        let text = default_settings_text();
        assert!(text.contains("\"terminal.fontSize\": 14,"));
        assert!(text.contains("\"terminal.scrollback\": 10000,"));
        assert!(text.contains("\"terminal.sessionBuffer\": 4,\n"));
        assert!(text.contains("\"terminal.lineHeight\": 1.2,"));
    }

    // Flat: no key opens a group, and a group's doc heads its first key.
    #[test]
    fn is_flat_with_the_group_comment_over_its_first_key() {
        let text = default_settings_text();
        assert!(!text.contains("\"terminal\": {"));
        assert!(!text.contains("\"ui\": {"));
        let group = text.find("// New cards: how big").unwrap();
        let first = text.find("\n  \"cards.").unwrap();
        assert!(group < first && first - group < 400, "{group} {first}");
        // The whole thing parses as one flat object.
        let parsed = parse_jsonc(&text).unwrap();
        assert!(parsed.as_object().unwrap().keys().all(|k| !k.is_empty()));
        assert!(parsed.get("terminal.fontSize").is_some());
    }
}
