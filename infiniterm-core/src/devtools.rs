//! The pure half of DevTools in a card: which port CEF listens on for the
//! debugging protocol, the origin Chromium must allow for our own frontend,
//! the address of the frontend page for one browser, and reading a page's
//! target id out of `Page.getFrameTree`'s reply.
//!
//! Why a port at all: CEF's `ShowDevTools` refuses windowless rendering and
//! opens a native window outside the canvas (spec
//! docs/superpowers/specs/2026-10-09-browser-extensions-cef-fork-design.md,
//! DevTools). The remote debugging port serves Chromium's own DevTools
//! frontend, which an ordinary browser card can show.
//!
//! Called by: `infiniterm-browser/src/process.rs` (the port and the switch,
//! before CEF starts), `infiniterm-browser/src/surface.rs` (the target id),
//! `infiniterm-ui/src/browsers.rs` (the card that shows the frontend).
//! Related: `config.rs` (`browser.devtools`).
use serde_json::Value;
use std::net::TcpListener;

/// The page served by the debugging port that is the full DevTools without
/// a copy of the page beside it (`inspector.html` adds a live view of the
/// page, which the page's own card already is).
const FRONTEND_PATH: &str = "/devtools/devtools_app.html";

/// A free loopback port, picked fresh at every launch so nothing else can
/// count on it. The probe socket is closed before CEF binds the port, so
/// another program could take it in between; CEF then fails to listen, the
/// browser still works and the DevTools command says it has no port.
// shortcut: a bind-and-release race, fine for a port nobody else is hunting.
pub fn free_port() -> Option<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    // CEF accepts 1024 to 65535 only.
    (port >= 1024).then_some(port)
}

/// Whether the settings text turns DevTools on (`browser.devtools`), read
/// before the model exists because CEF starts first. A missing or broken
/// file means the default.
pub fn wanted(settings_text: &str) -> bool {
    let value = crate::jsonc::parse_jsonc(settings_text).unwrap_or(Value::Null);
    crate::config::merge_config(&value).browser.devtools
}

/// The value for `--remote-allow-origins`: only our own frontend's origin,
/// so a web page cannot attach to the port (Chromium rejects its WebSocket).
pub fn allow_origin(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// The address a browser card opens to show DevTools for the page whose
/// target id is `target`.
pub fn frontend_url(port: u16, target: &str) -> String {
    format!("http://127.0.0.1:{port}{FRONTEND_PATH}?ws=127.0.0.1:{port}/devtools/page/{target}")
}

/// A page's DevTools target id: the id of its main frame, which
/// `Page.getFrameTree` answers as `{"frameTree":{"frame":{"id":"..."}}}`.
/// CEF's own frame identifier is a different value, so the id has to be
/// asked for. `None` for anything that is not that reply, or an empty id.
pub fn target_id_from_frame_tree(json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(json).ok()?;
    let id = value.get("frameTree")?.get("frame")?.get("id")?.as_str()?;
    (!id.is_empty()).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devtools_is_on_unless_the_settings_switch_it_off() {
        assert!(wanted(""), "no file");
        assert!(wanted("{ not json"), "a broken file");
        assert!(wanted(r#"{"browser.zoom": 1}"#));
        assert!(!wanted(r#"{"browser.devtools": false}"#), "flat key");
        assert!(!wanted(r#"{"browser": {"devtools": false}}"#), "nested key");
        assert!(wanted(r#"{"browser.devtools": true}"#));
    }

    #[test]
    fn a_free_port_is_one_cef_accepts() {
        let port = free_port().expect("the loopback interface is there");
        assert!(port >= 1024);
    }

    #[test]
    fn the_allowed_origin_is_the_frontend_s_own() {
        assert_eq!(allow_origin(9333), "http://127.0.0.1:9333");
    }

    #[test]
    fn the_frontend_url_names_the_port_and_the_target() {
        assert_eq!(
            frontend_url(9333, "ABC123"),
            "http://127.0.0.1:9333/devtools/devtools_app.html?ws=127.0.0.1:9333/devtools/page/ABC123"
        );
    }

    #[test]
    fn the_target_id_is_the_main_frame_s_id() {
        let reply = r#"{"frameTree":{"frame":{"id":"421181BF071812CC11955978B1A601A8","loaderId":"x","url":"https://example.com/"}}}"#;
        assert_eq!(
            target_id_from_frame_tree(reply).as_deref(),
            Some("421181BF071812CC11955978B1A601A8")
        );
    }

    #[test]
    fn anything_else_has_no_target_id() {
        for bad in [
            "",
            "not json",
            "{}",
            r#"{"frameTree":{}}"#,
            r#"{"frameTree":{"frame":{}}}"#,
            r#"{"frameTree":{"frame":{"id":""}}}"#,
            r#"{"frameTree":{"frame":{"id":7}}}"#,
        ] {
            assert_eq!(target_id_from_frame_tree(bad), None, "{bad}");
        }
    }
}
