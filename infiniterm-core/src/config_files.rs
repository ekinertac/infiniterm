//! The config directory at ~/.config/infiniterm/: four files in two pairs.
//! From the Tauri app's config.rs, minus the Tauri glue.
//!
//!   settings.json              yours, only what you changed
//!   settings.default.json      every setting with its default, commented
//!   keybindings.json           yours, only what you rebound
//!   keybindings.default.json   every binding, commented
//!
//! Beside them, `ui-themes/<name>.json` (#511): one file per look, the one
//! `ui.theme` names is applied (`ui_theme.rs`). The watcher reports every one.
//!
//! Sublime's arrangement: the defaults file can be read end to end to learn
//! what exists, while your file stays small enough to answer "what have I
//! changed?" at a glance. It also deletes machinery: when defaults and
//! overrides shared one file, a new release had to inject keys into a file
//! the user owns. The schema, the defaults and the comments live in
//! `config.rs`, `settings_doc.rs` and `keymap.rs`; this file moves bytes.
use crate::paths::{config_dir, home_dir};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime};

/// Which file, as a closed set: no string can name a path outside the
/// directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFile {
    Settings,
    SettingsDefault,
    Keybindings,
    KeybindingsDefault,
    /// `snippets.json`: the snippet picker's rows for one day, 2026-09-22.
    /// Read once to move its entries into `snippets_dir()`, then left alone.
    Snippets,
    /// The `ui-themes/` folder (#511): only the watcher's tag for "a file in
    /// it changed", the name riding in `ConfigChange::name`. Read and written
    /// through `ui_theme_read` / `ui_theme_write`, never by this variant.
    UiTheme,
}

impl ConfigFile {
    fn file_name(self) -> &'static str {
        match self {
            Self::Settings => "settings.json",
            Self::SettingsDefault => "settings.default.json",
            Self::Keybindings => "keybindings.json",
            Self::KeybindingsDefault => "keybindings.default.json",
            Self::Snippets => "snippets.json",
            Self::UiTheme => crate::ui_theme::DIR_NAME,
        }
    }

    /// True for the files the app owns and rewrites. Used to refuse to open
    /// one in an editor, since editing it does nothing.
    pub fn is_generated(self) -> bool {
        matches!(self, Self::SettingsDefault | Self::KeybindingsDefault)
    }
}

pub fn config_path(file: ConfigFile) -> PathBuf {
    config_dir().join(file.file_name())
}

/// Whether `path` is one of THIS machine's settings or keybindings files,
/// defaults included. A joined window's "open settings" makes editor cards on
/// these paths (the host's, which the viewer got from the file service), and
/// the host accepts exactly those from a viewer (#580).
pub fn is_config_file_path(path: &str) -> bool {
    [
        ConfigFile::Settings,
        ConfigFile::SettingsDefault,
        ConfigFile::Keybindings,
        ConfigFile::KeybindingsDefault,
    ]
    .iter()
    .any(|f| config_path(*f).to_string_lossy() == path)
}

/// The snippet picker's folder: one plain file per snippet (snippets.rs).
/// A folder rather than a `ConfigFile` because its unit is many files.
pub fn snippets_dir() -> PathBuf {
    config_dir().join("snippets")
}

/// The ui themes' folder: one `<name>.json` per look (#511).
pub fn ui_themes_dir() -> PathBuf {
    config_dir().join(crate::ui_theme::DIR_NAME)
}

/// Where a ui theme's logo files live: `ui-themes/<name>/icons/` (#544), or
/// `None` for a name that is not one plain file name. An empty name is the
/// built-in `default` theme.
pub fn ui_theme_icons_dir(name: &str) -> Option<PathBuf> {
    icons_dir_in(&ui_themes_dir(), name)
}

fn icons_dir_in(themes: &std::path::Path, name: &str) -> Option<PathBuf> {
    let name = if name.is_empty() {
        crate::ui_theme::DEFAULT_NAME
    } else {
        name
    };
    crate::ui_theme::valid_name(name).then(|| themes.join(name).join("icons"))
}

/// Where a ui theme lives, or `None` for a name that is not one plain file
/// name (`ui.theme` is typed by hand and must not reach outside the folder).
pub fn ui_theme_path(name: &str) -> Option<PathBuf> {
    crate::ui_theme::valid_name(name).then(|| ui_themes_dir().join(format!("{name}.json")))
}

/// A ui theme's text, or `None` when there is no such file.
pub fn ui_theme_read(name: &str) -> Option<String> {
    std::fs::read_to_string(ui_theme_path(name)?).ok()
}

/// Writes a ui theme, making the folder. Atomic like `config_write`.
pub fn ui_theme_write(name: &str, contents: &str) -> Result<(), String> {
    let path = ui_theme_path(name).ok_or_else(|| format!("\"{name}\" is not a ui theme name"))?;
    std::fs::create_dir_all(ui_themes_dir()).map_err(|e| e.to_string())?;
    crate::files::write_atomically(&path, contents).map_err(|e| e.to_string())
}

