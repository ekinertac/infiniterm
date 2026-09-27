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

/// `~/.config/infiniterm`, or `INFINITERM_CONFIG_DIR`: the Tauri app watches
/// the real one, so a test that edits settings must edit a copy.
pub fn config_dir() -> PathBuf {
    config_dir_from(std::env::var_os("INFINITERM_CONFIG_DIR").as_deref())
}

pub fn config_dir_from(override_: Option<&std::ffi::OsStr>) -> PathBuf {
    match override_ {
        Some(dir) => PathBuf::from(dir),
        None => home_dir().join(".config").join("infiniterm"),
    }
}

/// Application Support: the save file and the drafts. `INFINITERM_DATA_DIR`
/// moves it, and the socket with it: while the Tauri app is the shipping
/// one, a native build run beside it must not save over its canvas or steal
/// its socket, and a test must never touch the real directory. (The first
/// native run did exactly that, 2026-09-15, because this override was
/// missing here; the test below is the guard.)
pub fn app_support_dir() -> PathBuf {
    data_dir(std::env::var_os("INFINITERM_DATA_DIR").as_deref())
}

/// The per-instance directory: the override when set, else Application Support.
pub fn data_dir(override_: Option<&std::ffi::OsStr>) -> PathBuf {
    match override_ {
        Some(dir) => PathBuf::from(dir),
        None => home_dir()
            .join("Library")
            .join("Application Support")
            .join(BUNDLE_ID),
    }
}

pub fn layout_path() -> PathBuf {
    app_support_dir().join("workspace.json")
}

/// Where the omnibox's history lives, beside the save file.
pub fn history_path() -> PathBuf {
    app_support_dir().join("history.json")
}

/// Every agent state change, with a timestamp. Beside the save file so it
/// can be tailed while you work.
pub fn agent_log_path() -> PathBuf {
    app_support_dir().join("agent.log")
}

pub fn drafts_dir() -> PathBuf {
    app_support_dir().join("drafts")
}

/// The reference keeps themes under Tauri's app config dir, which on macOS
/// is Application Support, and its settings doc says so. Same place here.
pub fn themes_dir() -> PathBuf {
    app_support_dir().join("themes")
}

/// CEF's profile, the Claude extension's own seeded copy, and every other
/// extension `ift install-extension` has put in (`extensions::extensions_dir`).
pub fn browser_dir() -> PathBuf {
    app_support_dir().join("browser")
}

/// Chrome's own profile directory: where an extension `ift install-
/// extension` looks up is unpacked once Chrome has installed it
/// (`extensions::chrome_extension_dir`), and where the two Anthropic
/// native messaging manifests are copied from
/// (`infiniterm-browser::process::seed`).
pub fn chrome_support_dir() -> PathBuf {
    home_dir().join("Library/Application Support/Google/Chrome")
}

/// Where each card's `iftd` socket and `.meta` file live: `<data>/s/<id>.sock`.
///
/// Under the data dir on purpose, not a sibling of it: `INFINITERM_DATA_DIR`
/// then isolates a scratch instance's sockets for free, the same property
/// tmux needed a whole `session_name()` split (a real name and a dev name)
/// to get.
///
/// Named `s`, not `sessions`: a unix socket path is capped at
/// `sizeof(sockaddr_un.sun_path)`, 104 bytes on macOS including the NUL, and
/// every byte spent here is budget taken from the 16 hex character session
/// id joined onto it (see `backend::daemon::SUN_PATH_MAX`). Created on
/// demand by whoever writes into it first (`DaemonBackend::spawn_now`), not
/// here.
pub fn sessions_dir() -> PathBuf {
    app_support_dir().join("s")
}

/// Every command run, one line each, for `ift usage` (`usage_log.rs`).
/// Local only, beside `agent.log`.
pub fn usage_log_path() -> PathBuf {
    app_support_dir().join("usage.log")
}

/// The zsh integration a card's shell loads through ZDOTDIR
/// (`shell_integration`), rewritten at every launch.
pub fn shell_integration_dir() -> PathBuf {
    app_support_dir().join("shell").join("zsh")
}

/// The unix socket `ift` and the hook binary connect to. Its existence is
/// the answer to "is infiniterm running", which is what `ift` asks first.
pub fn socket_path() -> PathBuf {
    match std::env::var_os("INFINITERM_DATA_DIR") {
        Some(dir) => PathBuf::from(dir).join("infiniterm.sock"),
        None => std::env::temp_dir().join("infiniterm.sock"),
    }
}

/// `INFINITERM_DATA_DIR` is set: this instance is a side-by-side one.
pub fn data_dir_overridden() -> bool {
    std::env::var_os("INFINITERM_DATA_DIR").is_some()
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

    // The override moves the save file and the drafts (and the socket, see
    // `socket_path`); without it, Application Support.
    #[test]
    fn the_data_dir_override_moves_the_save_file_and_the_drafts() {
        let dir = std::path::Path::new("/tmp/infiniterm-x");
        assert_eq!(data_dir(Some(dir.as_os_str())), dir);
        assert!(data_dir(None)
            .to_string_lossy()
            .contains("Application Support"));
        assert!(layout_path().starts_with(app_support_dir()));
        assert!(drafts_dir().starts_with(app_support_dir()));
    }

    #[test]
    fn the_config_dir_override_moves_the_settings() {
        let dir = std::path::Path::new("/tmp/infiniterm-cfg");
        assert_eq!(config_dir_from(Some(dir.as_os_str())), dir);
        assert!(config_dir_from(None).ends_with(".config/infiniterm"));
    }

    // The config is hand-edited and this is not; they must never collide.
    #[test]
    fn layout_path_is_not_the_config_path() {
        assert_ne!(layout_path().parent(), Some(config_dir().as_path()));
    }

    // Short on purpose: see this fn's own doc comment for the socket path
    // budget "sessions" would have spent instead.
    #[test]
    fn sessions_dir_is_under_the_data_dir_and_short() {
        assert!(sessions_dir().starts_with(app_support_dir()));
        assert_eq!(sessions_dir().file_name().unwrap(), "s");
    }
}
