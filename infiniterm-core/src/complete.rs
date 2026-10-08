//! Completion for the editor's popup. A `Provider` answers "what could go
//! here?" for a whole text and a caret; the editor asks after every key and
//! draws the answer (`infiniterm-ui/src/editor_body.rs`).
//!
//! One provider so far: JSON Schema. A JSON or JSONC file that names a
//! schema (`"$schema"`, see `schema_store.rs`) gets its keys and values
//! completed from it. The editor's own `settings.json` needs no such line:
//! its schema is built here from the settings table (`settings_doc.rs`)
//! and the default config, so a new setting is offered as soon as it is
//! documented. No LSP: the keys are ours.
//!
//! Pure over text, so the rules are tested without an editor. A better
//! provider (buffer words, tree-sitter, an LSP client, YAML and TOML
//! modelines) can take the same place without a new popup.
use crate::config::{default_config, flatten};
use crate::fuzzy::fuzzy_match;
use crate::jsonctx::{context_at, Context, Slot};
use crate::schema::Schema;
use crate::schema_store::{resolve, schema_ref};
use crate::settings_doc::{doc, scalar};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// One thing the popup can insert.
#[derive(Clone, Debug, PartialEq)]
pub struct Completion {
    /// What the row shows first.
    pub label: String,
    /// What replaces the typed text when it is accepted: the label, or the
    /// label in quotes where a quote is still to be written.
    pub insert: String,
    /// Shown dimmed after the label: the default value or the type.
    pub detail: String,
    /// What it does, shown under the list while it is selected (the
    /// description's lines joined; the popup cuts it to its width).
    pub doc: String,
}

/// The popup's content: `items` replace the `typed` chars before the caret.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub typed: usize,
    pub items: Vec<Completion>,
}

/// Answers for a text and a caret (a char index).
pub trait Provider {
    fn complete(&self, doc: &str, caret: usize) -> Option<Offer>;
}

/// The most rows an offer carries; the popup shows fewer and scrolls.
pub const MAX_ITEMS: usize = 50;
/// With nothing typed yet, a list is offered only when it is this short:
/// a hundred settings are not worth a popup before the first letter.
const EMPTY_PREFIX_MAX: usize = 40;

/// The provider for a file, if it has one: the settings file gets the
/// built-in schema, any other `.json` or `.jsonc` file the schema it names.
/// Not `settings.default.json`, which is read-only.
pub fn provider_for(path: &str) -> Option<Box<dyn Provider>> {
    provider_in(path, &crate::paths::config_dir())
}

/// `provider_for` against an explicit config folder. The two paths are
/// compared as real paths: the app is handed `/private/tmp/...` for a file
/// in `/tmp/...` (a symlink), and a plain comparison missed the match.
fn provider_in(path: &str, config_dir: &Path) -> Option<Box<dyn Provider>> {
    let p = Path::new(path);
    let name = p.file_name()?.to_str()?;
    let dir = p.parent()?;
    let in_config = dir == config_dir
        || matches!(
            (std::fs::canonicalize(dir), std::fs::canonicalize(config_dir)),
            (Ok(a), Ok(b)) if a == b
        );
    if name == "settings.json" && in_config {
        return Some(Box::new(SchemaProvider {
            builtin: true,
            dir: dir.to_path_buf(),
        }));
    }
    let ext = p.extension()?.to_str()?;
    (name != "settings.default.json" && matches!(ext, "json" | "jsonc")).then(|| {
        Box::new(SchemaProvider {
            builtin: false,
            dir: dir.to_path_buf(),
        }) as Box<dyn Provider>
    })
}

struct SchemaProvider {
    /// The settings file: its schema is built in.
    builtin: bool,
    /// The file's folder, a relative `$schema` path is read from here.
    dir: PathBuf,
}

