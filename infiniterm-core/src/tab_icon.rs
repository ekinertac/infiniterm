//! Which icon a workspace tab wears (#544): the foreground program of the
//! workspace's focused card, turned into a Nerd Font glyph or a logo file.
//!
//! Where it fits: `render_title_bar` (infiniterm-ui/src/overlays.rs) asks
//! `representative` for the program of the workspace's cards (and `paint_header`
//! the card's own `proc`), then `choose` for what to draw; `IconFileCache`
//! answers "does this logo file exist" without a stat every frame. The theme's `tab.icons`, `tab.iconStyle` and `tab.iconMap` keys
//! (`ui_theme::TabTheme`) feed it; nothing is drawn unless `tab.icons` is on.
//! Related: `ui_theme.rs` (the keys), `program_state.rs` (the process the card
//! runs comes from the process table, `Card::proc`).
//!
//! Pure and windowless: the one thing it needs from the disk, whether a logo
//! file exists, comes in as a closure so the rules test without a folder.
use std::collections::BTreeMap;

/// The range `tab.iconSize` and `card.headerIconSize` take, in pixels.
pub const ICON_SIZE_MIN: f64 = 6.;
pub const ICON_SIZE_MAX: f64 = 64.;

/// An icon's size: the theme's, when it set one, else the built-in `default`.
pub fn size_px(set: Option<f64>, default: f64) -> f64 {
    set.unwrap_or(default)
}

/// The `tab.iconMap` entry (and the built-in one) for a program nothing else
/// names.
pub const FALLBACK_KEY: &str = "default";

/// Logo files a theme may ship, tried in this order.
pub const IMAGE_EXTENSIONS: [&str; 2] = ["svg", "png"];

/// `tab.iconStyle`: what an icon is made of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IconStyle {
    /// A Nerd Font glyph (the bundled icon font).
    #[default]
    Glyph,
    /// A logo file from the theme's folder, a glyph when there is none.
    Image,
}

impl IconStyle {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "glyph" => Some(Self::Glyph),
            "image" => Some(Self::Image),
            _ => None,
        }
    }
}

/// What a tab draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Icon {
    /// Text drawn in the icon font.
    Glyph(String),
    /// A file name inside the theme's icon folder, checked by `safe_file`.
    File(String),
}

/// Glyphs for the programs people run, as Nerd Font code points (all present
/// in `assets/fonts/SymbolsNerdFontMono-Regular.ttf`). A theme's map wins.
const BUILT_IN: &[(&str, &str)] = &[
    ("fish", "\u{f30c}"),
    ("zsh", "\u{f120}"),
    ("bash", "\u{f120}"),
    ("sh", "\u{f120}"),
    ("node", "\u{e718}"),
    ("bun", "\u{e718}"),
    ("deno", "\u{e718}"),
    ("npm", "\u{e71e}"),
    ("python", "\u{e73c}"),
    ("python3", "\u{e73c}"),
    ("git", "\u{e702}"),
    ("lazygit", "\u{e702}"),
    ("vim", "\u{e62b}"),
    ("nvim", "\u{e62b}"),
    ("cargo", "\u{e7a8}"),
    ("rustc", "\u{e7a8}"),
    ("go", "\u{e627}"),
    ("java", "\u{e738}"),
    ("ruby", "\u{e739}"),
    ("docker", "\u{e7b0}"),
    ("ssh", "\u{f023}"),
    ("claude", "\u{f0d0}"),
    ("codex", "\u{f0d0}"),
    ("opencode", "\u{f0d0}"),
    ("cursor-agent", "\u{f0d0}"),
    (FALLBACK_KEY, "\u{f120}"),
];

/// The program a workspace's tab shows: the focused card's, else the first
/// card's in reading order (a workspace you are not in has no focus of its
/// own to ask). `cards` is `(is_focused, foreground process)` in reading order;
/// `None` for a workspace with no card, or whose card has no process yet.
pub fn representative<'a>(cards: &[(bool, Option<&'a str>)]) -> Option<&'a str> {
    cards
        .iter()
        .find(|(focused, _)| *focused)
        .or_else(|| cards.first())
        .and_then(|(_, proc)| *proc)
}

