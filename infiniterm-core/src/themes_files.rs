//! Terminal colour schemes on disk. From the Tauri app's themes.rs; the
//! parser is `itermcolors.rs`.
//!
//! The whole iTerm2-Color-Schemes collection ships with the app and `seed`
//! copies it into the themes directory once per process, each file only if
//! it is not there yet: this adds, never overwrites, so a scheme you edited
//! or deleted stays that way. `theme: null` in the settings means the
//! default (Catppuccin Mocha), not "no theme".
use crate::paths::themes_dir;
use std::path::{Path, PathBuf};
use std::sync::Once;

/// The themes directory, created and seeded from `bundled` (the app's
/// resources) on the first call of the process. Five hundred `exists`
/// checks are cheap, but not on every theme preview.
pub fn ensure_themes_dir(bundled: Option<&Path>) -> PathBuf {
    static SEEDED: Once = Once::new();
    let dir = themes_dir();
    std::fs::create_dir_all(&dir).ok();
    if let Some(bundled) = bundled {
        SEEDED.call_once(|| seed(bundled, &dir));
    }
    dir
}

fn is_scheme(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("itermcolors"))
}

fn seed(bundled: &Path, dir: &Path) {
    let Ok(entries) = std::fs::read_dir(bundled) else {
        return;
    };
    for entry in entries.flatten() {
        let from = entry.path();
        if !is_scheme(&from) {
            continue;
        }
        let Some(name) = from.file_name() else {
            continue;
        };
        let to = dir.join(name);
        if !to.exists() {
            let _ = std::fs::copy(&from, &to);
        }
    }
}

/// Scheme names, without the extension, sorted so cycling has a stable order.
pub fn list_themes(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_scheme(p))
        .filter_map(|p| Some(p.file_stem()?.to_string_lossy().into_owned()))
        .collect();
    names.sort();
    names
}

/// A scheme's text. Rejects separators rather than joining blindly: a name
/// arrives from a settings file and `../` would read outside the directory.
pub fn read_theme(dir: &Path, name: &str) -> Result<String, String> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("invalid theme name".into());
    }
    let path = dir.join(format!("{name}.itermcolors"));
    std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Native check: seeding adds and never overwrites, and listing is sorted
    // without extensions.
    #[test]
    fn seed_adds_without_overwriting_and_list_is_sorted() {
        let base = std::env::temp_dir().join(format!("infiniterm-themes-{}", std::process::id()));
        let (bundled, dir) = (base.join("bundled"), base.join("themes"));
        std::fs::create_dir_all(&bundled).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(bundled.join("Zed.itermcolors"), "bundled").unwrap();
        std::fs::write(bundled.join("Apple.itermcolors"), "bundled").unwrap();
        std::fs::write(bundled.join("notes.txt"), "no").unwrap();
        std::fs::write(dir.join("Apple.itermcolors"), "edited").unwrap();
        seed(&bundled, &dir);
        assert_eq!(list_themes(&dir), ["Apple", "Zed"]);
        assert_eq!(read_theme(&dir, "Apple").unwrap(), "edited");
        assert!(read_theme(&dir, "../Apple").is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
