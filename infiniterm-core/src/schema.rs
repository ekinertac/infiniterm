//! A small part of JSON Schema, enough to complete keys and values:
//! `properties`, `additionalProperties`, `items`, `description`, `default`,
//! `type`, `enum`, `const`, `$ref` into the same document, and `allOf`,
//! `anyOf` and `oneOf` read as one merged set of possibilities.
//!
//! Not supported, on purpose: references to other documents, `if`/`then`,
//! `patternProperties`, `dependencies`, and every rule that only judges a
//! value (`minimum`, `pattern`). Completion needs "what may go here", not
//! "is this valid".
//!
//! Pure over a `serde_json::Value`. `jsonctx.rs` finds the path to the
//! caret, `schema_store.rs` finds and loads the document, `complete.rs`
//! turns what is found here into a popup.
use crate::jsonc::parse_jsonc;
use crate::jsonctx::Seg;
use serde_json::Value;

/// How deep `$ref` and `allOf` are followed: a cycle in a schema must not
/// hang the editor.
const DEPTH: usize = 8;

/// A property a schema allows.
#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub name: String,
    /// The default shown dimmed, else the type.
    pub detail: String,
    pub description: String,
}

/// A value a schema suggests: an `enum` member, a `const`, `true`/`false`
/// for a boolean, or the `default`.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub value: Value,
    pub is_default: bool,
}

pub struct Schema {
    root: Value,
}

fn scalar(v: &Value) -> String {
    match v.as_f64() {
        Some(n) if v.is_number() => n.to_string(),
        _ => v.to_string(),
    }
}

impl Schema {
    pub fn new(root: Value) -> Schema {
        Schema { root }
    }

    /// From JSON text; comments and trailing commas are allowed. `None`
    /// when it is not an object.
    pub fn parse(text: &str) -> Option<Schema> {
        let root = parse_jsonc(text).ok()?;
        root.is_object().then(|| Schema::new(root))
    }

