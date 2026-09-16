//! The omnibox's one network call: Google's suggest endpoint, and the pure
//! parser for what comes back. The FETCH is not here. It leaves the model as
//! an Effect and the ui runs `curl`, the way this app already runs git, ps,
//! lsof and open, rather than taking an HTTP dependency for one endpoint.
//!
//! Off unless `browser.suggestions` is true. The README's footprint section
//! says the app reaches the network only through browser cards, and that
//! stays true for anyone who leaves this alone.
//!
//! Related: omni/rank.rs (which takes the parsed strings), the Effect in
//! model/mod.rs, the runner in infiniterm-ui/src/runtime.rs.
use crate::omni::address::search_url;
use serde_json::Value;

/// The classic endpoint. `client=firefox` answers with a compact array:
/// ["<typed>", ["suggestion", ...], ...].
pub const SUGGEST_ENDPOINT: &str = "https://suggestqueries.google.com/complete/search";
/// Passed to curl as --max-time. Longer than this and the answer is useless,
/// because the typing has moved on.
pub const SUGGEST_TIMEOUT_S: &str = "1.5";

pub fn suggest_url(q: &str) -> String {
    search_url(&format!("{SUGGEST_ENDPOINT}?client=firefox&q=%s"), q)
}

/// Never panics: anything unexpected is no suggestions. This is a network
/// response, and one day it will be an error page.
pub fn parse_suggest(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return vec![];
    };
    let Some(list) = value.get(1).and_then(Value::as_array) else {
        return vec![];
    };
    list.iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compact_shape_is_the_second_element() {
        let body = r#"["rust",["rust book","rust playground","rust analyzer"],[],[]]"#;
        assert_eq!(
            parse_suggest(body),
            vec!["rust book", "rust playground", "rust analyzer"]
        );
    }

    #[test]
    fn every_broken_shape_is_empty() {
        for body in [
            "",
            "null",
            "{}",
            "[]",
            r#"["rust"]"#,
            r#"["rust",null]"#,
            r#"["rust",[1,2]]"#,
            "<html>",
        ] {
            assert!(parse_suggest(body).is_empty(), "{body}");
        }
    }

    #[test]
    fn the_query_is_encoded_into_the_endpoint() {
        assert!(suggest_url("a b").ends_with("q=a%20b"));
        assert!(suggest_url("x").starts_with(SUGGEST_ENDPOINT));
    }
}
