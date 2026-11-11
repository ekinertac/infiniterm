//! Links that belong to another app (#668): `slack://`, `zoommtg:`, `mailto:`,
//! `tel:`, `spotify:`, `notion://`. A browser card cannot show them; Chrome
//! and Safari hand them to the app macOS has registered for the scheme, and a
//! card that does not ends on an error page ("might be temporarily down").
//! Slack's magic login is the usual case: the web page signs you in, then
//! sends the browser to a `slack://` address.
//!
//! This file is the decision and the words for it, with no CEF and no gpui:
//! which addresses are app links, what the prompt says, and how one is
//! opened. The browser crate queues each one (`Shared::external_links`), the
//! ui asks first (`Pending::OpenAppLink`, because a page may redirect to an
//! app without a click, as Chrome's "Open Slack?" guards), and `links_fs::
//! open_app_link` hands it to macOS only after a yes.
//!
//! Related: infiniterm-browser/src/surface.rs (the protocol handler),
//! links_fs.rs (opening), model/cards_cmd.rs (the answer).

/// Schemes a browser card shows itself, or that are not another app's: a
/// link with one of these is never an app link. `infiniterm` is the card's own
/// page scheme (`app_protocol`), not an app to open.
const IN_CARD: &[&str] = &[
    "http",
    "https",
    "file",
    "about",
    "data",
    "blob",
    "javascript",
    "ws",
    "wss",
    "ftp",
    "chrome",
    "chrome-extension",
    "chrome-error",
    "chrome-untrusted",
    "devtools",
    "view-source",
    "infiniterm",
];

/// The scheme of `url` in lower case, if it has one: a letter, then letters,
/// digits, `+`, `-` or `.`, then `:`. `None` for a path, a bare word or text
/// that only has a colon in it (`localhost:3000` has the scheme `localhost`,
/// as a browser reads it too).
pub fn scheme(url: &str) -> Option<String> {
    let (head, _) = url.split_once(':')?;
    let mut chars = head.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic()
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    Some(head.to_ascii_lowercase())
}

/// Whether `url` is for another app: it has a scheme that no browser card
/// shows. No whitespace, so one line is one link.
pub fn is_app_link(url: &str) -> bool {
    !url.chars().any(char::is_whitespace)
        && scheme(url).is_some_and(|s| !IN_CARD.contains(&s.as_str()))
}

/// The question the prompt asks: the scheme and the start of the address, so
/// a person sees where a page is sending them. Long magic-login tokens are
/// cut.
pub fn question(url: &str) -> String {
    const MAX: usize = 60;
    let shown: String = if url.chars().count() > MAX {
        let cut: String = url.chars().take(MAX).collect();
        format!("{cut}…")
    } else {
        url.to_string()
    };
    format!("open {shown} in its app?")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_schemes_are_app_links_and_web_and_card_schemes_are_not() {
        for yes in [
            "slack://T03NP0LMH7D/magic-login/1?id=2",
            "zoommtg://zoom.us/join?confno=1",
            "mailto:me@example.com",
            "tel:+905551112233",
            "spotify:track:abc",
            "notion://www.notion.so/x",
            "MAILTO:Me@Example.com",
            "x-apple.systempreferences:com.apple.preference",
        ] {
            assert!(is_app_link(yes), "{yes}");
        }
        for no in [
            "https://example.com/a",
            "http://localhost:3000",
            "file:///Users/me/a.html",
            "about:blank",
            "data:text/html,hi",
            "blob:https://x/1",
            "javascript:void(0)",
            "view-source:https://x",
            "chrome://version",
            "infiniterm://app/guide",
            "/Users/me/a.html",
            "example.com",
            "",
            "slack://has space",
            "9slack://x",
        ] {
            assert!(!is_app_link(no), "{no}");
        }
    }

    #[test]
    fn the_scheme_is_the_part_before_the_first_colon_in_lower_case() {
        assert_eq!(scheme("Slack://x").as_deref(), Some("slack"));
        assert_eq!(scheme("x-y.z+1:rest").as_deref(), Some("x-y.z+1"));
        assert_eq!(scheme("no scheme"), None);
        assert_eq!(scheme("-bad:x"), None);
        assert_eq!(scheme(":x"), None);
    }

    #[test]
    fn the_question_names_the_address_and_cuts_a_long_token() {
        assert_eq!(question("mailto:a@b.c"), "open mailto:a@b.c in its app?");
        let long = format!("slack://T1/magic-login/{}", "x".repeat(200));
        let q = question(&long);
        assert!(q.starts_with("open slack://T1/magic-login/"), "{q}");
        assert!(q.ends_with("… in its app?"), "{q}");
        assert!(q.chars().count() < 100, "{}", q.chars().count());
    }
}
