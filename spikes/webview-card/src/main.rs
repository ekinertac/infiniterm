// Spike for #761: an OS webview (WKWebView through wry) as a child of the
// gpui window, placed at a card's rect on a zoomable canvas. It answers the
// questions in docs/agent/webview-spike.md: z-order against gpui's Metal
// layer, following zoom, covering and dimming, keys, page features, cost.
//
// The app is a tiny canvas: gpui-drawn "plain" cards and "web" cards, a zoom
// and a pan, all in the spike's own state (nothing of infiniterm-core). It is
// driven by a command file (cmd.rs), never by the keyboard or mouse, and it
// logs what it learned to `$WV_DIR/out.log`.
//
// Environment: WV_DIR (command and log directory, required), WV_STORE
// (`default`, `incognito`, or a seed string for a data store identifier),
// WV_UA (user agent override), WV_QUIT_AFTER_MS.
// macOS only. The product browser path is not touched.
// Related: cmd.rs (the command language), ../../docs/agent/webview-spike.md.

mod cmd;
mod gesture;

#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "macos")]
fn main() {
    mac::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("the webview spike runs on macOS only; see docs/agent/webview-spike.md for the other systems");
}
