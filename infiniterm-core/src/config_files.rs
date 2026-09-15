//! The config directory at ~/.config/infiniterm/: four files in two pairs.
//! From the Tauri app's config.rs, minus the Tauri glue.
//!
//!   settings.json              yours, only what you changed
//!   settings.default.json      every setting with its default, commented
//!   keybindings.json           yours, only what you rebound
//!   keybindings.default.json   every binding, commented
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
}

impl ConfigFile {
    fn file_name(self) -> &'static str {
        match self {
            Self::Settings => "settings.json",
            Self::SettingsDefault => "settings.default.json",
            Self::Keybindings => "keybindings.json",
            Self::KeybindingsDefault => "keybindings.default.json",
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
    // config that fails to parse on the next launch.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
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
    pub contents: String,
}

/// Re-sends a user file whenever its mtime changes, so edits apply without
/// a restart. Only the two files a person edits are watched: the generated
/// ones change on every launch by definition. Polling at 1 s rather than a
/// watcher dependency: two files, edited by hand, and a second of latency is
/// invisible while tuning. The thread ends when the receiver is dropped.
pub fn config_watch(tx: Sender<ConfigChange>) {
    std::thread::spawn(move || {
        let watched = [ConfigFile::Settings, ConfigFile::Keybindings];
        let mut last: [Option<SystemTime>; 2] = [None, None];
        loop {
            for (i, file) in watched.iter().enumerate() {
                let path = config_path(*file);
                let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                if stamp != last[i] {
                    last[i] = stamp;
                    let contents = std::fs::read_to_string(&path).unwrap_or_default();
                    if tx
                        .send(ConfigChange {
                            file: *file,
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
    fn every_file_lives_in_the_config_directory() {
        for file in [
            ConfigFile::Settings,
            ConfigFile::SettingsDefault,
            ConfigFile::Keybindings,
            ConfigFile::KeybindingsDefault,
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
