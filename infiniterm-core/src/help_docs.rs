//! The docs, inside the app: the website's guide pages and its generated
//! reference pages, written as Markdown into `<data>/docs/` at every launch
//! and opened by `help.docs` in a read-only editor card with the file tree
//! beside it, so the tree is the table of contents and Cmd+F is search.
//!
//! One source with infiniterm.app: the guide pages are the site's own files
//! (`site/src/content/docs/guide/*.md`), compiled in with `include_str!`,
//! and the reference pages come from `site_reference::pages`, which the
//! site build also runs. A page edited for the site is the page the app
//! shows from the next build on. Ekin chose this over sending people to the
//! website (2026-10-02): offline, instant, and it cannot drift.
//!
//! The app copy is cleaned for reading as text: the front matter becomes a
//! heading and a one-line summary, and the `<kbd>` tags the reference pages
//! use become backticks. Links stay as written; they point at the site's
//! paths and read fine. `files` is pure and tested; the ui writes them
//! (`runtime.rs`) and `editors.rs` marks the folder read-only: a click locks
//! the keyboard like any editor's (#41), so the arrows, Page Down and Cmd+F
//! work, and typing does nothing.
use crate::paths::app_support_dir;
use std::path::PathBuf;

/// Where the pages are written: under the data dir, so a scratch instance
/// has its own.
pub fn docs_dir() -> PathBuf {
    app_support_dir().join("docs")
}

/// The guide, in the order the site's sidebar lists it. The number in each
/// file name is what orders the editor's tree, which sorts by name.
const GUIDE: &[(&str, &str)] = &[
    (
        "01 Install.md",
        include_str!("../../site/src/content/docs/guide/install.md"),
    ),
    (
        "02 The canvas.md",
        include_str!("../../site/src/content/docs/guide/canvas.md"),
    ),
    (
        "03 Card states.md",
        include_str!("../../site/src/content/docs/guide/card-states.md"),
    ),
    (
        "04 Terminal cards.md",
        include_str!("../../site/src/content/docs/guide/terminal.md"),
    ),
    (
        "05 Editor cards.md",
        include_str!("../../site/src/content/docs/guide/editor.md"),
    ),
    (
        "06 ift and sessions.md",
        include_str!("../../site/src/content/docs/guide/ift-and-sessions.md"),
    ),
    (
        "07 Configuration.md",
        include_str!("../../site/src/content/docs/guide/configuration.md"),
    ),
];

/// The generated reference pages, by their file in `site_reference`.
const REFERENCE: &[(&str, &str)] = &[
    ("keys.md", "08 Keys.md"),
    ("commands.md", "09 Commands.md"),
    ("settings.md", "10 Settings.md"),
];

/// The page `help.docs` opens first.
pub const FIRST_PAGE: &str = "01 Install.md";

/// Every page, as (file name, Markdown), ready to write.
pub fn files() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = GUIDE
        .iter()
        .map(|(name, text)| (name.to_string(), clean(text)))
        .collect();
    let pages = crate::site_reference::pages();
    for (site_file, name) in REFERENCE {
        if let Some(p) = pages.iter().find(|p| p.file == *site_file) {
            out.push((name.to_string(), clean(&p.text)));
        }
    }
    out
}

/// Front matter to a heading and a summary line; `<kbd>X</kbd>` to `` `X` ``.
pub fn clean(text: &str) -> String {
    let (title, description, body) = split_front_matter(text);
    let mut out = String::new();
    if let Some(t) = title {
        out.push_str(&format!("# {t}\n\n"));
    }
    if let Some(d) = description {
        out.push_str(&format!("{d}\n\n"));
    }
    out.push_str(body.trim_start());
    out.replace("<kbd>", "`").replace("</kbd>", "`")
}

fn split_front_matter(text: &str) -> (Option<String>, Option<String>, &str) {
    let Some(rest) = text.strip_prefix("---\n") else {
        return (None, None, text);
    };
    let Some(end) = rest.find("\n---\n") else {
        return (None, None, text);
    };
    let field = |key: &str| {
        rest[..end]
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{key}: ")))
            .map(|v| v.trim().to_string())
    };
    (field("title"), field("description"), &rest[end + 5..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_becomes_a_heading_and_a_summary() {
        let text = clean("---\ntitle: Keys\ndescription: Every key.\n---\n\nBody here.\n");
        assert_eq!(text, "# Keys\n\nEvery key.\n\nBody here.\n");
    }

    #[test]
    fn a_page_without_front_matter_is_left_alone() {
        assert_eq!(clean("# Hi\n\ntext\n"), "# Hi\n\ntext\n");
    }

    #[test]
    fn kbd_tags_become_backticks() {
        assert_eq!(clean("<kbd>Cmd</kbd> <kbd>T</kbd>"), "`Cmd` `T`");
    }

    // Every guide page on the site is in the app: a new page added to the
    // site's guide folder without a line here fails this test.
    #[test]
    fn every_guide_page_on_the_site_is_shipped() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../site/src/content/docs/guide");
        let on_site = std::fs::read_dir(&dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|x| x == "md")
            })
            .count();
        assert_eq!(
            on_site,
            GUIDE.len(),
            "a guide page in {} is missing from GUIDE",
            dir.display()
        );
    }

    #[test]
    fn the_files_are_the_guide_then_the_reference_in_order() {
        let names: Vec<String> = files().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names.first().map(String::as_str), Some(FIRST_PAGE));
        assert_eq!(names.last().map(String::as_str), Some("10 Settings.md"));
        assert_eq!(names.len(), GUIDE.len() + REFERENCE.len());
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "the tree sorts by name, so the numbers must");
    }

    #[test]
    fn no_page_keeps_front_matter_or_kbd_tags() {
        for (name, text) in files() {
            assert!(!text.starts_with("---"), "{name} kept its front matter");
            assert!(!text.contains("<kbd>"), "{name} kept <kbd> tags");
        }
    }
}
