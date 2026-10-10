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
    fn the_real_file_has_releases_and_no_maintainer_note() {
        let shown = text();
        assert!(shown.contains("\n## 0.5."), "release sections are there");
        assert!(!shown.contains("How this file is kept"));
        assert!(shown.len() > 1000);
    }
}
