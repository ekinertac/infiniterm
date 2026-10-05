//! Completion for the editor's popup: a provider answers "what could go
//! here?" for the text before the caret. The first provider completes the
//! setting names in `settings.json`, from the same table that writes
//! `settings.default.json` (`settings_doc.rs`), so a new setting is
//! offered the moment it is documented. No LSP: the keys are ours.
//!
//! Pure over a line of text, so the rules are tested without an editor.
//! `infiniterm-ui/src/editor_body.rs` asks `completer_for(path)` when a file
//! loads, calls the result after every key and draws the popup; accepting
//! an item replaces the text from `Offer::from` to the caret.
//!
//! A provider is a plain function (`Completer`) so a better one (values,
//! buffer words, tree-sitter, an LSP client) can take its place without a
//! new popup. The offer is for one line: nothing here looks at the lines
//! above, which is enough for flat `"key": value` files.
use crate::config::{default_config, flatten};
use crate::fuzzy::fuzzy_match;
use crate::settings_doc::{doc, scalar};
use std::path::Path;
use std::sync::OnceLock;

/// One thing the popup can insert.
#[derive(Clone, Debug, PartialEq)]
pub struct Completion {
    /// What is inserted, and what the row shows first.
    pub label: String,
    /// Shown dimmed after the label: the setting's default value.
    pub detail: String,
    /// What it does, shown under the list while it is selected (the
    /// description's lines joined; the popup cuts it to its width).
    pub doc: String,
}

/// The popup's content: `items` replace the chars of the line from column
/// `from` (a char index) up to the caret.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub from: usize,
    pub items: Vec<Completion>,
}

/// A provider: the line's text before the caret in, an offer out.
pub type Completer = fn(&str) -> Option<Offer>;

/// The most rows an offer carries; the popup shows fewer and scrolls.
pub const MAX_ITEMS: usize = 50;

/// The provider for a file, if it has one: `settings.json` in the config
/// folder. Not `settings.default.json`, which is read-only.
pub fn completer_for(path: &str) -> Option<Completer> {
    completer_in(path, &crate::paths::config_dir())
}

/// `completer_for` against an explicit config folder. The two paths are
/// compared as real paths: the app is handed `/private/tmp/...` for a file
/// in `/tmp/...` (a symlink), and a plain comparison missed the match.
fn completer_in(path: &str, config_dir: &Path) -> Option<Completer> {
    let p = Path::new(path);
    if p.file_name()? != "settings.json" {
        return None;
    }
    let dir = p.parent()?;
    let same = dir == config_dir
        || matches!(
            (std::fs::canonicalize(dir), std::fs::canonicalize(config_dir)),
            (Ok(a), Ok(b)) if a == b
        );
    same.then_some(settings_keys as Completer)
}

/// The text typed so far inside a JSON KEY string at the end of `before`,
/// with the column it starts at (just after the opening quote). `None` when
/// the caret is not in a key: outside quotes, in a value, or in a comment.
fn key_prefix(before: &str) -> Option<(usize, String)> {
    if before.trim_start().starts_with("//") {
        return None;
    }
    let chars: Vec<char> = before.chars().collect();
    let mut open: Option<usize> = None;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' if open.is_some() => i += 1,
            '"' => open = if open.is_some() { None } else { Some(i) },
            _ => {}
        }
        i += 1;
    }
    let open = open?;
    // A key follows nothing, `{` or `,`; after a `:` it is a value.
    let lead: String = chars[..open].iter().collect();
    let lead = lead.trim_end();
    if !(lead.is_empty() || lead.ends_with('{') || lead.ends_with(',')) {
        return None;
    }
    Some((open + 1, chars[open + 1..].iter().collect()))
}

/// Every setting as a completion, built once: key order as the defaults
/// file lists them.
fn all_settings() -> &'static [Completion] {
    static ALL: OnceLock<Vec<Completion>> = OnceLock::new();
    ALL.get_or_init(|| {
        let value = serde_json::to_value(default_config()).expect("config serialises");
        let flat = value.as_object().map(flatten).unwrap_or_default();
        flat.iter()
            .map(|(key, v)| Completion {
                label: key.clone(),
                detail: scalar(v),
                doc: doc(key).join(" "),
            })
            .collect()
    })
}

