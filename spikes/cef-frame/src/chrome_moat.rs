//! Making a CEF browser pass Google's "secure browser" check.
//!
//! Google refuses sign-in from embedded Chromium ("This browser or app may
//! not be secure"). Ekin found the tells in ~/Code/glass (Electron) by
//! diffing against an Edge that signs in fine, and this is that fix ported:
//!
//! 1. The `Sec-CH-UA` / `Sec-CH-UA-Full-Version-List` request headers and
//!    `navigator.userAgentData` carry only the `Chromium` brand; a real
//!    Chrome adds `Google Chrome` at the same version. Instead of glass's
//!    two patches (a header rewrite plus a prototype patch in the page), CEF
//!    gets both from one DevTools call: `Emulation.setUserAgentOverride`
//!    with `userAgentMetadata` is what Chromium consults for the headers AND
//!    for `navigator.userAgentData`, so they cannot disagree.
//! 2. `window.chrome` is an empty object in embedded builds; Chrome and
//!    Edge carry `app`, `csi` and `loadTimes`. glass's document-start
//!    script is injected verbatim through `Page.addScriptToEvaluateOnNewDocument`.
//!
//! CEF's own UA string is already a plain Chrome one, so glass's third
//! fix (stripping the `Electron/` token) has no counterpart here.
//!
//! Everything is sent as raw CDP JSON through `send_dev_tools_message`
//! before the first navigation; the browser is created on `about:blank`
//! for that reason. Pure JSON builders here, tested; the sending is a
//! three-line function the spike calls.
use cef::{BrowserHost, ImplBrowserHost};

/// The version CEF reports, so every brand entry agrees with the UA string.
pub fn chromium_version() -> String {
    format!(
        "{}.{}.{}.{}",
        cef::sys::CHROME_VERSION_MAJOR,
        cef::sys::CHROME_VERSION_MINOR,
        cef::sys::CHROME_VERSION_BUILD,
        cef::sys::CHROME_VERSION_PATCH
    )
}

/// `Emulation.setUserAgentOverride` with the brand list a real Chrome sends.
/// The UA string is the one CEF already uses (major version only, the way
/// Chrome reduces it); the full version goes in `fullVersionList`.
pub fn ua_override_message(id: u32, full_version: &str) -> String {
    let major = full_version.split('.').next().unwrap_or("0");
    let ua = format!(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36"
    );
    // GREASE brand first, then Chromium and Google Chrome, the order Chrome
    // itself uses on this version; Google reads membership, not order.
    let brands = |v: &str| {
        format!(
            r#"[{{"brand":"Not)A;Brand","version":"8"}},{{"brand":"Chromium","version":"{v}"}},{{"brand":"Google Chrome","version":"{v}"}}]"#
        )
    };
    format!(
        r#"{{"id":{id},"method":"Emulation.setUserAgentOverride","params":{{"userAgent":"{ua}","userAgentMetadata":{{"brands":{},"fullVersionList":{},"platform":"macOS","platformVersion":"15.6.0","architecture":"arm","model":"","mobile":false,"bitness":"64","wow64":false}}}}}}"#,
        brands(major),
        brands(full_version).replace(r#""version":"8""#, r#""version":"8.0.0.0""#),
    )
}

/// glass's `src/preload/tab.ts` main-world patch, copied as it is: the
/// `window.chrome` members Edge carries, plus the brand patch kept as a
/// belt to the UA override's braces (harmless when the override already
/// put Google Chrome in the list, since it no-ops on lists that have it).
const DOCUMENT_START_JS: &str = r#"(() => {
  const addGoogleChrome = (list) => {
    if (!Array.isArray(list) || list.some((b) => b.brand === 'Google Chrome')) return list
    const chromium = list.find((b) => b.brand === 'Chromium')
    if (!chromium) return list
    return list.map((b) => ({ brand: b.brand, version: b.version })).concat([{ brand: 'Google Chrome', version: chromium.version }])
  }
  try {
    const uaData = navigator.userAgentData
    if (uaData) {
      const proto = Object.getPrototypeOf(uaData)
      const brandsGet = Object.getOwnPropertyDescriptor(proto, 'brands')?.get
      if (brandsGet) {
        Object.defineProperty(proto, 'brands', { configurable: true, get() { return addGoogleChrome(brandsGet.call(this)) } })
      }
      const ghev = proto.getHighEntropyValues
      if (typeof ghev === 'function') {
        Object.defineProperty(proto, 'getHighEntropyValues', { configurable: true, writable: true, value(hints) {
          return ghev.call(this, hints).then((res) => { if (Array.isArray(res.fullVersionList)) res.fullVersionList = addGoogleChrome(res.fullVersionList); return res })
        } })
      }
    }
  } catch {}
  try {
    const w = window
    if (!w.chrome || typeof w.chrome !== 'object') w.chrome = {}
    const c = w.chrome
    if (!c.app) c.app = { isInstalled: false, InstallState: { DISABLED: 'disabled', INSTALLED: 'installed', NOT_INSTALLED: 'not_installed' }, RunningState: { CANNOT_RUN: 'cannot_run', READY_TO_RUN: 'ready_to_run', RUNNING: 'running' }, getDetails() { return null }, getIsInstalled() { return false }, runningState() { return 'cannot_run' } }
    if (!c.csi) c.csi = function () { return { startE: Date.now(), onloadT: Date.now(), pageT: performance.now(), tran: 15 } }
    if (!c.loadTimes) c.loadTimes = function () { const t = Date.now() / 1000; return { requestTime: t, startLoadTime: t, commitLoadTime: t, finishDocumentLoadTime: t, finishLoadTime: t, firstPaintTime: t, firstPaintAfterLoadTime: 0, navigationType: 'Other', wasFetchedViaSpdy: true, wasNpnNegotiated: true, npnNegotiatedProtocol: 'h2', wasAlternateProtocolAvailable: false, connectionInfo: 'h2' } }
  } catch {}
})();"#;

