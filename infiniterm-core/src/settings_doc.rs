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
use crate::ui_theme::{BAR_KEYS, LOOK_KEYS};
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
        "terminal.cursorColor",
        &[
            "The cursor's colour as a hex colour (\"#ffcc00\"). Empty follows the theme.",
            "The letter under a block cursor is drawn in black or white, whichever reads.",
        ],
    ),
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
        "keyboard.commandModifier",
        &[
            "Linux and Windows: which key plays Cmd's part, since they have none.",
            "\"ctrl\": Ctrl, with Shift where plain Ctrl is the terminal's",
            "(Ctrl+Shift+T a new card, Ctrl+Shift+C copy, Ctrl+1 fit a card);",
            "plain Ctrl+letter always reaches the terminal. \"super\" (Linux",
            "only): the Mac's chords with Super for Cmd, Ctrl left whole for the",
            "terminal, at the cost of the Super chords your desktop takes first.",
            "Ignored on macOS.",
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
            "(\"/opt/homebrew/bin/htop\"). On Windows it runs through PowerShell",
            "instead, so write it as PowerShell.",
        ],
    ),
    (
        "cards",
        &["New cards: how big they are and where they start."],
    ),
    (
        "cards.width",
        &[
            "Width in 25px grid squares of canvas (not text columns), or 0 to size new cards from",
            "the window: the canvas at 100% minus a margin, at most 80 squares wide. 69 is 1725px,",
            "a full iTerm2 window. At terminal.fontSize 19 a square is about 2.6 text columns,",
            "so 50 is about 130 columns. Existing cards keep the size they have.",
        ],
    ),
    (
        "cards.height",
        &[
            "Height in 25px grid squares of canvas (not text rows), or 0 to size from the window.",
            "80 is 2000px; at terminal.fontSize 19 and lineHeight 1.2 that is about 87 text rows.",
        ],
    ),
    (
        "cards.gap",
        &[
            "The space between cards, in pixels (0 to 200, 25 by default). New cards, splits,",
            "growing a card, Canvas: tidy and group frames all use it. Cards already on the",
            "canvas stay where they are; a different gap shows as cards are added or tidied.",
        ],
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
        "editor.completeWords",
        &[
            "Offer words from the file and the card's other tabs while you type in",
            "an editor (two letters open the list, Tab takes the top one, Enter only",
            "after you moved the row). Inside comments and strings of a file with a",
            "grammar, words are not collected. settings.json and files that name a",
            "JSON schema complete their own keys regardless.",
        ],
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
        "editor.cursorColor",
        &["The caret's colour as a hex colour. Empty follows the theme."],
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
        "join.enabled",
        &[
            "Open the socket that `ift join <host>` on another Mac reaches through ssh,",
            "so it can show and drive this canvas. The socket is owner-only, like the one",
            "`ift send` uses, and ssh is the only way in. Read at launch.",
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
        "browser.devtools",
        &[
            "Let a browser card open DevTools (the palette's Browser: open DevTools).",
            "ON by default. It opens a debugging port on a random loopback port;",
            "only our own DevTools page may connect to it, a web page cannot.",
            "A program on this Mac can still attach to the port: turn it off if",
            "that matters to you. Read at launch: restart after changing it.",
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
        &[
            "The app's own chrome: labels, the status bar, the grid. How it LOOKS (fonts,",
            "window, background, card corners) is not set here but in a ui theme file,",
            "ui-themes/<name>.json beside this one; ui.theme picks it.",
        ],
    ),
    (
        "ui.theme",
        &[
            "Which ui theme gives the look: the file name in the ui-themes folder, without",
            ".json. \"default\" is ui-themes/default.json (the built-in look if the file is",
            "gone). A name with no file keeps the look you had and shows a notice.",
            "\"UI theme: switch…\" in the palette previews them and writes it here.",
        ],
    ),
    (
        "ui.hideLabelWhenFitted",
        &[
            "No label on the card Cmd+1 fitted to the window, like ui.hideLabelWhenMaximised",
            "does for a maximised one, nor on the only card of a workspace. A ui theme key.",
        ],
    ),
    (
        "ui.fontFamily",
        &[
            "Font family for the app's own chrome. .SystemUIFont, the default, is the",
            "platform's native app UI font. A comma-separated list uses the first real name.",
        ],
    ),
    (
        "ui.fontSize",
        &[
            "Base chrome size in screen pixels, before the Cmd+Shift+= multiplier.",
            "Workspace tabs, card tab strips, palettes and dialogs derive their sizes from it.",
        ],
    ),
    (
        "ui.fontWeight",
        &["Normal chrome weight: \"normal\", \"bold\", or a number from 100 to 900."],
    ),
    (
        "ui.fontWeightBold",
        &["Emphasised chrome weight: \"normal\", \"bold\", or a number from 100 to 900."],
    ),
    (
        "ui.inactiveDim",
        &[
            "How much an unfocused card's text is dimmed, 0 to 1. Painted as a scrim over",
            "the text rather than as opacity, which would blur it.",
        ],
    ),
    (
        "ui.windowOpacity",
        &[
            "How opaque the window's own fill is, 0.1 to 1: the canvas background, title bar",
            "and status bar. Below 1 the desktop shows through; cards stay solid.",
        ],
    ),
    (
        "ui.windowBlur",
        &["Blur what is behind the window. Only does anything while ui.windowOpacity is below 1."],
    ),
    (
        "ui.backgroundImage",
        &[
            "A picture behind the canvas: the name of one that comes with the app (\"dusk\",",
            "\"aurora\", \"ember\") or a path to your own, \"~\" allowed. Empty for none. A list",
            "of them rotates: [\"dusk\", \"~/Pictures/lake.jpg\"]. It is drawn opaque, so",
            "ui.windowOpacity does not show the desktop through it. Only the picture on screen",
            "and the next one are kept in memory, a 4K one is about 65 MB, so a long list costs",
            "no more than a short one.",
        ],
    ),
    (
        "ui.backgroundImageInterval",
        &["Seconds each picture of a ui.backgroundImage list stays before the next fades in. 5 to 86400."],
    ),
    (
        "ui.backgroundImageFade",
        &[
            "Seconds the crossfade between two pictures takes; 0 changes at once. A fade is an",
            "animation, so it is a cut while ui.animations is off or the system reduces motion.",
        ],
    ),
    (
        "ui.backgroundImageFit",
        &["\"cover\" fills the window and crops the edges; \"contain\" shows the whole picture."],
    ),
    (
        "ui.workspaceIsolation",
        &[
            "Ctrl+Tab switches between the current workspace's cards only, and the status bar",
            "counts this workspace's cards, not every workspace's. The palette still lists",
            "every card, so it stays the way to jump to another workspace.",
        ],
    ),
    (
        "ui.windowColor",
        &[
            "A colour for the title bar and the Dock icon: a name (red, orange, amber, yellow,",
            "lime, green, emerald, teal, cyan, sky, blue, indigo, violet, purple, pink, rose,",
            "gray) or a hex code like 3b82f6. Empty for none. The palette's \"Window: change the",
            "title bar and Dock colour\" sets it. A window opened by ift connect wears its host's.",
        ],
    ),
    (
        "ui.cardRadius",
        &[
            "Round the corners of cards and group frames, in pixels at 100% zoom (0 to 40).",
            "0, the default, keeps them square. The radius scales with the zoom like the card.",
        ],
    ),
    (
        "ui.cardOpacity",
        &[
            "How opaque a card's background is, 0.1 to 1. Text stays solid, so it never gets",
            "blurry; below 1 the canvas, the grid and ui.backgroundImage show through. Cells a",
            "program colours itself (vim's status line) and browser cards stay opaque.",
        ],
    ),
    (
        "ui.glyphBudget",
        &[
            "How many terminal cells (text columns times rows, not cards.width squares) may be",
            "on screen, across every visible card, before the canvas draws all of them as bars",
            "instead of text. Default 150000, about eleven full cards. A card of cards.width 50",
            "by cards.height 80 holds about 11000 at terminal.fontSize 19, so twelve need about",
            "130000. A window covered edge to edge in cards reads down to about 82% zoom, and the",
            "limit grows with the window. Each cell costs at most 50 nanoseconds a frame, so",
            "150000 is under 8 ms while the canvas moves; raise it until zooming gets slow.",
            "1000 to 5000000.",
        ],
    ),
    (
        "ui.textAsBars",
        &[
            "Off (the default): every card draws its text at every zoom, however small and however",
            "many cards are on screen. On: a card draws faint bars instead of text when its font is",
            "smaller than ui.minTextPx or more than ui.glyphBudget cells are on screen, which keeps",
            "the canvas fast on a slow Mac. Measured on an M4 Mac mini, 18 cards printing at once while",
            "zooming: about 45 frames a second with text, and no faster with bars.",
        ],
    ),
    (
        "ui.minTextPx",
        &[
            "How many device pixels tall a font must be on screen before its text is drawn; below",
            "that every card draws bars instead. Default 7: with terminal.fontSize 19 that is 37%",
            "zoom on a 1x screen, so a canvas of 20 cards zoomed out to 34% is bars whatever",
            "ui.glyphBudget says. 4 draws the text down to about 21% (tiny, but you see its shape).",
            "A retina screen keeps a floor of 4.5 points unless this is lower. 2 to 20.",
        ],
    ),
    (
        "ui.cardZoom",
        &[
            "The canvas scale Cmd+1 zooms to on a terminal card that is already fitted: 1.5 is 150%,",
            "with the card's bottom at the window's bottom, so you read the prompt and the newest",
            "lines larger. Cmd+Up and Cmd+Down pan along the card, Cmd+1 or Cmd+2 leaves, and typing",
            "returns the view to the bottom. 0 turns it off: a second Cmd+1 does nothing new.",
            "1.1 to 4. It was called ui.readZoom, which still works.",
        ],
    ),
    (
        "ui.showGrid",
        &["Draw the canvas grid. Off leaves the canvas background plain."],
    ),
    (
        "ui.fitPadding",
        &[
            "Screen pixels left around a card or cluster when it is fitted (Cmd+1, Cmd+2,",
            "Cmd+3). 0 lets a card touch the window's edges.",
        ],
    ),
    (
        "ui.fitMagnify",
        &[
            "Let a fit zoom past 100% so a card fills the window. Off: a fit never magnifies,",
            "and a card smaller than the window sits in open canvas.",
        ],
    ),
    (
        "ui.fitSplitSlot",
        &[
            "Cmd+1 on a card that was split frames the whole slot, both halves together.",
            "Off: it fits the card itself and fills the window with it.",
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
        "ui.cardLabelPosition",
        &[
            "The corner of a card its label sits in: \"top right\", \"top left\", \"bottom left\"",
            "or \"bottom right\". An ssh chip moves to the other side of the same edge.",
        ],
    ),
    (
        "ui.hideLabelWhenMaximised",
        &["No label on a maximised card: it fills the window and the status bar names it."],
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
        "ui.panStep",
        &[
            "How far one press of Cmd+Ctrl+Shift+Arrow moves the canvas, in screen",
            "pixels (20 to 1000). A held key repeats, so a smaller step is a slower pan.",
        ],
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
        "ui.showWhatsNew",
        &[
            "Show the changelog in an overlay the first time you open a new version",
            "after an update. Escape or the X in its corner closes it. \"Help: show",
            "what's new\" opens it any time.",
        ],
    ),
    (
        "ui.statusBarPwd",
        &[
            "Show the focused terminal card's directory in the status bar at all times.",
            "The card label shows the running program instead of the directory while one",
            "runs; this keeps the path visible then too. Home is written as ~.",
        ],
    ),
    (
        "ui.statusBarPwdSide",
        &[
            "Which end of the status bar the directory goes on: \"left\" (after the card",
            "label) or \"right\". Only used when ui.statusBarPwd is on.",
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

pub(crate) fn doc(key: &str) -> &'static [&'static str] {
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
    "Docs: https://infiniterm.app/reference/settings/",
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
pub(crate) fn scalar(value: &Value) -> String {
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
    let mut value = serde_json::to_value(config).expect("config serialises");
    // The look keys live in a ui theme (#511); listing them here would invite
    // copying them into a file that no longer reads them.
    if let Some(ui) = value.get_mut("ui").and_then(Value::as_object_mut) {
        for (key, _) in LOOK_KEYS {
            ui.remove(key.strip_prefix("ui.").unwrap_or(key));
        }
    }
    format!("{}\n{}\n", header.join("\n"), render(&value))
}

/// The text a new `ui-themes/default.json` starts with, and the ui theme
/// reference: what a ui theme is, then every key with its docs and its default
/// as a commented example. Delete the `//` of a line to use it. A test
/// uncomments every example and parses the result, so they cannot rot.
pub fn default_ui_theme_text() -> String {
    let ui = serde_json::to_value(default_config().ui).expect("ui serialises");
    let mut out = String::new();
    for line in UI_THEME_HEADER {
        out.push_str(&if line.is_empty() {
            "//\n".to_string()
        } else {
            format!("// {line}\n")
        });
    }
    out.push_str("{\n");
    let mut entry = |key: &str, lines: &[&str], default: String| {
        out.push('\n');
        for line in lines {
            out.push_str(&format!("  // {line}\n"));
        }
        out.push_str(&format!("  // \"{key}\": {default},\n"));
    };
    for (key, _) in LOOK_KEYS {
        let short = key.strip_prefix("ui.").unwrap_or(key);
        entry(key, doc(key), scalar(&ui[short]));
    }
    for (key, lines, default) in BAR_KEYS {
        entry(key, lines, default.to_string());
    }
    out.push_str("}\n");
    out
}

const UI_THEME_HEADER: &[&str] = &[
    "A ui theme: how infiniterm looks. The file name is the theme's name; settings.json",
    "\"ui.theme\" picks one, and \"UI theme: switch…\" in the palette previews them.",
    "Copy this file to make another look: ui-themes/glass.json. Only the keys you",
    "uncomment change anything; the rest keep the built-in look.",
    "",
    "The terminal colours are not here: that is \"theme\" in settings.json.",
    "Behaviour (pan step, animations, label sizes) stays in settings.json too.",
    "",
    "Comments and trailing commas are allowed. A file with a mistake is not applied",
    "and the look you had stays; a notice says why. Edits apply as you save.",
];

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

    // #511: settings.json no longer reads these, so the reference for it must
    // not offer them; they are the ui theme's.
    #[test]
    fn the_defaults_file_lists_no_look_key_but_names_the_ui_theme() {
        let text = default_settings_text();
        let parsed = parse_jsonc(&text).unwrap();
        for (key, _) in LOOK_KEYS {
            assert!(parsed.get(key).is_none(), "{key}");
        }
        assert_eq!(parsed["ui.theme"], json!("default"));
        assert!(parsed.get("ui.panStep").is_some());
    }

    #[test]
    fn the_ui_theme_reference_documents_every_key_and_its_examples_parse() {
        let text = default_ui_theme_text();
        assert!(crate::ui_theme::parse(&text).unwrap() == Default::default());
        let body: Vec<&str> = text
            .lines()
            .filter_map(|l| l.strip_prefix("  // \""))
            // A doc line may start with a quote too (`"cover" fills ...`);
            // an example is `"key": value,`.
            .filter(|l| l.split_once("\": ").is_some_and(|(k, _)| !k.contains(' ')))
            .collect();
        assert_eq!(body.len(), LOOK_KEYS.len() + BAR_KEYS.len());
        let all = format!(
            "{{\n{}\n}}",
            body.iter()
                .map(|l| format!("  \"{l}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let theme = crate::ui_theme::parse(&all).unwrap();
        assert_eq!(theme.look.len(), LOOK_KEYS.len());
        assert!(theme.titlebar.background.is_some());
        // The defaults it shows are the defaults in force.
        let ui = merge_config(&serde_json::Value::Object(theme.look)).ui;
        let mut again = default_config().ui;
        again.take_look_from(&ui);
        assert_eq!(again, default_config().ui);
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
