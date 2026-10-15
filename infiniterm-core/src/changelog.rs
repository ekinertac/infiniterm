//! The changelog as a card: `CHANGELOG.md` compiled into the app and shown as
//! a Page (`help.changelog`, the palette's "Help: open the changelog").
//!
//! The file is the product: what shipped, newest first, written for a user,
//! kept by day under "Unreleased" and folded by release at each publish. The
//! card shows it as it is (Ekin, 2026-10-09: showing the file is enough, no
//! "version you last ran" to store), minus the paragraph at the top that tells
//! the maintainers how the file is kept. Written under the data dir at every
//! launch like the docs, so a scratch instance has its own and an update
//! brings the new text with it.
//!
//! The same file is also the "what's new" overlay shown on the first launch
//! after an update (#402): `released_text` is the whole changelog without
//! the Unreleased days, `version_newer` decides when to show it from the
//! version stored in `<data>/last_version` (`read_last_version` /
//! `write_last_version`).
//!
//! Called by `runtime.rs` (the write) and `Model::open_changelog`
//! (`model/persist.rs`, the card). Related: `help_docs.rs`, `welcome.rs`.
use crate::paths::app_support_dir;
use std::path::PathBuf;

/// The file this build was made with.
const CHANGELOG: &str = include_str!("../../CHANGELOG.md");

/// Where the card's file lives: a name that reads well on the card's label.
pub fn changelog_path() -> PathBuf {
    app_support_dir().join("changelog").join("Changelog.md")
}

/// The text the card shows.
pub fn text() -> String {
    for_card(CHANGELOG)
}

/// Where the "what's new" overlay's file lives (`PageBody` reads a file).
pub fn whats_new_path() -> PathBuf {
    app_support_dir().join("changelog").join("What's new.md")
}

/// The changelog the overlay shows: every release, newest first, without the
/// Unreleased days (Ekin, 2026-10-15: they are not in a build anyone has).
pub fn released_text() -> String {
    without_unreleased(&for_card(CHANGELOG))
}

/// `text` without the `## Unreleased` section: from that heading to the next
/// `## ` one, or the end.
fn without_unreleased(text: &str) -> String {
    let Some(start) = text.find("\n## Unreleased") else {
        return text.to_string();
    };
    let after = start + 1;
    match text[after..].find("\n## ") {
        Some(next) => format!("{}{}", &text[..start], &text[after + next..]),
        None => text[..start].to_string(),
    }
}

/// Whether `running` is a later release than `stored`, both `X.Y.Z`. A string
/// that is not three numbers is never newer, so a damaged file shows nothing.
pub fn version_newer(stored: &str, running: &str) -> bool {
    fn parts(v: &str) -> Option<[u64; 3]> {
        let mut it = v.trim().split('.').map(|n| n.parse::<u64>().ok());
        let p = [it.next()??, it.next()??, it.next()??];
        it.next().is_none().then_some(p)
    }
    match (parts(stored), parts(running)) {
        (Some(a), Some(b)) => b > a,
        _ => false,
    }
}

/// The file the version last run is kept in.
pub fn last_version_path() -> PathBuf {
    app_support_dir().join("last_version")
}

pub fn read_last_version() -> Option<String> {
    let v = std::fs::read_to_string(last_version_path()).ok()?;
    let v = v.trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// Records `version` as the one that ran; nothing is written when it is
/// already there.
pub fn write_last_version(version: &str) -> std::io::Result<()> {
    if read_last_version().as_deref() == Some(version) {
        return Ok(());
    }
    let path = last_version_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, version)
}

/// `file` without the maintainers' note: the title, then everything from the
/// first release heading.
fn for_card(file: &str) -> String {
    match file.find("\n## ") {
        Some(at) => format!("# Changelog\n{}", &file[at..]),
        None => file.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_drops_the_note_on_how_the_file_is_kept() {
        let file = "# Changelog\n\nWhat changed.\n\nHow this file is kept: by day.\n\n## Unreleased\n\n- a\n\n## 0.5.9 (build 767), 2026-10-08\n\n- b\n";
        let shown = for_card(file);
        assert!(shown.starts_with("# Changelog\n"));
        assert!(!shown.contains("How this file is kept"));
        assert!(shown.contains("## Unreleased") && shown.contains("## 0.5.9"));
    }

    #[test]
    fn the_overlay_text_has_no_unreleased_days() {
        let file = "# Changelog\n\nnote\n\n## Unreleased\n\n### 2026-10-14\n\n- soon\n\n## 0.5.12 (build 884), 2026-10-14\n\n- shipped\n\n## 0.5.11 (build 801), 2026-10-09\n\n- older\n";
        let shown = without_unreleased(&for_card(file));
        assert!(!shown.contains("Unreleased") && !shown.contains("soon"));
        assert!(shown.contains("## 0.5.12") && shown.contains("- shipped"));
        assert!(shown.contains("## 0.5.11") && shown.contains("- older"));
        // Unreleased last: nothing after it to keep.
        let only = "# Changelog\n\n## 0.5.1\n\n- a\n\n## Unreleased\n\n- b\n";
        assert_eq!(without_unreleased(only), "# Changelog\n\n## 0.5.1\n\n- a\n");
        let real = released_text();
        assert!(!real.contains("## Unreleased"));
        assert!(real.contains("\n## 0.5."));
    }

    #[test]
    fn a_later_release_is_newer_and_nothing_else_is() {
        assert!(version_newer("0.5.12", "0.5.13"));
        assert!(version_newer("0.5.9", "0.5.10"), "numbers, not text");
        assert!(version_newer("0.5.12", "0.6.0"));
        assert!(!version_newer("0.5.12", "0.5.12"));
        assert!(
            !version_newer("0.5.13", "0.5.12"),
            "a downgrade shows nothing"
        );
        assert!(!version_newer("", "0.5.12"));
        assert!(!version_newer("junk", "0.5.12"));
        assert!(!version_newer("0.5.12", "0.5"));
    }

    #[test]
    fn the_real_file_has_releases_and_no_maintainer_note() {
        let shown = text();
        assert!(shown.contains("\n## 0.5."), "release sections are there");
        assert!(!shown.contains("How this file is kept"));
        assert!(shown.len() > 1000);
    }
}
