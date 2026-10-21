//! Naming a file on its first save: what the "save as" field starts with,
//! and whether a typed path can be written. Pure; the model asks
//! (`model/cards_cmd.rs`, `open_save_as`, `Pending::SaveAs`) and the
//! filesystem is a closure so this is testable.
//!
//! Shaped like the macOS save panel, in our own dialog (2026-09-26, Ekin):
//! the field starts on the card's directory and a name, with only the name's
//! stem selected, so typing a name and Enter is the whole job and the
//! directory is there to edit. A missing directory is refused rather than
//! created (a typo'd path would quietly make folders), and an existing file
//! is asked about before it is replaced, which is what the panel does.
use crate::card_label::tilde_path;

/// The name an unnamed buffer is offered.
pub const DEFAULT_NAME: &str = "untitled.txt";

/// The field's starting text for a buffer in `dir`, and the char range to
/// select in it (the name's stem: `untitled`).
pub fn suggestion(dir: &str, home: &str) -> (String, (usize, usize)) {
    let dir = tilde_path(dir.trim_end_matches('/'), home);
    let prefix = if dir.is_empty() {
        "/".to_string()
    } else {
        format!("{dir}/")
    };
    let start = prefix.chars().count();
    let stem = DEFAULT_NAME
        .rsplit_once('.')
        .map_or(DEFAULT_NAME, |(s, _)| s);
    (
        format!("{prefix}{DEFAULT_NAME}"),
        (start, start + stem.chars().count()),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Nothing there: write it.
    New,
    /// A file is there: ask before replacing it.
    Replace,
    /// The directory it would go in does not exist.
    NoDirectory(String),
}

/// Whether the absolute `path` can be written, asking `exists` of the disk.
pub fn target(path: &str, exists: impl Fn(&str) -> bool) -> Target {
    let dir = match path.rsplit_once('/') {
        Some(("", _)) => "/",
        Some((d, _)) => d,
        None => ".",
    };
    if !exists(dir) {
        return Target::NoDirectory(dir.to_string());
    }
    if exists(path) {
        Target::Replace
    } else {
        Target::New
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_starts_on_the_directory_with_the_stem_selected() {
        let (text, (a, b)) = suggestion("/Users/me/Code", "/Users/me");
        assert_eq!(text, "~/Code/untitled.txt");
        let picked: String = text.chars().skip(a).take(b - a).collect();
        assert_eq!(picked, "untitled");
        assert_eq!(suggestion("/", "/Users/me").0, "/untitled.txt");
    }

    #[test]
    fn a_missing_directory_is_refused_and_an_existing_file_is_asked_about() {
        let disk = ["/Users/me", "/Users/me/notes.txt", "/"];
        let exists = |p: &str| disk.contains(&p);
        assert_eq!(target("/Users/me/new.txt", exists), Target::New);
        assert_eq!(target("/Users/me/notes.txt", exists), Target::Replace);
        assert_eq!(
            target("/Users/me/a/b.txt", exists),
            Target::NoDirectory("/Users/me/a".into())
        );
        assert_eq!(target("/top.txt", exists), Target::New);
    }
}