/// `Page.addScriptToEvaluateOnNewDocument` carrying the patch above.
pub fn document_start_message(id: u32) -> String {
    format!(
        r#"{{"id":{id},"method":"Page.addScriptToEvaluateOnNewDocument","params":{{"source":{}}}}}"#,
        json_string(DOCUMENT_START_JS)
    )
}

/// Minimal JSON string quoting; the script has no control characters
/// beyond newlines and no non-ASCII, and the test says so.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Sends the three messages. `Page.enable` first: the document-start
/// script is registered by the page agent and is dropped on a disabled one.
pub fn apply(host: &BrowserHost) {
    let version = chromium_version();
    for (i, msg) in [
        r#"{"id":1,"method":"Page.enable"}"#.to_string(),
        ua_override_message(2, &version),
        document_start_message(3),
    ]
    .iter()
    .enumerate()
    {
        let ok = host.send_dev_tools_message(Some(msg.as_bytes()));
        eprintln!("[moat] message {} sent: {ok}", i + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ua_override_carries_google_chrome_at_the_chromium_version() {
        let msg = ua_override_message(2, "152.0.7977.83");
        assert!(msg.contains(r#""method":"Emulation.setUserAgentOverride""#));
        assert!(msg.contains(r#"{"brand":"Google Chrome","version":"152"}"#));
        assert!(msg.contains(r#"{"brand":"Google Chrome","version":"152.0.7977.83"}"#));
        assert!(msg.contains(r#"{"brand":"Chromium","version":"152.0.7977.83"}"#));
        assert!(msg.contains("Chrome/152.0.0.0 Safari/537.36"));
        // the GREASE entry has a full version in the full list, like Chrome
        assert!(msg.contains(r#"{"brand":"Not)A;Brand","version":"8.0.0.0"}"#));
    }

    #[test]
    fn document_start_script_is_valid_json_and_ascii() {
        assert!(DOCUMENT_START_JS.is_ascii());
        let msg = document_start_message(3);
        assert!(msg.starts_with(r#"{"id":3,"method":"Page.addScriptToEvaluateOnNewDocument""#));
        // round trip: unescape and compare
        let start = msg.find(r#""source":"#).unwrap() + r#""source":"#.len();
        let quoted = &msg[start..msg.len() - 2];
        let unescaped = quoted[1..quoted.len() - 1]
            .replace("\\n", "\n")
            .replace("\\\"", "\"")
            .replace("\\\\", "\\");
        assert_eq!(unescaped, DOCUMENT_START_JS);
    }

    #[test]
    fn json_string_escapes_what_json_requires() {
        assert_eq!(json_string("a\"b\\c\nd"), r#""a\"b\\c\nd""#);
        assert_eq!(json_string("\u{1}"), "\"\\u0001\"");
    }
}
