//! Resolves user-facing font settings once for every chrome renderer.
//!
//! gpui accepts `.SystemUIFont` as the platform-native app font. Keep that
//! alias intact rather than translating it to a particular Apple font name,
//! which lets the OS choose the correct face and future replacement.
use gpui::{font, Font, FontWeight};
use infiniterm_core::config::Ui;

pub const SYSTEM_UI_FONT: &str = ".SystemUIFont";

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
    use super::{ui_family_of, weight_of, UiTypography};
    use gpui::FontWeight;
    use infiniterm_core::config::default_config;

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