/// Setting names for the key being typed. A prefix with a dot (`ui.`,
/// `ui.fit`) matches keys that start with it, so `ui.` lists the group;
/// without a dot the letters match anywhere in the key, in order (`fitpad`
/// finds `ui.fitPadding`), best first. Nothing before the first letter.
pub fn settings_keys(before: &str) -> Option<Offer> {
    let (from, prefix) = key_prefix(before)?;
    if prefix.is_empty() {
        return None;
    }
    let lower = prefix.to_lowercase();
    let mut items: Vec<(f64, &Completion)> = all_settings()
        .iter()
        .filter_map(|c| {
            if prefix.contains('.') {
                c.label
                    .to_lowercase()
                    .starts_with(&lower)
                    .then_some((0., c))
            } else {
                fuzzy_match(&prefix, &c.label).map(|m| (m.score, c))
            }
        })
        .collect();
    // Best score first for the fuzzy case; the dotted case keeps the file's order.
    if !prefix.contains('.') {
        items.sort_by(|a, b| b.0.total_cmp(&a.0));
    }
    let items: Vec<Completion> = items
        .into_iter()
        .map(|(_, c)| c.clone())
        .take(MAX_ITEMS)
        .collect();
    // Typing a whole key leaves nothing to offer.
    if items.len() == 1 && items[0].label == prefix {
        return None;
    }
    (!items.is_empty()).then_some(Offer { from, items })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(offer: &Offer) -> Vec<&str> {
        offer.items.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    fn a_key_in_progress_is_found_after_a_quote() {
        assert_eq!(key_prefix("  \"ui.fi"), Some((3, "ui.fi".into())));
        assert_eq!(key_prefix("\""), Some((1, String::new())));
        assert_eq!(key_prefix("{ \"a\": 1, \"ui"), Some((11, "ui".into())));
    }

    #[test]
    fn values_comments_and_closed_strings_are_not_keys() {
        assert_eq!(key_prefix("  \"theme\": \"Du"), None, "a value");
        assert_eq!(key_prefix("  // \"ui."), None, "a comment");
        assert_eq!(key_prefix("  \"ui.fitPadding\""), None, "a closed string");
        assert_eq!(key_prefix("  ui."), None, "no quote");
    }

    #[test]
    fn a_dotted_prefix_lists_the_group_with_defaults_and_docs() {
        let offer = settings_keys("  \"ui.").unwrap();
        assert_eq!(offer.from, 3);
        assert!(offer.items.len() > 3);
        assert!(offer.items.iter().all(|c| c.label.starts_with("ui.")));
        let pad = offer
            .items
            .iter()
            .find(|c| c.label == "ui.fitPadding")
            .unwrap();
        assert!(!pad.detail.is_empty());
        assert!(!pad.doc.is_empty());
        // Case does not matter.
        assert!(settings_keys("\"UI.fitp")
            .unwrap()
            .items
            .iter()
            .any(|c| c.label == "ui.fitPadding"));
    }

    #[test]
    fn letters_without_a_dot_match_anywhere_in_the_key() {
        let offer = settings_keys("  \"fitpad").unwrap();
        assert_eq!(labels(&offer)[0], "ui.fitPadding");
    }

    #[test]
    fn nothing_for_an_empty_prefix_a_whole_key_or_a_value() {
        assert!(settings_keys("  \"").is_none());
        assert!(settings_keys("  \"ui.fitPadding").is_none());
        assert!(settings_keys("  \"theme\": \"ui.").is_none());
        assert!(settings_keys("  \"zzzzqq").is_none());
    }

    #[test]
    fn only_the_user_settings_file_has_a_completer() {
        let dir = crate::paths::config_dir();
        assert!(completer_for(&dir.join("settings.json").to_string_lossy()).is_some());
        assert!(completer_for(&dir.join("settings.default.json").to_string_lossy()).is_none());
        assert!(completer_for("/tmp/settings.json").is_none());
        assert!(completer_for("/tmp/other.json").is_none());
    }

    #[test]
    fn a_symlinked_path_to_the_config_folder_still_matches() {
        let base = std::env::temp_dir().join(format!("infiniterm-complete-{}", std::process::id()));
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, base.join("link")).unwrap();
        let through_link = base.join("link").join("settings.json");
        assert!(completer_in(&through_link.to_string_lossy(), &real).is_some());
        // Another folder with the same file name does not.
        let other = base.join("other");
        std::fs::create_dir_all(&other).unwrap();
        assert!(completer_in(&other.join("settings.json").to_string_lossy(), &real).is_none());
        let _ = std::fs::remove_dir_all(&base);
    }
}
