# CEF frames inside gpui (2026-09-14)

Question: can a CEF off-screen browser be drawn as a texture inside a gpui window, zoomed, with the mouse forwarded back, while the Claude in Chrome extension keeps working? Yes on all four, on gpui 0.2.2 (crates.io) and cef 152.3.0, software `on_paint` path.

## Verified

1. Wikipedia renders in the gpui window through `on_paint` -> `RenderImage` -> `img()`. CEF hands BGRA and gpui keeps images as BGRA, so the frame is a memcpy: 0.7 to 1.5 ms for 1024x768 at scale 1, measured in the status bar. Extrapolated, a 1080p card is about 3 ms per changed frame on the M4; a 4K card would want `on_accelerated_paint` (IOSurface), which cef-rs supports but gpui has no public texture path for.
2. `=` / `-` / `0` change the zoom and the img is drawn at `view * zoom`; the page stays sharp because the source is the same buffer scaled by the GPU (upscaled, so at 1.56 it is a bit soft; the real card would ask CEF for a higher device scale instead).
3. A click on the "Talk" link at zoom 1.56 opened the Talk page: window position divided by zoom is the page position CEF wants. Scroll wheel is wired the same way but not exercised (no tool to send wheel events remotely).
4. The extension loads (`--load-extension`, same profile as `cef-extension`), Claude Code lists the window as a connected browser and drove a tab in it. That tab appeared as a separate native window, because a browser the extension creates goes through CEF's default life-span handling; the real card has to take `on_before_popup` and the `chrome.windows` path and give each a card.

## Google sign-in (the moat, 2026-09-14)

Google refuses embedded Chromium ("This browser or app may not be secure"). Ekin found the two tells in ~/Code/glass by diffing against Edge, and `src/chrome_moat.rs` is that fix for CEF, verified end to end: he signed in to his Google account inside this window and landed on the account page.

1. The `Sec-CH-UA` headers and `navigator.userAgentData` lacked the `Google Chrome` brand. One DevTools message, `Emulation.setUserAgentOverride` with `userAgentMetadata`, fixes both sides at once, since Chromium reads the same metadata for the headers and for the JS object. Checked on httpbin.org/headers and with a probe run on that page: `"Google Chrome";v="152"` in the header, in `brands` and in `fullVersionList`.
2. `window.chrome` lacked `app`, `csi`, `loadTimes`. glass's document-start script, copied verbatim, goes in through `Page.addScriptToEvaluateOnNewDocument` after `Page.enable`.

Both are sent before the first navigation, so the browser is created on `about:blank` and then told to load. `userAgentData` only exists in secure contexts, which is why the probe runs on an https page and not a `data:` URL. Cmd+V had to be forwarded (`frame.paste()`) before a 32-character password could go in; typed keys are forwarded as CHAR events, the editing keys as RAWKEYDOWN/KEYUP with Windows virtual key codes.

## Traps

- `isHandlingSendEvent`: CEF requires NSApplication to implement CefAppProtocol and aborts with "unrecognized selector" about ten seconds in (when the extension opened its window). gpui owns the NSApplication subclass, so `cef_app_protocol::install` adds the two methods at runtime with the objc crate. It does not yet wrap `sendEvent:` to set the flag the way cefsimple does; if nested-run-loop bugs show up (modal dialogs, drag and drop), that is the first thing to add.
- Both crates glob-export `App`, `Window`, `Point`, `MouseEvent`: import gpui by name.
- The browser must be created inside `open_window`'s builder: CEF asks for the device scale factor at creation and only the window knows it.
- Two processes on one cache path: Chrome's singleton makes the second one hand off and exit ("Opening in existing browser session", `initialize` returns 0). Kill cefclient first and remove `profile/Singleton*` if the previous run was killed hard.
- gpui does not draw while the Mac is locked; a window created behind the lock screen drew once and never again even after the unlock (the occlusion state never changed). Restarting the app fixed it. Not a spike concern, but explains an afternoon.
- Launch through `open target/bundle/cef-frame.app`: a binary started from a shell is not activated by LaunchServices and gpui's key events never arrive.

## How to run

```
cargo build
cargo run --manifest-path <cef-rs checkout>/Cargo.toml -p cef --bin bundle-cef-app -- cef-frame -o target/bundle
open --stderr "$PWD/run.log" --stdout "$PWD/run.log" target/bundle/cef-frame.app
```

`bundle-cef-app` lives in the cef-rs repo (dev branch) and builds the `.app` with the framework and the five helper bundles from `[package.metadata.cef.bundle]`. `.cargo/config.toml` points `CEF_PATH` at the shared CEF download (`export-cef-dir`, ~300 MB, in `~/.local/share/cef`).

The code of this spike was deleted on 2026-09-16 once its replacement was in the crates (`infiniterm-browser`, `infiniterm-term`, `infiniterm-ui`); `git log -- spikes/cef-frame` has it.