/// The names the picker lists: `default` and every `*.json` in the folder.
pub fn list_ui_themes() -> Vec<String> {
    let stems = std::fs::read_dir(ui_themes_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            (path.extension()? == "json")
                .then(|| path.file_stem()?.to_str().map(String::from))
                .flatten()
        });
    crate::ui_theme::names_from_stems(stems)
}

/// Startup, before settings.json is applied (#511): makes sure
/// `ui-themes/default.json` exists, and moves the look keys that an older
/// settings.json still holds into it (`look_migration`). Returns the keys
/// that moved. Errors are the caller's to log: a failed move leaves
/// settings.json as it was, and its look keys are then ignored with a notice.
pub fn migrate_look_keys() -> Result<Vec<String>, String> {
    use crate::ui_theme::DEFAULT_NAME;
    let seed = crate::settings_doc::default_ui_theme_text();
    let theme = ui_theme_read(DEFAULT_NAME);
    if let Some(settings) = config_read(ConfigFile::Settings) {
        if let Some(m) =
            crate::look_migration::migrate(&settings, theme.as_deref(), &seed, DEFAULT_NAME)
        {
            // The theme first: if the second write fails the keys are in
            // both files, which the next launch resolves without losing any.
            ui_theme_write(DEFAULT_NAME, &m.theme)?;
            config_write(ConfigFile::Settings, &m.settings)?;
            return Ok(m.keys);
        }
    }
    if theme.is_none() {
        ui_theme_write(DEFAULT_NAME, &seed)?;
    }
    Ok(vec![])
}

/// The single file this app used before the directory existed.
fn legacy_path() -> PathBuf {
    home_dir().join(".config").join("infiniterm.json")
}

/// A file's contents, or `None` when it does not exist yet.
pub fn config_read(file: ConfigFile) -> Option<String> {
    std::fs::read_to_string(config_path(file)).ok()
}

pub fn config_write(file: ConfigFile, contents: &str) -> Result<(), String> {
    let path = config_path(file);
    std::fs::create_dir_all(config_dir()).map_err(|e| e.to_string())?;
    // Write-then-rename, so an interrupted write cannot leave a truncated
    // config that fails to parse on the next launch, and through a symlink
    // (a settings.json kept in a dotfiles repo) rather than over it.
    crate::files::write_atomically(&path, contents).map_err(|e| e.to_string())
}

/// Moves the pre-directory `~/.config/infiniterm.json` in, returning where
/// it went. Moved rather than copied: two files both claiming to be the
/// config is a worse surprise than one that relocated and said where. Does
/// nothing if the new settings file exists, so it cannot overwrite settings.
pub fn config_migrate() -> Option<PathBuf> {
    let old = legacy_path();
    let new = config_path(ConfigFile::Settings);
    if !old.exists() || new.exists() {
        return None;
    }
    std::fs::create_dir_all(config_dir()).ok()?;
    std::fs::rename(&old, &new).ok()?;
    Some(new)
}

/// Opens a config file in whatever the system uses for JSON. Refuses the
/// generated ones: opening a file whose edits are discarded on the next
/// launch is an invitation to lose work.
pub fn config_open(file: ConfigFile) -> Result<(), String> {
    if file.is_generated() {
        return Err("that file is regenerated every launch; edit the one beside it".into());
    }
    let path = config_path(file);
    if !path.exists() {
        return Err(format!("{} does not exist yet", path.display()));
    }
    crate::links_fs::open_with_system(&path)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigChange {
    pub file: ConfigFile,
    /// The ui theme's name, for `ConfigFile::UiTheme`; `None` otherwise.
    pub name: Option<String>,
    pub contents: String,
}

/// One poll of the ui-themes folder: the `(name, contents)` of every file
/// that is new or changed since `last`, and of every one that went away (empty
/// contents, which is the built-in look). `last` is updated. Every theme is
/// watched, not only the active one, because the active one changes (the
/// picker, `ui.theme`) and the watcher does not know; the model drops the
/// ones that are not in force.
fn poll_ui_themes(
    dir: &std::path::Path,
    last: &mut std::collections::HashMap<String, SystemTime>,
) -> Vec<(String, String)> {
    let mut changed = vec![];
    let mut seen = std::collections::HashSet::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let Some(name) = (path.extension().is_some_and(|e| e == "json"))
            .then(|| path.file_stem().and_then(|s| s.to_str()).map(String::from))
            .flatten()
            .filter(|n| crate::ui_theme::valid_name(n))
        else {
            continue;
        };
        let Some(stamp) = entry.metadata().and_then(|m| m.modified()).ok() else {
            continue;
        };
        seen.insert(name.clone());
        if last.get(&name) != Some(&stamp) {
            last.insert(name.clone(), stamp);
            changed.push((name, std::fs::read_to_string(&path).unwrap_or_default()));
        }
    }
    let gone: Vec<String> = last
        .keys()
        .filter(|n| !seen.contains(*n))
        .cloned()
        .collect();
    for name in gone {
        last.remove(&name);
        changed.push((name, String::new()));
    }
    changed
}

