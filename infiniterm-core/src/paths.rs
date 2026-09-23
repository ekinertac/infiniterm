//! Where the app keeps its files. One place, so the config directory, the
//! save file, the drafts, the themes and the socket cannot drift apart, and
//! so the per-OS answers have one seam to move behind. Windows moved behind
//! it on 2026-09-23: `%APPDATA%\infiniterm` for what the app writes,
//! `%APPDATA%\infiniterm\config` for what a person edits, and a named pipe
//! rather than a socket file (see `transport.rs`).
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
/// `HOME` first on both platforms: a Windows Git Bash or MSYS shell sets it
/// and means it, and `USERPROFILE` is the native answer behind it.
pub fn home_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home);
    }
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\"))
    }
    #[cfg(unix)]
    {
        PathBuf::from("/")
    }
}

/// `%APPDATA%` (roaming), the Windows home for per-user application data.
/// Falls back under the profile directory when the variable is missing,
/// which only happens in a stripped service environment.
#[cfg(windows)]
fn appdata_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join("AppData").join("Roaming"))
}

/// `~/.config/infiniterm`, or `INFINITERM_CONFIG_DIR`: the Tauri app watches
/// the real one, so a test that edits settings must edit a copy.
pub fn config_dir() -> PathBuf {
    config_dir_from(std::env::var_os("INFINITERM_CONFIG_DIR").as_deref())
}

pub fn config_dir_from(override_: Option<&std::ffi::OsStr>) -> PathBuf {
    if let Some(dir) = override_ {
        return PathBuf::from(dir);
    }
    // Windows has no `~/.config` convention and a dotted directory in the
    // profile root is invisible to Explorer; the settings live beside the
    // save file instead, in their own `config` folder so the two stay
    // separable (see this file's header for why they are separate at all).
    #[cfg(windows)]
    {
        appdata_dir().join("infiniterm").join("config")
    }
    #[cfg(unix)]
    {
        home_dir().join(".config").join("infiniterm")
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
    if let Some(dir) = override_ {
        return PathBuf::from(dir);
    }
    // `infiniterm`, not the bundle id: the bundle id is a macOS idea and
    // the Mac path uses it so a native build opens the Tauri app's existing
    // canvas. Windows has no canvas to inherit.
    #[cfg(windows)]
    {
        appdata_dir().join("infiniterm")
    }
    #[cfg(unix)]
    {
        home_dir()
            .join("Library")
            .join("Application Support")
            .join(BUNDLE_ID)
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
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join("AppData").join("Local"))
            .join("Google")
            .join("Chrome")
            .join("User Data")
    }
    #[cfg(unix)]
    {
        home_dir().join("Library/Application Support/Google/Chrome")
    }
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

/// The endpoint `ift` and the hook binary connect to (`transport.rs`). Its
/// existence is the answer to "is infiniterm running", which is what `ift`
/// asks first.
///
/// `infiniterm-cli` and `infiniterm-hook` each carry their own copy of this
/// answer — the hook binary has no dependencies on purpose — so a change
/// here is a change in three files.
pub fn socket_path() -> PathBuf {
    socket_path_for(std::env::var_os("INFINITERM_DATA_DIR").as_deref())
}

/// A named pipe has no directory to live in, so `INFINITERM_DATA_DIR`
/// isolates a scratch instance by going into the pipe's NAME instead. A
/// hash rather than the path itself: a pipe name may not contain a
/// backslash past the prefix, and every data dir does.
pub fn socket_path_for(data_dir_override: Option<&std::ffi::OsStr>) -> PathBuf {
    #[cfg(windows)]
    {
        match data_dir_override {
            Some(dir) => PathBuf::from(format!(
                r"\\.\pipe\infiniterm-{:016x}",
                path_hash(&dir.to_string_lossy())
            )),
            None => PathBuf::from(r"\\.\pipe\infiniterm"),
        }
    }
    #[cfg(unix)]
    {
        match data_dir_override {
            Some(dir) => PathBuf::from(dir).join("infiniterm.sock"),
            None => std::env::temp_dir().join("infiniterm.sock"),
        }
    }
}

/// FNV-1a. Not a security boundary: this only has to keep two data dirs on
/// one machine from colliding, and pulling in a hash crate for that would
/// buy nothing. Kept out of cfg so its test runs on every platform.
pub fn path_hash(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// `INFINITERM_DATA_DIR` is set: this instance is a side-by-side one.
pub fn data_dir_overridden() -> bool {
    std::env::var_os("INFINITERM_DATA_DIR").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The save file sits in the per-OS place for what an app writes for
    // itself: Application Support on macOS, %APPDATA% on Windows.
    #[test]
    fn layout_path_is_under_the_data_dir() {
        let path = layout_path();
        assert!(path.starts_with(app_support_dir()), "{path:?}");
        assert_eq!(path.file_name().unwrap(), "workspace.json");
        #[cfg(unix)]
        assert!(path.to_string_lossy().contains("Application Support"));
        #[cfg(windows)]
        assert!(path.to_string_lossy().contains("infiniterm"));
    }

    // The override moves the save file and the drafts (and the socket, see
    // `socket_path`); without it, Application Support.
    #[test]
    fn the_data_dir_override_moves_the_save_file_and_the_drafts() {
        let dir = std::env::temp_dir().join("infiniterm-x");
        assert_eq!(data_dir(Some(dir.as_os_str())), dir);
        assert_ne!(data_dir(None), dir);
        assert!(layout_path().starts_with(app_support_dir()));
        assert!(drafts_dir().starts_with(app_support_dir()));
    }

    #[test]
    fn the_config_dir_override_moves_the_settings() {
        let dir = std::env::temp_dir().join("infiniterm-cfg");
        assert_eq!(config_dir_from(Some(dir.as_os_str())), dir);
        #[cfg(unix)]
        assert!(config_dir_from(None).ends_with(".config/infiniterm"));
        #[cfg(windows)]
        assert!(config_dir_from(None).ends_with("infiniterm/config"));
    }

    // On Windows the endpoint is a pipe NAME, so the data dir override has
    // nowhere to put a file and goes into the name instead; the two
    // instances must not land on the same pipe.
    #[test]
    fn the_data_dir_override_moves_the_endpoint() {
        let a = std::env::temp_dir().join("infiniterm-a");
        let b = std::env::temp_dir().join("infiniterm-b");
        assert_ne!(socket_path_for(Some(a.as_os_str())), socket_path_for(None));
        assert_ne!(
            socket_path_for(Some(a.as_os_str())),
            socket_path_for(Some(b.as_os_str()))
        );
        // Same dir, same endpoint: `ift` and the app have to agree.
        assert_eq!(
            socket_path_for(Some(a.as_os_str())),
            socket_path_for(Some(a.as_os_str()))
        );
        #[cfg(windows)]
        {
            let p = socket_path_for(Some(a.as_os_str()));
            let text = p.to_string_lossy();
            assert!(text.starts_with(r"\\.\pipe\infiniterm-"), "{text}");
            // A backslash past the prefix would make the name unusable.
            assert!(!text[r"\\.\pipe\".len()..].contains('\\'), "{text}");
        }
    }

    #[test]
    fn the_path_hash_separates_neighbouring_dirs() {
        assert_ne!(path_hash("/tmp/a"), path_hash("/tmp/b"));
        assert_eq!(path_hash("/tmp/a"), path_hash("/tmp/a"));
        assert_ne!(path_hash(""), path_hash("x"));
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
