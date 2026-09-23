//! Extensions beyond the Claude in Chrome one `infiniterm-browser` seeds on
//! its own: `ift install-extension <path|id|store-url>` puts an unpacked
//! copy under `<browser_dir>/extensions/<id>`, and `installed_extensions`
//! is what `infiniterm-browser::process::seed` adds to the Claude
//! extension's own directory when it builds the `--load-extension` list.
//!
//! Every function here takes the browser directory explicitly
//! (`paths::browser_dir()`'s caller resolves it once), the same shape
//! `process::seed` already uses, so a test never touches the real one and
//! `INFINITERM_DATA_DIR` isolates a scratch instance's installs the way it
//! does everything else.
//!
//! Related: infiniterm-browser/src/process.rs (seeds the Claude extension
//! into `<browser_dir>/extension`, unchanged, and loads this module's list
//! alongside it), infiniterm-cli/src/main.rs (`ift install-extension`).
use crate::paths::chrome_support_dir;
use std::path::{Path, PathBuf};

/// Chrome's newest installed copy of extension `id`, if any: Chrome keeps
/// every version it has ever unpacked under one directory per id, and only
/// the newest is the one actually running. `infiniterm-browser::process`
/// used to have its own copy of this, hardcoded to the Claude extension's
/// id; this is that function, generalised, so both it and
/// `ift install-extension` resolve an id the same way.
pub fn chrome_extension_dir(id: &str) -> Option<PathBuf> {
    let dir = chrome_support_dir().join("Default/Extensions").join(id);
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("manifest.json").is_file())
        .collect();
    versions.sort();
    versions.pop()
}

/// Regular files and directories only: a Chrome extension directory holds
/// no locks of its own, but the profile-seeding code beside this shares
/// the same rule (`Singleton*` files must not travel), so this stays the
/// one copy of it.
pub fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)?.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("Singleton") {
            continue;
        }
        let target = to.join(&name);
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Chrome and CEF's extension id: 32 characters, each one of `a`..`p` (a
/// base16 alphabet shifted so ids read as pronounceable nonsense words
/// instead of hex), derived from the extension's signing key. Distinct
/// enough from an ordinary path or word that checking the shape alone is
/// enough to tell an id from a directory a caller meant literally.
fn looks_like_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

/// An extension id from a bare id, or from a Chrome Web Store url — either
/// the current `chromewebstore.google.com/detail/<name>/<id>` or the
/// pre-2022 `chrome.google.com/webstore/detail/<name>/<id>` — `None` when
/// `input` is neither.
pub fn parse_extension_id(input: &str) -> Option<String> {
    let trimmed = input.trim().trim_end_matches('/');
    if looks_like_id(trimmed) {
        return Some(trimmed.to_string());
    }
    let path = trimmed.split('?').next().unwrap_or(trimmed);
    let last = crate::paths::base_name(path);
    looks_like_id(last).then(|| last.to_string())
}

/// `<browser_dir>/extensions`: where `install` puts what it installs and
/// `installed_extensions` reads back from.
pub fn extensions_dir(browser_dir: &Path) -> PathBuf {
    browser_dir.join("extensions")
}

/// Every extra extension's directory, one per subdirectory of
/// `extensions_dir` that has a `manifest.json`, sorted so the
/// `--load-extension` list `infiniterm-browser::process::seed` builds from
/// it is the same on every launch.
pub fn installed_extensions(browser_dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(extensions_dir(browser_dir))
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.join("manifest.json").is_file())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}

/// `ift install-extension <path|id|store-url>`: copies an unpacked
/// extension into `extensions_dir`, where the browser picks it up on its
/// next launch — CEF takes `--load-extension` once, at startup, so a
/// running app does not see a fresh install until it restarts.
///
/// `source` is either a local unpacked directory (its own name becomes the
/// destination's), or an id / Chrome Web Store url, looked up in the
/// user's own Chrome profile: CEF cannot install a signed `.crx` from the
/// store directly, but Chrome unpacks one the moment it is installed
/// there, which is how Ekin already has Dark Reader on disk. Returns the
/// installed directory, or why it could not be.
pub fn install(browser_dir: &Path, source: &str) -> Result<PathBuf, String> {
    let path = Path::new(source);
    let (src, name) = if path.join("manifest.json").is_file() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{source}: no directory name"))?;
        (path.to_path_buf(), name)
    } else if let Some(id) = parse_extension_id(source) {
        let src = chrome_extension_dir(&id).ok_or_else(|| {
            format!(
                "{id}: not found in Chrome's own Extensions (~/Library/Application Support/Google/Chrome/Default/Extensions) — install it in Chrome first, CEF cannot pull a .crx from the store directly"
            )
        })?;
        (src, id)
    } else {
        return Err(format!(
            "{source}: not a directory with a manifest.json, and not an extension id or Chrome Web Store url"
        ));
    };
    let dest = extensions_dir(browser_dir).join(&name);
    copy_dir(&src, &dest).map_err(|e| format!("{source}: {e}"))?;
    Ok(dest)
}

#[cfg(test)]
mod parse_extension_id_tests {
    use super::*;

    const DARK_READER: &str = "eimadpbcbfnmbkopoojfekhnkhdbieeh";

    #[test]
    fn a_bare_id_is_itself() {
        assert_eq!(parse_extension_id(DARK_READER), Some(DARK_READER.into()));
    }

    #[test]
    fn a_web_store_url_ends_in_the_id() {
        assert_eq!(
            parse_extension_id(&format!(
                "https://chromewebstore.google.com/detail/dark-reader/{DARK_READER}"
            )),
            Some(DARK_READER.into())
        );
        // The pre-2022 host, and a trailing slash.
        assert_eq!(
            parse_extension_id(&format!(
                "https://chrome.google.com/webstore/detail/dark-reader/{DARK_READER}/"
            )),
            Some(DARK_READER.into())
        );
    }

    #[test]
    fn a_query_string_does_not_hide_the_id() {
        assert_eq!(
            parse_extension_id(&format!(
                "https://chromewebstore.google.com/detail/dark-reader/{DARK_READER}?hl=en"
            )),
            Some(DARK_READER.into())
        );
    }

    #[test]
    fn neither_a_path_nor_a_short_word_is_an_id() {
        assert_eq!(parse_extension_id("./my-extension"), None);
        assert_eq!(parse_extension_id("dark-reader"), None);
        // 32 chars but outside a..p: not the alphabet an id is drawn from.
        assert_eq!(parse_extension_id("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"), None);
    }
}

#[cfg(test)]
mod install_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ift-extensions-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_local_unpacked_directory_is_copied_in_by_its_own_name() {
        let src = scratch("src");
        std::fs::create_dir_all(src.join("icons")).unwrap();
        std::fs::write(src.join("manifest.json"), "{}").unwrap();
        std::fs::write(src.join("icons/16.png"), b"fake-png").unwrap();

        let browser = scratch("browser");
        let dest = install(&browser, src.to_str().unwrap()).unwrap();

        assert!(dest.join("manifest.json").is_file());
        assert!(dest.join("icons/16.png").is_file());
        assert_eq!(
            installed_extensions(&browser),
            vec![dest],
            "the copy is what a later launch would load"
        );

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&browser);
    }

    #[test]
    fn neither_a_path_nor_an_id_is_a_clear_error_not_a_panic() {
        let browser = scratch("browser-err");
        let err = install(&browser, "not-a-real-path-or-id").unwrap_err();
        assert!(err.contains("not-a-real-path-or-id"));
        let _ = std::fs::remove_dir_all(&browser);
    }
}