/// Re-sends a user file whenever its mtime changes, so edits apply without
/// a restart. Only the files a person edits are watched: the generated
/// ones change on every launch by definition. Polling at 1 s rather than a
/// watcher dependency: a few files, edited by hand, and a second of latency
/// is invisible while tuning. The thread ends when the receiver is dropped.
pub fn config_watch(tx: Sender<ConfigChange>) {
    std::thread::spawn(move || {
        let watched = [ConfigFile::Settings, ConfigFile::Keybindings];
        let mut last: [Option<SystemTime>; 2] = [None; 2];
        let mut last_themes = std::collections::HashMap::new();
        loop {
            for (name, contents) in poll_ui_themes(&ui_themes_dir(), &mut last_themes) {
                let change = ConfigChange {
                    file: ConfigFile::UiTheme,
                    name: Some(name),
                    contents,
                };
                if tx.send(change).is_err() {
                    return;
                }
            }
            for (i, file) in watched.iter().enumerate() {
                let path = config_path(*file);
                let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                if stamp != last[i] {
                    last[i] = stamp;
                    let contents = std::fs::read_to_string(&path).unwrap_or_default();
                    if tx
                        .send(ConfigChange {
                            file: *file,
                            name: None,
                            contents,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_themes_logos_live_in_its_own_icons_folder_and_a_bad_name_has_none() {
        let themes = std::path::Path::new("/cfg/ui-themes");
        assert_eq!(
            icons_dir_in(themes, "glass"),
            Some(PathBuf::from("/cfg/ui-themes/glass/icons"))
        );
        assert_eq!(
            icons_dir_in(themes, ""),
            Some(PathBuf::from("/cfg/ui-themes/default/icons"))
        );
        assert_eq!(icons_dir_in(themes, "../x"), None);
    }

    #[test]
    fn every_file_lives_in_the_config_directory() {
        for file in [
            ConfigFile::Settings,
            ConfigFile::SettingsDefault,
            ConfigFile::Keybindings,
            ConfigFile::KeybindingsDefault,
            ConfigFile::UiTheme,
        ] {
            assert_eq!(config_path(file).parent(), Some(config_dir().as_path()));
        }
    }

    #[test]
    fn the_generated_files_are_the_default_ones() {
        assert!(ConfigFile::SettingsDefault.is_generated());
        assert!(ConfigFile::KeybindingsDefault.is_generated());
        assert!(!ConfigFile::Settings.is_generated());
        assert!(!ConfigFile::Keybindings.is_generated());
        assert!(!ConfigFile::UiTheme.is_generated());
    }

    // `ui.theme` is typed by hand: it must stay inside the folder.
    #[test]
    fn a_ui_theme_path_stays_inside_the_folder() {
        let p = ui_theme_path("glass").unwrap();
        assert_eq!(p.parent(), Some(ui_themes_dir().as_path()));
        assert!(p.ends_with("ui-themes/glass.json"));
        assert_eq!(ui_theme_path("../settings"), None);
        assert_eq!(ui_theme_path(""), None);
        assert_eq!(ui_theme_read("../../etc/passwd"), None);
    }

    #[test]
    fn the_watcher_reports_new_changed_and_removed_themes_once() {
        let dir = std::env::temp_dir().join(format!("ift-uitheme-poll-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut last = Default::default();
        assert!(poll_ui_themes(&dir, &mut last).is_empty());
        std::fs::write(dir.join("a.json"), "{}").unwrap();
        std::fs::write(dir.join("notes.txt"), "no").unwrap();
        std::fs::write(dir.join(".hidden.json"), "no").unwrap();
        assert_eq!(
            poll_ui_themes(&dir, &mut last),
            [("a".to_string(), "{}".to_string())]
        );
        // Nothing changed: nothing sent.
        assert!(poll_ui_themes(&dir, &mut last).is_empty());
        std::fs::remove_file(dir.join("a.json")).unwrap();
        assert_eq!(
            poll_ui_themes(&dir, &mut last),
            [("a".to_string(), String::new())]
        );
        assert!(poll_ui_themes(&dir, &mut last).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Opening a file whose edits are discarded next launch invites losing work.
    #[test]
    fn refuses_to_open_a_generated_file() {
        assert!(config_open(ConfigFile::SettingsDefault).is_err());
    }

    // The directory, not the old single file.
    #[test]
    fn the_legacy_path_is_not_one_of_the_new_ones() {
        assert_ne!(legacy_path(), config_path(ConfigFile::Settings));
        assert!(legacy_path().ends_with("infiniterm.json"));
    }
}
