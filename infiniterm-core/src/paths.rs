//! Where the app keeps its files. One place, so the config directory, the
//! save file, the drafts, the themes and the socket cannot drift apart, and
//! so the per-OS answers have one seam to move behind when Linux and
//! Windows arrive (the mapping's "cross-platform" note).
//!
//! Two directories on purpose, from the Tauri app's config.rs and layout.rs:
//! `~/.config/infiniterm/` is hand-edited (settings, keybindings, their
//! generated `.default` siblings, themes) and sits where a developer expects
//! to find configuration; `~/Library/Application Support/<bundle id>/` holds
//! what the app writes several times a minute (`workspace.json`, drafts)
//! and nobody should hand-edit. Keeping them apart means a corrupt layout
//! can be deleted without taking the settings with it. The bundle id is the
//! Tauri app's, so a native build opens the existing canvas.
use std::path::PathBuf;

pub const BUNDLE_ID: &str = "dev.ekinertac.infiniterm";

/// The shell's own idea of home, so a new card starts somewhere sensible.
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_dir() -> PathBuf {
    home_dir().join(".config").join("infiniterm")
}

/// Application Support: the save file and the drafts.
pub fn app_support_dir() -> PathBuf {
    home_dir()
        .join("Library")
        .join("Application Support")
        .join(BUNDLE_ID)
}

pub fn layout_path() -> PathBuf {
    app_support_dir().join("workspace.json")
}

pub fn drafts_dir() -> PathBuf {
    app_support_dir().join("drafts")
}

/// The reference keeps themes under Tauri's app config dir, which on macOS
/// is Application Support, and its settings doc says so. Same place here.
pub fn themes_dir() -> PathBuf {
    app_support_dir().join("themes")
}

/// The unix socket `ift` and the hook binary connect to. Its existence is
/// the answer to "is infiniterm running", which is what `ift` asks first.
pub fn socket_path() -> PathBuf {
    std::env::temp_dir().join("infiniterm.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_path_is_under_application_support() {
        let path = layout_path();
        assert!(
            path.ends_with("dev.ekinertac.infiniterm/workspace.json"),
            "{path:?}"
        );
        assert!(path.to_string_lossy().contains("Application Support"));
    }

    // The config is hand-edited and this is not; they must never collide.
    #[test]
    fn layout_path_is_not_the_config_path() {
        assert_ne!(layout_path().parent(), Some(config_dir().as_path()));
    }
}
