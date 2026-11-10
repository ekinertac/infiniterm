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

/// The absolute path a typed address could name on this Mac (#666): `~`
/// expanded, an absolute path as typed, a relative one joined to `base` (the
/// focused terminal's folder). `None` for a web address, an `about:`/`data:`
/// address, an empty line, or a relative name with no folder to read it
/// from. Nothing here looks at the disk: the caller asks whether the path
/// exists, and a path that does not is not an address.
pub fn local_path_candidate(input: &str, home: &str, base: Option<&str>) -> Option<String> {
    let t = input.trim();
    if t.is_empty() || t.contains("://") || t.starts_with("about:") || t.starts_with("data:") {
        return None;
    }
    if t == "~" {
        return Some(home.to_string());
    }
    if let Some(rest) = t.strip_prefix("~/") {
        return Some(format!("{}/{}", home.trim_end_matches('/'), rest));
    }
    if t.starts_with('/') || is_windows_path(t) {
        return Some(t.to_string());
    }
    base.map(|b| format!("{}/{}", b.trim_end_matches('/'), t))
}

/// A Windows path: a drive letter and a separator (`C:\x`, `C:/x`), or the
/// verbatim `\\?\` form `canonicalize` returns there. Told by its text, not
/// by the OS this runs on, so the rule is tested on every machine; no Unix
/// absolute path starts this way.
pub fn is_windows_path(path: &str) -> bool {
    let b = path.as_bytes();
    path.starts_with(r"\\?\")
        || (b.len() >= 3
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && matches!(b[2], b'\\' | b'/'))
}

/// `file://` and the path, percent-encoded: a space is `%20`, a non-ASCII
/// character is its UTF-8 bytes, and `/` stays a separator. A Windows path
/// becomes `file:///C:/dir/name`: the verbatim `\\?\` prefix dropped and
/// every `\` a `/`, because a browser reads `file://%5C%5C%3F%5CC...` as a
/// host that does not exist.
pub fn file_url(path: &str) -> String {
    let windows = is_windows_path(path);
    let path = if windows {
        let bare = path.strip_prefix(r"\\?\").unwrap_or(path);
        format!("/{}", bare.replace('\\', "/"))
    } else {
        path.to_string()
    };
    let mut out = String::from("file://");
    for byte in path.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(*byte as char)
            }
            // The drive's colon: a Windows file name cannot hold another.
            b':' if windows => out.push(':'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
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

    // #666: what a typed line could name on this Mac, before the disk is asked.
    #[test]
    fn a_typed_path_becomes_an_absolute_candidate() {
        let home = "/Users/me";
        let base = Some("/Users/me/Code/site");
        assert_eq!(
            local_path_candidate("/Users/me/site/index.html", home, None).as_deref(),
            Some("/Users/me/site/index.html")
        );
        assert_eq!(
            local_path_candidate("~/site/index.html", home, None).as_deref(),
            Some("/Users/me/site/index.html")
        );
        assert_eq!(
            local_path_candidate("~", home, None).as_deref(),
            Some("/Users/me")
        );
        // A relative name needs the focused terminal's folder.
        assert_eq!(local_path_candidate("index.html", home, None), None);
        assert_eq!(
            local_path_candidate("index.html", home, base).as_deref(),
            Some("/Users/me/Code/site/index.html")
        );
        assert_eq!(
            local_path_candidate("my page.html", home, base).as_deref(),
            Some("/Users/me/Code/site/my page.html")
        );
        // Web and special addresses are not paths.
        for no in [
            "https://example.com/a",
            "file:///tmp/x",
            "about:blank",
            "data:x",
            "",
            "  ",
        ] {
            assert_eq!(local_path_candidate(no, home, base), None, "{no:?}");
        }
    }

    #[test]
    fn a_file_address_encodes_what_a_url_cannot_carry() {
        assert_eq!(
            file_url("/Users/me/a b/c.html"),
            "file:///Users/me/a%20b/c.html"
        );
        assert_eq!(file_url("/tmp/é.html"), "file:///tmp/%C3%A9.html");
        assert_eq!(file_url("/tmp/a#b?c"), "file:///tmp/a%23b%3Fc");
    }

    // The Windows form is told by its text, so this runs on the Mac too.
    #[test]
    fn a_windows_path_is_a_file_address_with_a_drive_and_forward_slashes() {
        assert_eq!(
            file_url(r"C:\Users\PC\a b\c.html"),
            "file:///C:/Users/PC/a%20b/c.html"
        );
        // What `canonicalize` returns there, and a mixed join onto it.
        assert_eq!(
            file_url(r"\\?\C:\Users\PC\Temp\x/index.html"),
            "file:///C:/Users/PC/Temp/x/index.html"
        );
        assert_eq!(file_url("D:/dir/é.html"), "file:///D:/dir/%C3%A9.html");
        // A Unix path with a backslash or a colon in a name is not one.
        assert!(!is_windows_path("/tmp/a:b"));
        assert!(!is_windows_path("relative\\path"));
        assert_eq!(file_url("/tmp/a:b"), "file:///tmp/a%3Ab");
    }

    #[test]
    fn a_typed_windows_path_is_an_absolute_candidate() {
        assert_eq!(
            local_path_candidate(r"C:\site\index.html", "C:/Users/PC", Some("C:/work")).as_deref(),
            Some(r"C:\site\index.html")
        );
        assert_eq!(
            local_path_candidate("site/index.html", "C:/Users/PC", Some("C:/work")).as_deref(),
            Some("C:/work/site/index.html")
        );
    }

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