/// The schema of `settings.json`, built once from the documented settings.
/// Each flat dotted key is a property with its description and default; the
/// ones that take a fixed set of words carry it as an `enum`.
pub fn settings_schema() -> Arc<Schema> {
    static S: OnceLock<Arc<Schema>> = OnceLock::new();
    S.get_or_init(|| {
        let value = serde_json::to_value(default_config()).expect("config serialises");
        let flat = value.as_object().map(flatten).unwrap_or_default();
        let mut props = serde_json::Map::new();
        for (key, v) in &flat {
            let mut p = json!({ "default": v, "description": doc(key).join(" ") });
            if let Some((_, choices)) = SETTING_CHOICES.iter().find(|(k, _)| k == key) {
                p["enum"] = json!(choices);
            } else if v.is_boolean() {
                p["type"] = json!("boolean");
            }
            props.insert(key.clone(), p);
        }
        Arc::new(Schema::new(
            json!({ "type": "object", "properties": props }),
        ))
    })
    .clone()
}

/// Settings that take one of a few words. A test holds every entry to the
/// defaults: the key exists and its default is one of the choices.
pub const SETTING_CHOICES: &[(&str, &[&str])] = &[
    ("terminal.backend", &["daemon", "pty", "tmux"]),
    ("terminal.cursorStyle", &["block", "bar", "underline"]),
    ("editor.wrap", &["prose", "always", "never"]),
    ("ui.backgroundImageFit", &["cover", "contain"]),
    (
        "ui.cardLabelPosition",
        &["top right", "top left", "bottom right", "bottom left"],
    ),
    ("ui.fullscreen", &["cover", "native"]),
];

impl SchemaProvider {
    fn schema(&self, doc: &str) -> Option<Arc<Schema>> {
        if self.builtin {
            return Some(settings_schema());
        }
        let source = resolve(&schema_ref(doc)?, &self.dir)?;
        crate::schema_store::global().get(&source)
    }
}

impl Provider for SchemaProvider {
    fn complete(&self, doc: &str, caret: usize) -> Option<Offer> {
        let ctx = context_at(doc, caret)?;
        let schema = self.schema(doc)?;
        offer(&schema, &ctx)
    }
}

/// Keeps the candidates the typed text allows, best first. A prefix with a
/// dot (`ui.`, `ui.fit`) matches names that start with it, so `ui.` lists
/// the group; without a dot the letters match anywhere, in order (`fitpad`
/// finds `ui.fitPadding`), best score first.
fn filter(prefix: &str, items: Vec<Completion>) -> Vec<Completion> {
    if prefix.is_empty() {
        return if items.len() <= EMPTY_PREFIX_MAX {
            items
        } else {
            vec![]
        };
    }
    let lower = prefix.to_lowercase();
    let mut scored: Vec<(f64, Completion)> = items
        .into_iter()
        .filter_map(|c| {
            if prefix.contains('.') {
                c.label
                    .to_lowercase()
                    .starts_with(&lower)
                    .then_some((0., c))
            } else {
                fuzzy_match(prefix, &c.label).map(|m| (m.score, c))
            }
        })
        .collect();
    if !prefix.contains('.') {
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    }
    scored.into_iter().map(|(_, c)| c).collect()
}