    /// Follows a `$ref` into this document (`#/definitions/x`,
    /// `#/$defs/x`); anything else is left as it is.
    fn deref<'a>(&'a self, node: &'a Value) -> &'a Value {
        let mut node = node;
        for _ in 0..DEPTH {
            let Some(target) = node
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|r| r.strip_prefix('#'))
                .and_then(|p| self.root.pointer(p))
            else {
                break;
            };
            node = target;
        }
        node
    }

    /// `node` and, merged in, whatever its `allOf`, `anyOf` and `oneOf`
    /// say, each followed through `$ref`.
    fn expand<'a>(&'a self, node: &'a Value, out: &mut Vec<&'a Value>, depth: usize) {
        let node = self.deref(node);
        out.push(node);
        if depth >= DEPTH {
            return;
        }
        for key in ["allOf", "anyOf", "oneOf"] {
            if let Some(members) = node.get(key).and_then(Value::as_array) {
                for m in members {
                    self.expand(m, out, depth + 1);
                }
            }
        }
    }

    /// The schemas that apply at `path`, from the root down.
    pub fn nodes_at(&self, path: &[Seg]) -> Vec<&Value> {
        let mut cur = vec![];
        self.expand(&self.root, &mut cur, 0);
        for seg in path {
            let mut next = vec![];
            for n in &cur {
                match seg {
                    Seg::Key(k) => {
                        if let Some(p) = n.get("properties").and_then(|p| p.get(k)) {
                            self.expand(p, &mut next, 0);
                        } else if let Some(ap) =
                            n.get("additionalProperties").filter(|v| v.is_object())
                        {
                            self.expand(ap, &mut next, 0);
                        }
                    }
                    Seg::Index(i) => match n.get("items") {
                        Some(items) if items.is_object() => self.expand(items, &mut next, 0),
                        Some(items) => {
                            if let Some(item) = items.get(*i) {
                                self.expand(item, &mut next, 0);
                            }
                        }
                        None => {
                            if let Some(item) = n.get("prefixItems").and_then(|p| p.get(*i)) {
                                self.expand(item, &mut next, 0);
                            }
                        }
                    },
                }
            }
            cur = next;
        }
        cur
    }

    /// Every property the nodes allow, first mention wins, in schema order.
    pub fn properties(&self, nodes: &[&Value]) -> Vec<Property> {
        let mut out: Vec<Property> = vec![];
        for n in nodes {
            let Some(props) = n.get("properties").and_then(Value::as_object) else {
                continue;
            };
            for (name, def) in props {
                if out.iter().any(|p| &p.name == name) {
                    continue;
                }
                let def = self.deref(def);
                let description = ["description", "markdownDescription", "title"]
                    .iter()
                    .find_map(|k| def.get(k).and_then(Value::as_str))
                    .unwrap_or("")
                    .to_string();
                let detail = match def.get("default") {
                    Some(d) => scalar(d),
                    None => match def.get("type") {
                        Some(Value::String(t)) => t.clone(),
                        Some(Value::Array(ts)) => ts
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join("|"),
                        _ => String::new(),
                    },
                };
                out.push(Property {
                    name: name.clone(),
                    detail,
                    description,
                });
            }
        }
        out
    }

    /// The values the nodes suggest: `enum` members and `const` first, then
    /// `true` and `false` for a boolean, then the `default`, none twice.
    pub fn suggestions(&self, nodes: &[&Value]) -> Vec<Suggestion> {
        let mut out: Vec<Suggestion> = vec![];
        let add = |value: Value, is_default: bool, out: &mut Vec<Suggestion>| match out
            .iter_mut()
            .find(|s| s.value == value)
        {
            Some(s) => s.is_default |= is_default,
            None => out.push(Suggestion { value, is_default }),
        };
        for n in nodes {
            if let Some(members) = n.get("enum").and_then(Value::as_array) {
                for m in members {
                    add(m.clone(), false, &mut out);
                }
            }
            if let Some(c) = n.get("const") {
                add(c.clone(), false, &mut out);
            }
            let boolean = match n.get("type") {
                Some(Value::String(t)) => t == "boolean",
                Some(Value::Array(ts)) => ts.iter().any(|t| t == "boolean"),
                _ => false,
            };
            if boolean {
                add(Value::Bool(true), false, &mut out);
                add(Value::Bool(false), false, &mut out);
            }
        }
        for n in nodes {
            if let Some(d) = n.get("default") {
                if matches!(d, Value::String(_) | Value::Number(_) | Value::Bool(_)) {
                    add(d.clone(), true, &mut out);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Schema {
        Schema::new(json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "The name."},
                "debug": {"type": "boolean", "default": false},
                "mode": {"enum": ["fast", "safe"], "default": "safe"},
                "server": {"$ref": "#/definitions/server"},
                "list": {"type": "array", "items": {"$ref": "#/$defs/item"}},
                "map": {"type": "object", "additionalProperties": {"type": "object", "properties": {"x": {"type": "number"}}}}
            },
            "definitions": {"server": {"type": "object", "properties": {
                "host": {"type": "string", "default": "localhost"},
                "tls": {"type": "boolean"}
            }}},
            "$defs": {"item": {"properties": {"id": {"type": ["integer", "null"]}}}}
        }))
    }

    fn names(s: &Schema, nodes: &[&Value]) -> Vec<String> {
        s.properties(nodes).into_iter().map(|p| p.name).collect()
    }

    #[test]
    fn the_root_lists_its_properties_with_defaults_or_types() {
        let s = schema();
        let nodes = s.nodes_at(&[]);
        let props = s.properties(&nodes);
        assert_eq!(props.len(), 6);
        let by = |n: &str| props.iter().find(|p| p.name == n).unwrap().clone();
        assert_eq!(by("name").detail, "string");
        assert_eq!(by("name").description, "The name.");
        assert_eq!(by("debug").detail, "false");
        assert_eq!(by("mode").detail, "\"safe\"");
    }

    #[test]
    fn a_ref_into_definitions_or_defs_is_followed() {
        let s = schema();
        let server = s.nodes_at(&[Seg::Key("server".into())]);
        assert_eq!(names(&s, &server), vec!["host", "tls"]);
        let item = s.nodes_at(&[Seg::Key("list".into()), Seg::Index(3)]);
        assert_eq!(names(&s, &item), vec!["id"]);
        assert_eq!(s.properties(&item)[0].detail, "integer|null");
    }

    #[test]
    fn additional_properties_describe_keys_the_schema_does_not_name() {
        let s = schema();
        let inner = s.nodes_at(&[Seg::Key("map".into()), Seg::Key("anything".into())]);
        assert_eq!(names(&s, &inner), vec!["x"]);
        assert!(s.nodes_at(&[Seg::Key("nope".into())]).is_empty());
    }

    #[test]
    fn all_of_any_of_and_one_of_are_merged() {
        let s = Schema::new(json!({
            "allOf": [{"properties": {"a": {}}}],
            "oneOf": [{"properties": {"b": {}}}, {"properties": {"a": {}, "c": {}}}]
        }));
        assert_eq!(names(&s, &s.nodes_at(&[])), vec!["a", "b", "c"]);
    }

    #[test]
    fn suggestions_are_enum_members_booleans_and_the_default_once_each() {
        let s = schema();
        let mode = s.nodes_at(&[Seg::Key("mode".into())]);
        let got = s.suggestions(&mode);
        assert_eq!(
            got,
            vec![
                Suggestion {
                    value: json!("fast"),
                    is_default: false
                },
                Suggestion {
                    value: json!("safe"),
                    is_default: true
                },
            ]
        );
        let debug = s.nodes_at(&[Seg::Key("debug".into())]);
        let got = s.suggestions(&debug);
        assert_eq!(got.len(), 2);
        assert!(got.iter().any(|g| g.value == json!(false) && g.is_default));
        assert!(s
            .suggestions(&s.nodes_at(&[Seg::Key("name".into())]))
            .is_empty());
    }

    #[test]
    fn a_reference_cycle_ends_instead_of_hanging() {
        let s = Schema::new(json!({
            "$ref": "#/definitions/a",
            "definitions": {"a": {"$ref": "#/definitions/b"}, "b": {"$ref": "#/definitions/a"}}
        }));
        let _ = s.nodes_at(&[Seg::Key("x".into())]);
        let n = s.nodes_at(&[]);
        assert!(s.properties(&n).is_empty());
    }

    #[test]
    fn parse_takes_comments_and_refuses_what_is_not_an_object() {
        assert!(Schema::parse("{ // note\n \"type\": \"object\", }").is_some());
        assert!(Schema::parse("[1]").is_none());
        assert!(Schema::parse("nope").is_none());
    }
}
