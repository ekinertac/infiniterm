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
/// the real one, so a test that edits settings must edit a copy. An instance
/// with its own data directory (`INFINITERM_DATA_DIR`: a scratch run, a remote
/// host) starts CLEAN: its settings live in `<data>/config` unless
/// `INFINITERM_CONFIG_DIR` says otherwise (Ekin, 2026-10-05: a new instance
/// must not read the personal files). `make run` names the real one on purpose.
pub fn config_dir() -> PathBuf {
    config_dir_for(
        std::env::var_os("INFINITERM_CONFIG_DIR").as_deref(),
        std::env::var_os("INFINITERM_DATA_DIR").as_deref(),
    )
}

/// `config_dir` over explicit values: the config override wins, then a data
/// override's own `config` folder, then `~/.config/infiniterm`.
pub fn config_dir_for(config: Option<&std::ffi::OsStr>, data: Option<&std::ffi::OsStr>) -> PathBuf {
    match (config, data) {
        (Some(dir), _) => PathBuf::from(dir),
        (None, Some(data)) => PathBuf::from(data).join("config"),
        (None, None) => config_dir_from(None),
    }
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
    #[cfg(not(windows))]
    {
        default_data_dir(
            &home_dir(),
            std::env::var_os("XDG_DATA_HOME").as_deref(),
            cfg!(target_os = "linux"),
        )
    }
}

