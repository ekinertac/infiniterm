# The three spikes on one canvas (2026-09-14)

Question: do terminal cards and browser cards share one world under the Tauri app's zoom commands without the numbers from the earlier spikes degrading? Yes. Twelve `zsh -l` terminals and three CEF browsers (Hacker News, Wikipedia, Zed), the viewport starting at fit-all, `Cmd+1` fitting the focused card and `Cmd+2` fitting all with the app's own easing and durations. Ekin's verdict on the remote screenshots: "it works perfectly".

## What it settled

1. `src/viewport.rs` is `zoomActions.ts` and `zoomAnimation.ts` ported as pure functions with eleven tests: `fit_rect` (FIT_PADDING 48, MAX_FIT_SCALE 1, scale clamped to 0.05..4), `centre_on`, `fit_frame` (centre interpolated linearly, scale geometrically, ease-out-cubic; FIT 240 ms, ZOOM 130 ms), `anchored_viewport` for a wheel zoom about the cursor. It moves into `infiniterm-core` unchanged.
2. A card body is one call, `paint(bounds, scale)`, whether the body is a shaped grid or a CEF texture. That is the trait the mapping asks `infiniterm-ui` to define, and nothing in the viewport or focus code knows which it is.
3. Input routing as the reference app has it: Cmd chords go to the canvas (`Cmd+=` `Cmd+-` zoom by 1.2, `Cmd+0` actual size, `Cmd+1` `Cmd+2` fits, `Cmd+scroll` anchored zoom, `Cmd+drag` a pan only after `DRAG_SLOP` 4 px, middle-drag at once); everything else to the focused card, `Cmd+V/C/X/A/Z` included when the card is a browser. Bare scroll goes to the card under the cursor.
4. Terminals drain a 256 KiB budget per frame before painting, as in term-zoom; browsers paint the last `on_paint` frame. The CEF message pump is a gpui foreground task calling `do_message_loop_work` every 4 ms.

## Not done here, for the real card

- Zoom above 1.0 upscales the CEF texture. The card should raise CEF's device scale factor instead (the Tauri app's WKWebView card re-lays out per zoom and has to fight it; the port lays out at a fixed size so there is no such fight).
- Popups and extension-created windows still open as native windows.
- Key encoding for the terminal is the shell subset; the browser's is ten keys plus text.

## How to run

```
cargo build --release
cargo run --manifest-path ~/Code/cef-rs/Cargo.toml -p cef --bin bundle-cef-app -- canvas -o target/bundle
pkill -9 -f "MacOS/canvas"; rm -f ../cef-extension/profile/Singleton*
open --stderr "$PWD/run.log" --stdout "$PWD/run.log" target/bundle/canvas.app
osascript -e 'tell application "canvas" to activate'
```

Shares `../cef-extension/profile` and `../cef-extension/extension`. `run.log` gets the moat messages and an fps line.
