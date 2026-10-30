//! Which picture the Dock shows (`ui.icon`, a look key of a ui theme).
//!
//! Where it fits: `config::Ui::icon` holds the choice; `Model::sync_app_icon`
//! pushes `Effect::AppIcon` when the theme in force changes it; the ui's
//! `instance_icon::set_base` draws the picture, with the window-colour badge
//! over it. This file is the pure part: the two names, and which picture and
//! where it comes from (`source`), so the fallback is tested without a bundle.
//!
//! Constraint: the Finder, Launchpad and the Cmd+Tab list read the bundle's
//! own `AppIcon.icns` (the `default` picture), which a running app cannot
//! change. Only the Dock tile and the app switcher image follow `ui.icon`.
//! The same two pictures are compiled into the binary (`include_bytes!` in
//! `infiniterm-ui`), so a scratch or dev bundle without `Resources/icons/`
//! still shows the right one.
//!
//! Related: `tools/bundle.sh` (copies `assets/icons/*.png`),
//! `tools/make-icns.sh`, `ui_theme.rs` (`LOOK_KEYS`), `settings_doc.rs`.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The values `ui.icon` accepts, in the order the docs list them.
pub const NAMES: [&str; 2] = ["default", "filled"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppIcon {
    /// The outline picture: the one the bundle's `AppIcon.icns` holds.
    #[default]
    Default,
    /// The same four cards, filled.
    Filled,
}

impl AppIcon {
    /// The name as written in a ui theme. Any other text is `None`: the theme
    /// parser rejects it (`ui_theme::Kind::Choice`), and a value that still
    /// arrives here (an old file, a host's settings) reads as the default.
    pub fn parse(text: &str) -> Option<AppIcon> {
        match text {
            "default" => Some(AppIcon::Default),
            "filled" => Some(AppIcon::Filled),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AppIcon::Default => NAMES[0],
            AppIcon::Filled => NAMES[1],
        }
    }
}

/// Where the Dock picture comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconSource {
    /// No override: the bundle's own icon shows.
    Bundle,
    /// The picture file in the bundle's `Resources/icons/`.
    File(PathBuf),
    /// The copy compiled into the binary, because the file is not there.
    Compiled(AppIcon),
}

/// The source for `icon`. `icons_dir` is `Resources/icons` (None when the
/// app cannot tell where it runs); `exists` is `Path::is_file` in the app.
pub fn source(
    icon: AppIcon,
    icons_dir: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
) -> IconSource {
    match icon {
        AppIcon::Default => IconSource::Bundle,
        AppIcon::Filled => match icons_dir.map(|d| d.join("filled.png")) {
            Some(file) if exists(&file) => IconSource::File(file),
            _ => IconSource::Compiled(icon),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_names_parse_and_anything_else_does_not() {
        assert_eq!(AppIcon::parse("default"), Some(AppIcon::Default));
        assert_eq!(AppIcon::parse("filled"), Some(AppIcon::Filled));
        for bad in ["", "Filled", "outline", " filled"] {
            assert_eq!(AppIcon::parse(bad), None, "{bad:?}");
        }
        for n in NAMES {
            assert_eq!(AppIcon::parse(n).unwrap().name(), n);
        }
    }

    #[test]
    fn default_clears_the_override() {
        let dir = Path::new("/r/icons");
        assert_eq!(
            source(AppIcon::Default, Some(dir), |_| true),
            IconSource::Bundle
        );
        assert_eq!(
            source(AppIcon::Default, None, |_| false),
            IconSource::Bundle
        );
    }

    #[test]
    fn filled_reads_the_bundle_file() {
        let dir = Path::new("/r/icons");
        assert_eq!(
            source(AppIcon::Filled, Some(dir), |p| p
                == Path::new("/r/icons/filled.png")),
            IconSource::File(PathBuf::from("/r/icons/filled.png"))
        );
    }

    #[test]
    fn filled_falls_back_to_the_compiled_picture() {
        let dir = Path::new("/r/icons");
        // The folder exists but the file does not, or there is no folder.
        assert_eq!(
            source(AppIcon::Filled, Some(dir), |_| false),
            IconSource::Compiled(AppIcon::Filled)
        );
        assert_eq!(
            source(AppIcon::Filled, None, |_| true),
            IconSource::Compiled(AppIcon::Filled)
        );
    }
}
