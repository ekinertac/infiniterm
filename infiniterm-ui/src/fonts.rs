//! Resolves user-facing font settings once for every chrome renderer.
//!
//! gpui accepts `.SystemUIFont` as the platform-native app font. Keep that
//! alias intact rather than translating it to a particular Apple font name,
//! which lets the OS choose the correct face and future replacement.
//!
//! The terminal family is chosen against the fonts this machine HAS
//! (`terminal_family_of`), because gpui does not fall through a list: it
//! takes one name, and a name that is not installed gets the platform's
//! default face. On Linux that face is proportional, so the default
//! `ui-monospace, Menlo, monospace` drew a wide-spaced grid (the #464 spike).
//! `terminals.rs::family_of` is the caller; docs/agent/linux-port.md has
//! the spike.
use gpui::{font, Font, FontWeight, Window};
use infiniterm_core::config::Ui;
use std::sync::OnceLock;

pub const SYSTEM_UI_FONT: &str = ".SystemUIFont";

/// What a generic `monospace` or `ui-monospace` means here: the first of
/// these that is installed. Menlo ships with every Mac. On Linux there is no
/// one font every distribution ships, so this is the usual set in the order
/// fontconfig's own `monospace` alias prefers them.
#[cfg(target_os = "macos")]
const PLATFORM_MONOSPACE: &[&str] = &["Menlo", "Monaco"];
#[cfg(windows)]
const PLATFORM_MONOSPACE: &[&str] = &["Cascadia Mono", "Consolas"];
#[cfg(not(any(target_os = "macos", windows)))]
const PLATFORM_MONOSPACE: &[&str] = &[
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Noto Sans Mono",
    "Ubuntu Mono",
    "Cascadia Mono",
];

static INSTALLED: OnceLock<Vec<String>> = OnceLock::new();

/// Reads the font families gpui can draw, once, when the main window opens
/// (`main.rs`): the list is a walk over every installed face, and the
/// terminal family is asked for every frame, by callers that have no window
/// at hand. A font installed while the app runs is seen after a relaunch.
pub fn remember_installed(window: &Window) {
    INSTALLED.get_or_init(|| window.text_system().all_font_names());
}

/// The families `remember_installed` read; empty before the window opens,
/// which `terminal_family_of` answers with the list's first named family,
/// the rule from before #464.
pub fn installed_families() -> &'static [String] {
    INSTALLED.get().map(Vec::as_slice).unwrap_or(&[])
}

/// The terminal family for a CSS-style list:
/// 1. the first named family that is installed, in the list's order (the
///    generic names are skipped here, as they always were, so
///    `ui-monospace, JetBrainsMono Nerd Font` still means JetBrains);
/// 2. else, when the list says `monospace` or `ui-monospace`, the first
///    installed entry of `PLATFORM_MONOSPACE`;
/// 3. else the first named family anyway (it may be a PostScript name gpui
///    resolves although the family list does not carry it);
/// 4. else the first entry of `PLATFORM_MONOSPACE`.
///
/// Names compare without case, the way font lookups do.
pub fn terminal_family_of(list: &str, installed: &[String]) -> String {
    terminal_family_with(list, installed, PLATFORM_MONOSPACE)
}

fn terminal_family_with(list: &str, installed: &[String], platform: &[&str]) -> String {
    // The installed spelling is returned, so a lookup that does care about
    // case still finds it.
    let installed_as = |name: &str| installed.iter().find(|i| i.eq_ignore_ascii_case(name));
    let entries: Vec<String> = list
        .split(',')
        .map(|f| f.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|f| !f.is_empty())
        .collect();
    let is_generic = |f: &str| matches!(f, "ui-monospace" | "monospace" | "system-ui");
    let named = || entries.iter().filter(|f| !is_generic(f));
    if let Some(found) = named().find_map(|f| installed_as(f)) {
        return found.clone();
    }
    if entries
        .iter()
        .any(|f| f == "monospace" || f == "ui-monospace")
    {
        if let Some(found) = platform.iter().find_map(|f| installed_as(f)) {
            return found.clone();
        }
    }
    if let Some(first) = named().next() {
        return first.clone();
    }
    platform.first().copied().unwrap_or("Menlo").to_string()
}

/// The first usable family in a CSS-style list. Generic system UI maps to
/// gpui's native alias; monospace generics are skipped because gpui expects
/// a concrete family name.
pub fn ui_family_of(list: &str) -> String {
    for raw in list.split(',') {
        let family = raw.trim().trim_matches('"').trim_matches('\'').to_string();
        match family.as_str() {
            "" | "ui-monospace" | "monospace" => continue,
            "system-ui" => return SYSTEM_UI_FONT.to_string(),
            _ => return family,
        }
    }
    SYSTEM_UI_FONT.to_string()
}

/// CSS-like font weights shared by terminal content and UI chrome.
pub fn weight_of(value: &str) -> FontWeight {
    match value.trim().to_ascii_lowercase().as_str() {
        "bold" => FontWeight::BOLD,
        "normal" | "" => FontWeight::NORMAL,
        number => number
            .parse::<f32>()
            .ok()
            .filter(|weight| (100. ..=900.).contains(weight))
            .map(FontWeight)
            .unwrap_or(FontWeight::NORMAL),
    }
}

#[derive(Clone, Debug)]
pub struct UiTypography {
    pub family: String,
    pub font_size: f64,
    pub regular: Font,
    pub bold: Font,
}