/// The shell a card runs when no program is in front of it: the process table
/// reports a job or a child of the shell, so a shell at its prompt has none
/// and its tab would show the fallback icon. `configured` is `terminal.shell`
/// (empty for the login shell, `$SHELL`); only its first word names the program.
pub fn shell_name(configured: &str, login_shell: Option<&str>) -> String {
    let first = configured.split_whitespace().next();
    first.or(login_shell).map(program_key).unwrap_or_default()
}

/// A process name as the map is keyed: the last path component, lower case,
/// without the leading `-` a login shell shows (`-zsh`).
pub fn program_key(proc: &str) -> String {
    let base = proc.trim().rsplit('/').next().unwrap_or("");
    base.trim_start_matches('-').to_lowercase()
}

/// Whether `name` can be read from the theme's icon folder: one plain file
/// name with a logo extension, so a map value of `../../x.png` cannot leave it.
pub fn safe_file(name: &str) -> bool {
    let plain = !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
        && name.trim() == name;
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase());
    plain && ext.is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.as_str()))
}

/// A map value is a file when it ends in a logo extension, else a glyph.
fn from_map_value(v: &str) -> Option<Icon> {
    let ext = v.rsplit_once('.').map(|(_, e)| e.to_lowercase());
    if ext.is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.as_str())) {
        safe_file(v).then(|| Icon::File(v.to_string()))
    } else {
        (!v.is_empty()).then(|| Icon::Glyph(v.to_string()))
    }
}

/// The icon for `proc`. The theme's map is asked first (the program, then
/// `default`); in the `Image` style a file named after the program
/// (`bun.svg`, `bun.png`) is tried before the built-in glyphs; the built-in
/// table and its `default` come last, so an enabled icon always draws
/// something. A file the map or the program names but `has_file` does not find
/// is skipped, so a missing logo falls back to a glyph.
pub fn choose(
    proc: &str,
    style: IconStyle,
    map: &BTreeMap<String, String>,
    has_file: &dyn Fn(&str) -> bool,
) -> Icon {
    let key = program_key(proc);
    let usable = |icon: Icon| match &icon {
        Icon::File(f) if !has_file(f) => None,
        _ => Some(icon),
    };
    let named = (!key.is_empty())
        .then(|| map.get(&key))
        .flatten()
        .and_then(|v| from_map_value(v))
        .and_then(usable);
    if let Some(icon) = named {
        return icon;
    }
    if style == IconStyle::Image && !key.is_empty() {
        for ext in IMAGE_EXTENSIONS {
            let file = format!("{key}.{ext}");
            if safe_file(&file) && has_file(&file) {
                return Icon::File(file);
            }
        }
    }
    let built_in = |k: &str| {
        BUILT_IN
            .iter()
            .find(|(name, _)| *name == k)
            .map(|(_, g)| Icon::Glyph((*g).to_string()))
    };
    if !key.is_empty() {
        if let Some(icon) = built_in(&key) {
            return icon;
        }
    }
    if let Some(icon) = map
        .get(FALLBACK_KEY)
        .and_then(|v| from_map_value(v))
        .and_then(usable)
    {
        return icon;
    }
    if style == IconStyle::Image {
        for ext in IMAGE_EXTENSIONS {
            let file = format!("{FALLBACK_KEY}.{ext}");
            if has_file(&file) {
                return Icon::File(file);
            }
        }
    }
    built_in(FALLBACK_KEY).expect("the table has a default")
}

/// How long the answer for a theme's icon folder is trusted before the
/// folder's mtime is looked at again. A tab is drawn every frame, and a stat
/// per tab per frame is the cost this cache exists to avoid; half a second
/// is quick enough for "I just dropped a logo in".
pub const ICON_DIR_RECHECK_MS: f64 = 500.;

/// Whether the logo files a theme names exist, remembered per file name.
/// Forgotten when the folder changes (another theme), or when the folder's
/// mtime moves (a file added or removed in it), which is looked at at most
/// every `ICON_DIR_RECHECK_MS`. Pure: the clock, the mtime and the file test
/// come in, so the rules test without a folder.
#[derive(Debug, Default)]
pub struct IconFileCache {
    dir: Option<std::path::PathBuf>,
    mtime: Option<std::time::SystemTime>,
    checked_ms: f64,
    known: std::collections::HashMap<String, bool>,
}