/// Where the data lives with no override: Application Support on a Mac, and
/// on Linux (a server running `iftd`, #118) `$XDG_DATA_HOME` or
/// `~/.local/share`, where a Linux user looks. A relative `XDG_DATA_HOME` is
/// ignored, as the XDG spec says. A pure function so both answers are tested
/// on either OS. Windows does not come through here (`data_dir` answers
/// with `%APPDATA%` itself).
pub fn default_data_dir(
    home: &std::path::Path,
    xdg_data_home: Option<&std::ffi::OsStr>,
    linux: bool,
) -> PathBuf {
    if linux {
        xdg_data_home
            .map(PathBuf::from)
            // `has_root`, not `is_absolute`: the rule is "not relative", and
            // on Windows `/data` has no drive letter, so it is not absolute
            // there although it is plainly not a relative path either.
            .filter(|p| p.has_root())
            .unwrap_or_else(|| home.join(".local").join("share"))
            .join(BUNDLE_ID)
    } else {
        home.join("Library")
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

/// The socket `ift join` viewers reach through ssh (`backend::join`). Short
/// on purpose: the data dir already takes about 70 of the 103 bytes a unix
/// socket path may have.
pub fn join_socket() -> PathBuf {
    app_support_dir().join("j.sock")
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

/// The last segment of a path written as TEXT, with either separator.
///
/// Not `Path::file_name`: these strings arrive from a save file, a process
/// table or a line of terminal output, and a Windows path can reach a Mac
/// (a canvas copied across, an ssh session, a log) or the other way. `Path`
/// on unix does not treat a backslash as a separator at all, so a card
/// showing a Windows path would wear the whole thing as its name.
///
/// Returns the input itself when there is no separator in it.
pub fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The same split, keeping both halves: `("a/b", "c")` for `a/b/c`. `None`
/// when there is no separator.
pub fn split_parent(path: &str) -> Option<(&str, &str)> {
    path.rsplit_once(['/', '\\'])
}

/// The directory a path sits in, as text, for a card's cwd.
///
/// Three callers reached this three slightly different ways before it
/// existed, all of them splitting on `/` alone. `"/"` for a path with no
/// parent is kept as it was: a shell on either platform understands it,
/// and every caller here is handed an absolute path anyway.
pub fn parent_dir(path: &str) -> String {
    match split_parent(path) {
        // `C:\a` splits to ("C:", "a"), and a bare `C:` does not name the
        // drive's root: it means "wherever that drive's cursor is", which
        // is a different directory from one shell to the next.
        Some((dir, _)) if dir.ends_with(':') => format!("{dir}\\"),
        Some((dir, _)) if !dir.is_empty() => dir.to_string(),
        // `/a`: the parent is the root itself.
        _ => "/".to_string(),
    }
}

/// Whether a path names a place on its own, rather than one relative to
/// some directory.
///
/// Not `Path::is_absolute`, which answers for THIS platform only: on
/// Windows it calls `/Users/ekin` relative, and a canvas saved on a Mac is
/// opened here with every path spelled that way. Both roots count on both
/// platforms, because a path that arrives is a path somebody already
/// resolved and its spelling says where it came from, not where it is being
/// read.
pub fn is_rooted(path: &str) -> bool {
    // `C:\x` and `C:/x`: a drive letter, a colon and a separator. Spelled
    // out because `Path::is_absolute` only knows it on Windows, and this
    // function's point is to answer the same on both.
    let b = path.as_bytes();
    let drive =
        b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'/' | b'\\');
    drive
        || path.starts_with('/')
        || path.starts_with('\\')
        || std::path::Path::new(path).is_absolute()
}

/// `std::fs::canonicalize`, with the answer in a form a shell, a card label
/// and a person can all read.
///
/// On Windows canonicalize returns the VERBATIM form, `\\?\C:\...`, which
/// tells the OS to skip path parsing entirely. That is right for an API call
/// and wrong for every use this app has: a card's cwd, a command line handed
/// to a shell, a label somebody reads. `ift <dir>` opened a card whose
/// directory read `\\?\C:\Users\PC\Code\infiniterm` until this existed,
/// and the hook command written into settings.json had the same problem.
///
/// One function rather than a rule to remember at each call site, because
/// the rule was forgotten twice on the first day.
pub fn canonical(path: impl AsRef<std::path::Path>) -> std::io::Result<PathBuf> {
    let full = std::fs::canonicalize(path)?;
    Ok(PathBuf::from(strip_verbatim(&full.to_string_lossy())))
}

/// The plain form of a Windows verbatim path, and anything else unchanged.
/// A network share is spelled `\\?\UNC\server\share` in that form and
/// `\\server\share` in this one, which is why the answer is owned: the
/// two leading separators are not there to borrow.
pub fn strip_verbatim(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
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
        #[cfg(target_os = "macos")]
        assert!(path.to_string_lossy().contains("Application Support"));
        #[cfg(windows)]
        assert!(path.to_string_lossy().contains("infiniterm"));
    }

    // #118: a Mac keeps its data in Application Support, Linux where XDG says.
    #[test]
    fn the_default_data_dir_follows_the_os() {
        let home = std::path::Path::new("/home/u");
        assert_eq!(
            default_data_dir(home, None, false),
            home.join("Library/Application Support/dev.ekinertac.infiniterm")
        );
        assert_eq!(
            default_data_dir(home, None, true),
            home.join(".local/share/dev.ekinertac.infiniterm")
        );
        let xdg = std::ffi::OsString::from("/data");
        assert_eq!(
            default_data_dir(home, Some(&xdg), true),
            std::path::Path::new("/data/dev.ekinertac.infiniterm")
        );
        let relative = std::ffi::OsString::from("data");
        assert_eq!(
            default_data_dir(home, Some(&relative), true),
            home.join(".local/share/dev.ekinertac.infiniterm"),
            "a relative XDG_DATA_HOME is ignored"
        );
        assert_eq!(
            default_data_dir(home, Some(&xdg), false),
            home.join("Library/Application Support/dev.ekinertac.infiniterm"),
            "a Mac ignores it"
        );
    }

    // The override moves the save file and the drafts (and the socket, see
    // `socket_path`); without it, Application Support.
    #[test]
    fn the_data_dir_override_moves_the_save_file_and_the_drafts() {
        let dir = std::env::temp_dir().join("infiniterm-x");
        assert_eq!(data_dir(Some(dir.as_os_str())), dir);
        assert_ne!(data_dir(None), dir);
        #[cfg(not(windows))]
        assert!(data_dir(None)
            .to_string_lossy()
            .contains("dev.ekinertac.infiniterm"));
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

    #[test]
    fn an_instance_with_its_own_data_dir_starts_with_its_own_config() {
        use std::ffi::OsStr;
        let (cfg, data) = (OsStr::new("/c"), OsStr::new("/d"));
        assert_eq!(config_dir_for(Some(cfg), Some(data)), PathBuf::from("/c"));
        assert_eq!(
            config_dir_for(None, Some(data)),
            PathBuf::from("/d").join("config")
        );
        #[cfg(unix)]
        assert!(config_dir_for(None, None).ends_with(".config/infiniterm"));
        #[cfg(windows)]
        assert!(config_dir_for(None, None).ends_with("infiniterm/config"));
    }

    // The config is hand-edited and this is not; they must never collide.
    #[test]
    fn layout_path_is_not_the_config_path() {
        assert_ne!(layout_path().parent(), Some(config_dir().as_path()));
    }

    #[test]
    fn a_base_name_is_the_last_segment_under_either_separator() {
        assert_eq!(base_name("/Users/ekin/Code/a.rs"), "a.rs");
        assert_eq!(base_name(r"C:\Users\PC\Code\a.rs"), "a.rs");
        // Mixed, which is what a Windows shell hands back half the time.
        assert_eq!(base_name(r"C:/Users/PC\Code/a.rs"), "a.rs");
        assert_eq!(base_name("a.rs"), "a.rs");
        assert_eq!(base_name(""), "");
        // A trailing separator names nothing, and saying so beats guessing.
        assert_eq!(base_name("/Users/ekin/"), "");
    }

    #[test]
    fn split_parent_keeps_both_halves_or_neither() {
        assert_eq!(split_parent("/a/b/c"), Some(("/a/b", "c")));
        assert_eq!(split_parent(r"C:\a\b"), Some((r"C:\a", "b")));
        assert_eq!(split_parent("c"), None);
    }

    #[test]
    fn a_parent_dir_survives_a_drive_letter_and_a_root() {
        assert_eq!(parent_dir("/Users/ekin/a.rs"), "/Users/ekin");
        assert_eq!(parent_dir(r"C:\Users\PC\a.rs"), r"C:\Users\PC");
        // A drive with nothing between it and the file: the root, not `C:`.
        assert_eq!(parent_dir(r"C:\a.rs"), r"C:\");
        assert_eq!(parent_dir("/a.rs"), "/");
        assert_eq!(parent_dir("a.rs"), "/");
    }

    #[test]
    fn a_verbatim_path_comes_back_plain() {
        assert_eq!(strip_verbatim(r"\\?\C:\Users\PC"), r"C:\Users\PC");
        // A UNC share keeps its two leading separators.
        assert_eq!(strip_verbatim(r"\\?\UNC\server\share"), r"\\server\share");
        // Anything that is not verbatim is its own answer, unix paths included.
        assert_eq!(strip_verbatim(r"C:\Users\PC"), r"C:\Users\PC");
        assert_eq!(strip_verbatim("/Users/ekin"), "/Users/ekin");
        assert_eq!(strip_verbatim(""), "");
    }

    // The seam itself: whatever the OS hands back, nothing downstream sees a
    // prefix it cannot use.
    #[test]
    fn canonical_never_returns_a_verbatim_path() {
        let dir = std::env::temp_dir();
        let got = canonical(&dir).unwrap();
        let text = got.to_string_lossy();
        assert!(!text.starts_with(r"\\?\"), "{text}");
        assert!(canonical(dir.join("no-such-thing-xyz")).is_err());
    }

    #[test]
    fn a_rooted_path_is_one_that_names_its_own_place() {
        assert!(is_rooted("/Users/ekin/a.rs"));
        assert!(is_rooted(r"C:\Users\PC\a.rs"));
        assert!(is_rooted("C:/Users/PC/a.rs"));
        // A UNC share, and a Windows path rooted on the current drive.
        assert!(is_rooted(r"\\server\share"));
        assert!(is_rooted(r"\Users\PC"));
        // Relative, on either platform.
        assert!(!is_rooted("a.rs"));
        assert!(!is_rooted("src/a.rs"));
        assert!(!is_rooted(r"src"));
        assert!(!is_rooted("~/a.rs"), "a tilde is the caller's to expand");
        assert!(!is_rooted(""));
    }

    // Short on purpose: see this fn's own doc comment for the socket path
    // budget "sessions" would have spent instead.
    #[test]
    fn sessions_dir_is_under_the_data_dir_and_short() {
        assert!(sessions_dir().starts_with(app_support_dir()));
        assert_eq!(sessions_dir().file_name().unwrap(), "s");
    }
}