impl UiTypography {
    pub fn from_config(ui: &Ui) -> Self {
        let family = ui_family_of(&ui.font_family);
        let mut regular = font(family.clone());
        regular.weight = weight_of(&ui.font_weight);
        let mut bold = font(family.clone());
        bold.weight = weight_of(&ui.font_weight_bold);
        Self {
            family,
            font_size: ui.font_size,
            regular,
            bold,
        }
    }

    /// Scale a legacy chrome size around the 13 px default so changing
    /// `ui.fontSize` preserves the established visual hierarchy.
    pub fn size(&self, default_px: f32) -> f32 {
        default_px * self.font_size as f32 / 13.
    }
}

#[cfg(test)]
mod tests {
    use super::{terminal_family_with, ui_family_of, weight_of, UiTypography};
    use gpui::FontWeight;
    use infiniterm_core::config::default_config;

    const LINUX: &[&str] = &["DejaVu Sans Mono", "Liberation Mono"];
    const DEFAULT: &str = "ui-monospace, Menlo, monospace";

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_default_list_is_menlo_on_a_mac() {
        let mac = names(&["Helvetica", "Menlo", "Monaco"]);
        assert_eq!(terminal_family_with(DEFAULT, &mac, &["Menlo"]), "Menlo");
    }

    #[test]
    fn the_default_list_on_linux_is_an_installed_monospace_font_not_menlo() {
        // What the #464 droplet had: no Menlo, so gpui drew DejaVu Sans.
        let linux = names(&["DejaVu Sans", "DejaVu Sans Mono", "DejaVu Serif"]);
        assert_eq!(
            terminal_family_with(DEFAULT, &linux, LINUX),
            "DejaVu Sans Mono"
        );
        let other = names(&["Liberation Mono", "Liberation Sans"]);
        assert_eq!(
            terminal_family_with(DEFAULT, &other, LINUX),
            "Liberation Mono"
        );
    }

    #[test]
    fn a_named_family_that_is_installed_wins_over_the_generic_names() {
        let fonts = names(&["JetBrainsMono Nerd Font", "Menlo"]);
        assert_eq!(
            terminal_family_with(
                "ui-monospace, \"JetBrainsMono Nerd Font\", monospace",
                &fonts,
                &["Menlo"]
            ),
            "JetBrainsMono Nerd Font"
        );
        // Case does not matter; the installed spelling is what gpui gets.
        assert_eq!(
            terminal_family_with("jetbrainsmono nerd font", &fonts, &["Menlo"]),
            "JetBrainsMono Nerd Font"
        );
    }

    #[test]
    fn a_missing_family_falls_through_to_the_next_one() {
        let fonts = names(&["Fira Code", "Menlo"]);
        assert_eq!(
            terminal_family_with("Iosevka, Fira Code", &fonts, &["Menlo"]),
            "Fira Code"
        );
    }

    #[test]
    fn with_nothing_installed_matching_the_first_name_is_kept_else_the_platform_font() {
        // A PostScript name the family list does not carry: still tried.
        assert_eq!(
            terminal_family_with("JetBrainsMonoNF-Regular", &[], &["Menlo"]),
            "JetBrainsMonoNF-Regular"
        );
        assert_eq!(
            terminal_family_with("monospace", &[], LINUX),
            "DejaVu Sans Mono"
        );
        assert_eq!(terminal_family_with("", &[], &[]), "Menlo");
    }

    #[test]
    fn ui_family_keeps_the_system_alias_and_reads_a_css_style_list() {
        assert_eq!(ui_family_of(".SystemUIFont"), ".SystemUIFont");
        assert_eq!(
            ui_family_of("system-ui, \"JetBrainsMono Nerd Font\", monospace"),
            ".SystemUIFont"
        );
        assert_eq!(
            ui_family_of("ui-monospace, \"JetBrainsMono Nerd Font\", monospace"),
            "JetBrainsMono Nerd Font"
        );
        assert_eq!(ui_family_of("monospace"), ".SystemUIFont");
    }

    #[test]
    fn weights_accept_names_and_css_numbers() {
        assert_eq!(weight_of("normal"), FontWeight::NORMAL);
        assert_eq!(weight_of("bold"), FontWeight::BOLD);
        assert_eq!(weight_of("450"), FontWeight(450.));
        assert_eq!(weight_of(" 750 "), FontWeight(750.));
        assert_eq!(weight_of("heavy"), FontWeight::NORMAL);
        assert_eq!(weight_of("1000"), FontWeight::NORMAL);
    }

    #[test]
    fn typography_resolves_the_default_and_custom_ui_config() {
        let mut ui = default_config().ui;
        let system = UiTypography::from_config(&ui);
        assert_eq!(system.family, ".SystemUIFont");
        assert_eq!(system.font_size, 13.);
        assert_eq!(system.regular.weight, FontWeight::NORMAL);
        assert_eq!(system.bold.weight, FontWeight::BOLD);

        ui.font_family = "JetBrainsMono Nerd Font".into();
        ui.font_size = 26.;
        ui.font_weight = "450".into();
        ui.font_weight_bold = "750".into();
        let custom = UiTypography::from_config(&ui);
        assert_eq!(custom.family, "JetBrainsMono Nerd Font");
        assert_eq!(custom.size(12.), 24.);
        assert_eq!(custom.regular.weight, FontWeight(450.));
        assert_eq!(custom.bold.weight, FontWeight(750.));
    }
}