fn offer(schema: &Schema, ctx: &Context) -> Option<Offer> {
    let nodes = schema.nodes_at(&ctx.path);
    if nodes.is_empty() {
        return None;
    }
    let items: Vec<Completion> = match ctx.slot {
        Slot::Key => schema
            .properties(&nodes)
            .into_iter()
            .filter(|p| !ctx.siblings.contains(&p.name))
            .map(|p| Completion {
                insert: if ctx.in_string {
                    p.name.clone()
                } else {
                    format!("\"{}\"", p.name)
                },
                label: p.name,
                detail: p.detail,
                doc: p.description,
            })
            .collect(),
        Slot::Value => schema
            .suggestions(&nodes)
            .into_iter()
            .filter_map(|s| {
                // Inside quotes only a string can be written; elsewhere a
                // string needs its quotes.
                let (label, insert) = match &s.value {
                    Value::String(t) if ctx.in_string => (t.clone(), t.clone()),
                    Value::String(t) => (t.clone(), serde_json::to_string(t).ok()?),
                    other if ctx.in_string => {
                        let _ = other;
                        return None;
                    }
                    other => (scalar(other), scalar(other)),
                };
                Some(Completion {
                    label,
                    insert,
                    detail: if s.is_default {
                        "default".into()
                    } else {
                        String::new()
                    },
                    doc: String::new(),
                })
            })
            .collect(),
    };
    let items: Vec<Completion> = filter(&ctx.prefix, items)
        .into_iter()
        .take(MAX_ITEMS)
        .collect();
    // Typing the whole word leaves nothing to offer.
    if items.len() == 1 && items[0].insert.trim_matches('"') == ctx.prefix {
        return None;
    }
    (!items.is_empty()).then_some(Offer {
        typed: ctx.typed,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(doc: &str) -> Option<Offer> {
        let ctx = context_at(doc, doc.chars().count())?;
        offer(&settings_schema(), &ctx)
    }

    fn labels(o: &Offer) -> Vec<&str> {
        o.items.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    fn a_dotted_prefix_lists_the_group_with_defaults_and_docs() {
        let o = settings("{\n  \"ui.").unwrap();
        assert_eq!(o.typed, 3);
        assert!(o.items.len() > 3);
        assert!(o
            .items
            .iter()
            .all(|c| c.label.starts_with("ui.") && c.insert == c.label));
        let pad = o.items.iter().find(|c| c.label == "ui.fitPadding").unwrap();
        assert!(!pad.detail.is_empty());
        assert!(!pad.doc.is_empty());
        assert!(settings("{\"UI.fitp")
            .unwrap()
            .items
            .iter()
            .any(|c| c.label == "ui.fitPadding"));
    }

    #[test]
    fn ui_typography_settings_are_completed_with_defaults_and_docs() {
        let o = settings("{\n  \"ui.font").unwrap();
        for key in [
            "ui.fontFamily",
            "ui.fontSize",
            "ui.fontWeight",
            "ui.fontWeightBold",
        ] {
            let item = o.items.iter().find(|item| item.label == key).unwrap();
            assert!(!item.detail.is_empty(), "{key} has no default");
            assert!(!item.doc.is_empty(), "{key} has no documentation");
        }
    }

    #[test]
    fn letters_without_a_dot_match_anywhere_in_the_key() {
        assert_eq!(labels(&settings("{ \"fitpad").unwrap())[0], "ui.fitPadding");
    }

    #[test]
    fn a_key_without_its_quote_is_inserted_with_both() {
        let o = settings("{ fitpad").unwrap();
        assert_eq!(o.items[0].insert, "\"ui.fitPadding\"");
        assert_eq!(o.typed, 6);
    }

    #[test]
    fn keys_already_in_the_object_are_not_offered_again() {
        let o = settings("{ \"ui.fitPadding\": 1, \"ui.fit").unwrap();
        assert!(!labels(&o).contains(&"ui.fitPadding"));
        assert!(labels(&o).contains(&"ui.fitMagnify"));
        // Alone, the one left is offered; with it taken there is nothing.
        assert!(settings("{ \"ui.fitPadding\": 1, \"ui.fitP").is_none());
    }

    #[test]
    fn nothing_for_a_long_list_with_no_prefix_a_whole_key_a_comment_or_nonsense() {
        assert!(settings("{ \"").is_none());
        assert!(settings("{ \"ui.fitPadding").is_none());
        assert!(settings("{ // \"ui.").is_none());
        assert!(settings("{ \"zzzzqq").is_none());
    }

    #[test]
    fn a_setting_with_choices_offers_them_and_marks_the_default() {
        let o = settings("{ \"ui.fullscreen\": ").unwrap();
        assert_eq!(labels(&o), vec!["cover", "native"]);
        assert_eq!(
            o.items[0].insert, "\"cover\"",
            "quotes where none is written yet"
        );
        assert_eq!(o.items[0].detail, "default");
        // Inside quotes the bare word goes in.
        let o = settings("{ \"ui.fullscreen\": \"na").unwrap();
        assert_eq!(labels(&o), vec!["native"]);
        assert_eq!(o.items[0].insert, "native");
        assert_eq!(o.typed, 2);
    }

    #[test]
    fn a_boolean_setting_offers_true_and_false_and_never_inside_quotes() {
        let o = settings("{ \"ui.showFps\": ").unwrap();
        assert_eq!(labels(&o), vec!["true", "false"]);
        assert_eq!(o.items[1].detail, "default");
        assert!(settings("{ \"ui.showFps\": \"").is_none());
        // A bare word narrows them.
        assert_eq!(
            labels(&settings("{ \"ui.showFps\": tr").unwrap()),
            vec!["true"]
        );
    }

    #[test]
    fn every_setting_choice_names_a_real_setting_and_includes_its_default() {
        let value = serde_json::to_value(default_config()).unwrap();
        let flat = flatten(value.as_object().unwrap());
        for (key, choices) in SETTING_CHOICES {
            let v = flat
                .get(*key)
                .unwrap_or_else(|| panic!("{key} is not a setting"));
            let default = v
                .as_str()
                .unwrap_or_else(|| panic!("{key} default is not a string"));
            assert!(
                choices.contains(&default),
                "{key}: default {default:?} not in {choices:?}"
            );
        }
    }

    #[test]
    fn a_schema_from_a_file_completes_nested_keys_and_values() {
        let schema = Schema::new(json!({
            "properties": {"server": {"properties": {
                "host": {"type": "string", "description": "Where."},
                "mode": {"enum": ["a", "b"]}
            }}}
        }));
        let at = |doc: &str| {
            let ctx = context_at(doc, doc.chars().count())?;
            offer(&schema, &ctx)
        };
        let o = at("{\"server\": {\"h").unwrap();
        assert_eq!(labels(&o), vec!["host"]);
        assert_eq!(o.items[0].doc, "Where.");
        assert_eq!(
            labels(&at("{\"server\": {\"mode\": ").unwrap()),
            vec!["a", "b"]
        );
        // A path the schema does not know gives nothing.
        assert!(at("{\"other\": {\"h").is_none());
    }

    #[test]
    fn which_files_have_a_provider() {
        let base = std::env::temp_dir().join(format!("infiniterm-complete-{}", std::process::id()));
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, base.join("link")).unwrap();
        let has = |p: &std::path::Path| provider_in(&p.to_string_lossy(), &real).is_some();
        assert!(has(&real.join("settings.json")));
        // The config folder through a symlink still matches.
        assert!(has(&base.join("link").join("settings.json")));
        // Any other JSON file may name its own schema; read-only defaults and other types do not.
        assert!(has(&base.join("other.json")));
        assert!(has(&base.join("x.jsonc")));
        assert!(!has(&real.join("settings.default.json")));
        assert!(!has(&base.join("notes.txt")));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_json_file_naming_a_local_schema_gets_its_keys_and_values() {
        let dir =
            std::env::temp_dir().join(format!("infiniterm-local-schema-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("app.schema.json"),
            r#"{"properties": {"port": {"type": "integer", "default": 80, "description": "Listen port."},
                "mode": {"enum": ["dev", "prod"]}}}"#,
        )
        .unwrap();
        let p = SchemaProvider {
            builtin: false,
            dir: dir.clone(),
        };
        let at = |doc: &str| p.complete(doc, doc.chars().count());
        let head = "{\n  \"$schema\": \"./app.schema.json\",\n  ";
        let o = at(&format!("{head}\"po")).unwrap();
        assert_eq!(labels(&o), vec!["port"]);
        assert_eq!(
            (o.items[0].detail.as_str(), o.items[0].doc.as_str()),
            ("80", "Listen port.")
        );
        assert_eq!(
            labels(&at(&format!("{head}\"mode\": \"")).unwrap()),
            vec!["dev", "prod"]
        );
        // The schema line itself is not completed: it names no property.
        assert!(at("{\n  \"$schema\": \"./app.sch").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_with_no_schema_line_offers_nothing() {
        let p = SchemaProvider {
            builtin: false,
            dir: std::env::temp_dir(),
        };
        assert!(p.complete("{ \"a", 4).is_none());
        assert!(
            p.complete("{ \"$schema\": \"http://x.org/s.json\", \"a", 40)
                .is_none(),
            "http is refused"
        );
    }
}
