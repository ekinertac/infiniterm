//! Is what was typed an address or a search? The one question the omnibox
//! asks before anything else, and the answer the Enter key acts on.
//!
//! Called by `omni::providers`, by `omni::rank`, and by the two URL prompts
//! that predate the omnibox through `Model::normalise_url`. Related:
//! omni/engines.rs, which answers the same question for a scoped search.
//!
//! The rules are Chrome's, with one that matters here: a local address is
//! http. A browser card sent to https://localhost:8087 gets a TLS error,
//! which is exactly what happened to the screencast's demo server.

/// The default search engine. `%s` is replaced by the encoded query.
pub const SEARCH_TEMPLATE: &str = "https://www.google.com/search?q=%s";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Address {
    Navigate(String),
    Search(String),
}

/// Hosts that are always local, and so always http.
fn is_local(host: &str) -> bool {
    let name = host.split(':').next().unwrap_or("");
    name == "localhost"
        || name == "0.0.0.0"
        || name.starts_with("127.")
        || name.ends_with(".localhost")
}

pub fn parse_address(input: &str) -> Option<Address> {
    let t = input.trim();
    if t.is_empty() {
        return None;
    }
    // A scheme and nothing else is what the old prompt's placeholder left
    // behind; it is not an address.
    if let Some((_, rest)) = t.split_once("://") {
        if rest.is_empty() {
            return None;
        }
        return Some(Address::Navigate(t.to_string()));
    }
    if t.starts_with("about:") || t.starts_with("data:") {
        return Some(Address::Navigate(t.to_string()));
    }
    if t.contains(char::is_whitespace) {
        return Some(Address::Search(t.to_string()));
    }
    let host = t.split(['/', '?', '#']).next().unwrap_or(t);
    if is_local(host) {
        return Some(Address::Navigate(format!("http://{t}")));
    }
    // A dot with at least two characters after it is a host, not prose.
    if let Some((_, tld)) = host.rsplit_once('.') {
        if tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Some(Address::Navigate(format!("https://{t}")));
        }
    }
    Some(Address::Search(t.to_string()))
}

/// Percent-encodes everything a query string must not carry raw.
pub fn search_url(template: &str, query: &str) -> String {
    let mut encoded = String::new();
    for byte in query.trim().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    template.replace("%s", &encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scheme_is_kept_as_typed() {
        assert_eq!(
            parse_address("https://example.com/a?b=c"),
            Some(Address::Navigate("https://example.com/a?b=c".into()))
        );
        assert_eq!(
            parse_address("file:///tmp/x"),
            Some(Address::Navigate("file:///tmp/x".into()))
        );
    }

    // The screencast's browser card went to https://localhost:8087 and got a
    // TLS error: a local server is http, and guessing https for it is wrong.
    #[test]
    fn localhost_and_bare_addresses_are_http() {
        assert_eq!(
            parse_address("localhost:8087"),
            Some(Address::Navigate("http://localhost:8087".into()))
        );
        assert_eq!(
            parse_address("127.0.0.1:3000/x"),
            Some(Address::Navigate("http://127.0.0.1:3000/x".into()))
        );
    }

    #[test]
    fn a_bare_host_gets_https() {
        assert_eq!(
            parse_address("news.ycombinator.com/newest"),
            Some(Address::Navigate(
                "https://news.ycombinator.com/newest".into()
            ))
        );
    }

    #[test]
    fn anything_else_is_a_search() {
        assert_eq!(
            parse_address("rust hashmap entry"),
            Some(Address::Search("rust hashmap entry".into()))
        );
        // A dotted word with a space in it is a sentence, not a host.
        assert_eq!(
            parse_address("what is rust.lang"),
            Some(Address::Search("what is rust.lang".into()))
        );
        assert_eq!(parse_address("   "), None);
        assert_eq!(parse_address("https://"), None);
    }

    #[test]
    fn a_search_url_is_percent_encoded() {
        assert_eq!(
            search_url(SEARCH_TEMPLATE, "a b&c"),
            "https://www.google.com/search?q=a%20b%26c"
        );
    }
}
