//! Which icon a row of the editor's file tree wears when
//! `editor.explorer.icons` is on (#701): the Material Icon Theme's choice for
//! the file's name, else its extension, else the plain file; a folder gets a
//! closed or an open folder.
//!
//! Where it fits: `EditorBody::paint_tree` (infiniterm-ui/src/editor_body.rs)
//! asks `icon_for` for each row and draws the SVG of that name, which
//! `explorer_icons.rs` in the ui crate serves to gpui. The tables are
//! generated (`explorer_icon_table.rs`, by tools/explorer-icons.py).
//!
//! The order is VS Code's: an exact file name beats an extension, and a longer
//! extension beats a shorter one (`foo.d.ts` is `d.ts` before `ts`,
//! `x.spec.tsx` is `spec.tsx`). Matching ignores case.
use crate::explorer_icon_table::{FILE, FILE_EXTENSIONS, FILE_NAMES, FOLDER, FOLDER_OPEN};

/// The icon's name (an SVG file in assets/explorer-icons, without `.svg`).
pub fn icon_for(name: &str, is_dir: bool, expanded: bool) -> &'static str {
    if is_dir {
        return if expanded { FOLDER_OPEN } else { FOLDER };
    }
    let name = name.to_lowercase();
    if let Some(icon) = find(FILE_NAMES, &name) {
        return icon;
    }
    // Every suffix after a dot, longest first. A leading dot is a hidden
    // file's name, not an extension, so `.env` is only a file name; a name
    // the table lacks is the plain file.
    name.match_indices('.')
        .filter(|(at, _)| *at > 0)
        .find_map(|(at, _)| find(FILE_EXTENSIONS, &name[at + 1..]))
        .unwrap_or(FILE)
}

fn find(table: &'static [(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    table
        .binary_search_by(|(k, _)| (*k).cmp(key))
        .ok()
        .map(|i| table[i].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_are_sorted_for_the_binary_search() {
        for table in [FILE_NAMES, FILE_EXTENSIONS] {
            assert!(table.windows(2).all(|w| w[0].0 < w[1].0));
        }
    }

    #[test]
    fn an_extension_picks_its_language() {
        assert_eq!(icon_for("main.rs", false, false), "rust");
        assert_eq!(icon_for("app.py", false, false), "python");
        assert_eq!(icon_for("NOTES.MD", false, false), "markdown");
    }

    #[test]
    fn a_file_name_beats_its_extension_and_a_longer_extension_a_shorter_one() {
        assert_eq!(icon_for("README.md", false, false), "readme");
        assert_eq!(icon_for("package.json", false, false), "nodejs");
        assert_eq!(icon_for(".gitignore", false, false), "git");
        assert_eq!(icon_for("x.spec.tsx", false, false), "test-jsx");
        assert_eq!(icon_for("index.d.ts", false, false), "typescript-def");
        assert_eq!(icon_for("index.ts", false, false), "typescript");
    }

    #[test]
    fn an_unknown_or_bare_name_is_the_plain_file() {
        assert_eq!(icon_for("notes.zzzunknown", false, false), FILE);
        assert_eq!(icon_for("LICENSEX", false, false), FILE);
        assert_eq!(icon_for("trailingdot.", false, false), FILE);
    }

    #[test]
    fn a_folder_is_closed_or_open() {
        assert_eq!(icon_for("src", true, false), "folder");
        assert_eq!(icon_for("src", true, true), "folder-open");
    }
}