impl IconFileCache {
    /// Whether `dir/file` exists. `mtime` is the folder's modification time
    /// and `exists` the real file test; each is called only when needed.
    pub fn has_file(
        &mut self,
        dir: &std::path::Path,
        file: &str,
        now_ms: f64,
        mtime: impl FnOnce(&std::path::Path) -> Option<std::time::SystemTime>,
        exists: impl FnOnce(&std::path::Path) -> bool,
    ) -> bool {
        if self.dir.as_deref() != Some(dir) {
            *self = Self {
                dir: Some(dir.to_path_buf()),
                mtime: mtime(dir),
                checked_ms: now_ms,
                known: Default::default(),
            };
        } else if now_ms - self.checked_ms >= ICON_DIR_RECHECK_MS {
            self.checked_ms = now_ms;
            let latest = mtime(dir);
            if latest != self.mtime {
                self.mtime = latest;
                self.known.clear();
            }
        }
        if let Some(found) = self.known.get(file) {
            return *found;
        }
        let found = exists(&dir.join(file));
        self.known.insert(file.to_string(), found);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    fn at(secs: u64) -> Option<SystemTime> {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
    }

    #[test]
    fn the_cache_asks_the_disk_once_per_file_until_the_folder_changes() {
        let mut c = IconFileCache::default();
        let dir = Path::new("/t/glass/icons");
        let mut tests = 0;
        for now in [0., 100., 200., 300., 400.] {
            let has = c.has_file(
                dir,
                "bun.svg",
                now,
                |_| at(1),
                |_| {
                    tests += 1;
                    true
                },
            );
            assert!(has);
        }
        assert_eq!(
            tests, 1,
            "a file is tested once while the folder is as it was"
        );
    }

    #[test]
    fn the_folder_is_not_looked_at_between_rechecks() {
        let mut c = IconFileCache::default();
        let dir = Path::new("/t/glass/icons");
        let mut stats = 0;
        for now in [0., 10., 20., 499.] {
            c.has_file(
                dir,
                "a.png",
                now,
                |_| {
                    stats += 1;
                    at(1)
                },
                |_| false,
            );
        }
        assert_eq!(stats, 1, "only the first call (the folder is new) stats it");
        c.has_file(
            dir,
            "a.png",
            500.,
            |_| {
                stats += 1;
                at(1)
            },
            |_| false,
        );
        assert_eq!(stats, 2);
    }

    #[test]
    fn a_new_mtime_forgets_what_was_known() {
        let mut c = IconFileCache::default();
        let dir = Path::new("/t/glass/icons");
        assert!(!c.has_file(dir, "bun.svg", 0., |_| at(1), |_| false));
        // The logo is dropped in: the folder's mtime moves.
        assert!(c.has_file(dir, "bun.svg", 600., |_| at(2), |_| true));
        // And removed again.
        assert!(!c.has_file(dir, "bun.svg", 1200., |_| at(3), |_| false));
    }

    #[test]
    fn another_theme_folder_starts_clean() {
        let mut c = IconFileCache::default();
        let a = Path::new("/t/a/icons");
        let b = Path::new("/t/b/icons");
        assert!(c.has_file(a, "x.svg", 0., |_| at(1), |_| true));
        assert!(!c.has_file(b, "x.svg", 1., |_| at(1), |_| false));
        assert!(c.has_file(a, "x.svg", 2., |_| at(1), |_| true));
    }

    #[test]
    fn a_missing_folder_is_cached_as_missing() {
        let mut c = IconFileCache::default();
        let dir = Path::new("/t/none/icons");
        assert!(!c.has_file(dir, "x.svg", 0., |_| None, |_| false));
        assert!(c.has_file(dir, "x.svg", 600., |_| at(5), |_| true));
    }

    fn none(_: &str) -> bool {
        false
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn a_process_name_is_keyed_by_its_last_component_without_a_login_dash() {
        assert_eq!(program_key("-zsh"), "zsh");
        assert_eq!(program_key("/opt/homebrew/bin/Fish"), "fish");
        assert_eq!(program_key("  bun "), "bun");
        assert_eq!(program_key(""), "");
    }

    #[test]
    fn a_logo_file_is_one_plain_name_with_a_logo_extension() {
        assert!(safe_file("bun.svg"));
        assert!(safe_file("Claude.PNG"));
        for bad in [
            "",
            "../x.png",
            "a/b.png",
            ".hidden.png",
            "x.jpg",
            "x",
            " x.png",
            "a\\b.svg",
        ] {
            assert!(!safe_file(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_built_in_table_names_a_glyph_and_an_unknown_program_gets_the_default() {
        let m = BTreeMap::new();
        assert_eq!(
            choose("-fish", IconStyle::Glyph, &m, &none),
            Icon::Glyph("\u{f30c}".into())
        );
        assert_eq!(
            choose("mystery-tool", IconStyle::Glyph, &m, &none),
            Icon::Glyph("\u{f120}".into())
        );
        assert_eq!(
            choose("", IconStyle::Glyph, &m, &none),
            Icon::Glyph("\u{f120}".into())
        );
    }

    #[test]
    fn the_map_beats_the_built_in_and_a_value_with_an_extension_is_a_file() {
        let m = map(&[("fish", "X"), ("bun", "bun-logo.svg"), ("default", "?")]);
        let has = |f: &str| f == "bun-logo.svg";
        assert_eq!(
            choose("fish", IconStyle::Glyph, &m, &has),
            Icon::Glyph("X".into())
        );
        assert_eq!(
            choose("bun", IconStyle::Image, &m, &has),
            Icon::File("bun-logo.svg".into())
        );
        assert_eq!(
            choose("zsh", IconStyle::Glyph, &m, &has),
            Icon::Glyph("\u{f120}".into())
        );
        assert_eq!(
            choose("unheard-of", IconStyle::Glyph, &m, &has),
            Icon::Glyph("?".into())
        );
    }

    #[test]
    fn the_image_style_finds_a_file_named_after_the_program_svg_before_png() {
        let m = BTreeMap::new();
        let both = |f: &str| f == "bun.svg" || f == "bun.png";
        assert_eq!(
            choose("bun", IconStyle::Image, &m, &both),
            Icon::File("bun.svg".into())
        );
        let png = |f: &str| f == "bun.png";
        assert_eq!(
            choose("bun", IconStyle::Image, &m, &png),
            Icon::File("bun.png".into())
        );
        // The glyph style never looks for a file.
        assert_eq!(
            choose("bun", IconStyle::Glyph, &m, &both),
            Icon::Glyph("\u{e718}".into())
        );
    }

    #[test]
    fn a_missing_logo_falls_back_to_a_glyph() {
        let m = map(&[("bun", "bun.svg")]);
        assert_eq!(
            choose("bun", IconStyle::Image, &m, &none),
            Icon::Glyph("\u{e718}".into())
        );
        let unsafe_map = map(&[("bun", "../bun.svg")]);
        assert_eq!(
            choose("bun", IconStyle::Image, &unsafe_map, &|_| true),
            Icon::File("bun.svg".into()),
            "an unsafe map value is ignored; the program's own file is found"
        );
    }

    #[test]
    fn a_default_logo_covers_programs_without_one() {
        let has = |f: &str| f == "default.png";
        assert_eq!(
            choose("mystery-tool", IconStyle::Image, &BTreeMap::new(), &has),
            Icon::File("default.png".into())
        );
    }

    #[test]
    fn an_idle_card_is_its_shell() {
        assert_eq!(shell_name("", Some("/bin/zsh")), "zsh");
        assert_eq!(
            shell_name("/opt/homebrew/bin/fish -l", Some("/bin/zsh")),
            "fish"
        );
        assert_eq!(shell_name("  ", Some("/bin/bash")), "bash");
        assert_eq!(shell_name("", None), "");
    }

    #[test]
    fn a_workspace_shows_its_focused_card_else_its_first() {
        let cards = [(false, Some("zsh")), (true, Some("claude")), (false, None)];
        assert_eq!(representative(&cards), Some("claude"));
        let unfocused = [(false, Some("fish")), (false, Some("bun"))];
        assert_eq!(representative(&unfocused), Some("fish"));
        assert_eq!(representative(&[(true, None)]), None);
        assert_eq!(representative(&[]), None);
    }

    #[test]
    fn a_set_size_beats_the_built_in_one() {
        assert_eq!(size_px(None, 15.), 15.);
        assert_eq!(size_px(Some(11.), 15.), 11.);
    }

    #[test]
    fn the_style_names_parse() {
        assert_eq!(IconStyle::parse("glyph"), Some(IconStyle::Glyph));
        assert_eq!(IconStyle::parse("image"), Some(IconStyle::Image));
        assert_eq!(IconStyle::parse("svg"), None);
    }
}
