# Webview spike (#761): an OS webview as a browser card

What this covers: the answer to one question. Can a browser card be a WKWebView (through the `wry` crate) inside the gpui window instead of CEF, and what breaks? The spike code is `spikes/webview-card/`. The product browser path (`infiniterm-browser`, `browser_body.rs`, `browsers.rs`, omnibox) is not touched.

Read it before deciding on #761. Related: `browser-and-pages.md` (what the CEF browser card does today), `traps-code.md`.

Date: 2026-11-16. Machine: Mac mini M4, macOS 15 (Darwin 24.6), display scale 1 (NOT Retina). wry 0.57.2, gpui 0.2.2 (the app's own lockfile).

## Verdict in short

| # | Question | Verdict |
|---|---|---|
| 1 | Embed a webview at a card's rect | YES |
| 2 | Follow canvas zoom and pan | PARTLY (good from 0.5x up, not below) |
| 3 | Overlap, dim, ghost | PARTLY (works with native helper views, three visible compromises) |
| 4 | Keys, focus, wheel | PARTLY (mechanism found, real keyboard and mouse not tested) |
| 5 | Page features | PARTLY (sign-in page, video, popups, app links, find, downloads work; DevTools docks and breaks the frame; no Widevine) |
| 6 | Cost | YES (much lighter than CEF) |
| 7 | Windows and Linux | Read from docs only: Windows PARTLY, Linux X11 NO without patching gpui, Wayland NO |
| 8 | Login sessions | YES (with one trap: the store is not in our data dir) |

Can it replace CEF on macOS? Yes for ordinary browsing, with a snapshot for far zoom and covered cards. It is not equal: no Widevine, DevTools only docked, Claude in Chrome gone (already accepted in #761).

## How the spike works

One binary, `spikes/webview-card/`. It opens a gpui window with a tiny canvas (plain gpui cards and web cards, a zoom and a pan). Nothing is driven by keyboard or mouse. A 16 ms timer reads new lines from `$WV_DIR/cmd`, and everything it learns goes to `$WV_DIR/out.log`. The commands are in `src/cmd.rs` (`web`, `plain`, `zoom`, `anim`, `cover`, `dim`, `snap`, `eval`, `hit`, `click`, `key`, `find`, `devtools`, `info`, `windows`).

```
cd spikes/webview-card && cargo build
WV_DIR=/tmp/wv WV_STORE=seedA target/debug/webview-card &      # then: echo "web a 40 40 500 380 http://127.0.0.1:8765/state.html" >> /tmp/wv/cmd
```

For the download link make the file first: `head -c 100000 /dev/urandom > fixtures/file.bin` (not committed). `fixtures/state.html` is the test page (cookies, localStorage, IndexedDB, popup link, download link, app link). It is served from `python3 -m http.server 8765` in `fixtures/`, because `file://` keeps no cookies.

`click` and `key` build NSEvents and give them to `NSApp sendEvent:` inside the spike process. They never go through the HID tap, so they move nothing on the Mac. They do not behave exactly like real input (see question 4).

## 1. Embed: YES

gpui's `Window` implements `HasWindowHandle`. On macOS it returns the NSView of the gpui view (`AppKitWindowHandle`). `WebViewBuilder::build_as_child(&window)` adds the WKWebView as a subview of that view. Note that gpui's own `Window::window_handle()` (an `AnyWindowHandle`) hides the trait method: call `HasWindowHandle::window_handle(window)` and add the `raw-window-handle` 0.6 crate.

Evidence (`info` command): the gpui view's layer is a `CAMetalLayer`, the view has one subview, the `WryWebView`. Coordinates: wry takes top-left logical pixels relative to the gpui view (the gpui view itself is NOT flipped, so raw AppKit frames and masks are bottom-left).

Z-order: a subview always draws above the Metal layer. A gpui card painted later (a "plain" card in front) is hidden behind the webview. Screenshot: `spikes/webview-card/shots/q1-embed.png` (the red-violet gpui card is under the page).

## 2. Canvas zoom and pan: PARTLY

What works: frame = card rect times zoom, `webview.zoom(z)` (`setPageZoom`). The page then lays out at the card's world width at every zoom (`innerWidth` stayed 400 for a 400 px card at zoom 0.25, 0.5, 1, 2, 5), text is re-rasterised, so it is sharp (not a scaled bitmap). Shots: `q2-zoom15-page.png`, `q2-page-0.5.png`. (Sharpness was judged on a scale-1 display; a Retina check is in the needs-input list.)

Cost of following: 180 to 240 steps of a 3 to 4 s zoom animation at 60 Hz, `set_bounds` + `setPageZoom` + mask cost 0.12 ms on average, 0.25 ms worst. During that animation one small page cost about 12% of a core in WebContent and 13% in the WebKit GPU process (`top`). Eight screenshots during the animation showed no gpui placeholder pixel around the card (`edge` check: the webview always covered the gpui rect). I did not judge the feel by eye.

What does not work:

- Below about 0.5x the live page stops matching the card. WebKit keeps text from shrinking under about 9 px, so text reflows and gets cropped (`q2-025-page.png`). The private `_setMinimumLogicalFontSize:` setter does not exist on `WKPreferences` here. Far zoom (fit-all, 0.25x and below) needs the snapshot (question 3).
- The two other ways were tried and dropped: scaling the NSView `bounds` (the content ended up scaled twice) and `setMagnification:` (content cropped, frame fights). Only `pageZoom` is usable.
- A zoom change relayouts the page on each step. The page sees `resize` events while the canvas zooms.

## 3. Overlap, dim, ghost: PARTLY

A native view floats above everything gpui draws. Three tools work.

(a) Snapshot: `takeSnapshotWithConfiguration:` gives an NSImage in about 1 s on a small page (29 KB PNG); it is decoded to a gpui `RenderImage` and drawn with `img()` under or instead of the webview, and the webview is hidden. Works. Shot: `q3-snap-translucent.png` (a translucent card over the snapshot looks right).

(b) Layer mask: a `CAShapeLayer` mask on the webview's layer, even-odd path, the whole rect minus the rects of the cards in front. The covering gpui card shows through the hole. Works, including with several holes (code supports a list). Shot: `q3-hole.png`. Compromises: under a TRANSLUCENT card the hole shows gpui's placeholder, not the page (`q3-hole-translucent.png`), unless a snapshot is drawn under the hole too. And a mask does NOT change hit testing: `hitTest:` still returned the webview inside a hole, so a click on the covering card would reach the page.

The fix that worked: a plain `NSView` above the webview over each hole. It takes the hit (`hit` command: hole gives `NSView`, page gives `WryWebView`, empty canvas gives `GPUIView`) and AppKit's default `mouseDown:` passes the event up the responder chain to the gpui view. A synthetic click reached the NSEvent monitor but I could not make a synthetic event reach gpui's own handler, so "the click arrives in gpui" is NOT proven; it is the documented default behaviour of NSView only.

(c) Dim: `setAlphaValue:` on the webview changed nothing (page pixel stayed 255). A translucent colour `NSView` over the webview works (pixel 255 to 190 for alpha 0.8) and it is also the click catcher for an unfocused card, which matches "the first click on an unfocused card only focuses it" (`q3-dim-overlay.png`).

Ghost during a drag: hide the webview and show the snapshot (mode `snap`); the gpui ghost then draws over it.

Visible compromises, in order of how much they matter:
1. A snapshot is a still picture: a covered or dragged card does not animate or play video while covered.
2. Every gpui overlay (palette, prompts, status bar, menus, ghost) must be turned into a list of cover rects each frame, or the webview must be hidden while one is open. The model has no such list today.
3. Dim is a colour wash over the page, as the terminal cards do it, but a dimmed page cannot show gpui's card background through it.
Not tested: rounded corners on the page (CALayer `cornerRadius` + mask should do it), a page under a window resize.

## 4. Input and focus: PARTLY

Facts from the run (synthetic events inside the process):
- With the webview first responder, typed characters reach the page (an input's value became "yz"); with a Cmd chord the page got nothing.
- An `NSEvent` local monitor sees every key-down BEFORE dispatch whoever is first responder (logged with the webview focused). `keycode.rs` already uses one, so the app can read the chord and swallow it there.
- wry's child webview overrides `performKeyEquivalent:` to return NO (source: `wry_web_view.rs`, issue tauri#9426), so a Cmd chord is not eaten by the page. `window.performKeyEquivalent` returned YES for Cmd+K both with gpui and with the webview focused, but I could not tell WHO claimed it (the spike has no key bindings), so the gpui chord route is not proven.
- gpui's native view is the parent of the webview, so for key equivalents AppKit offers it to gpui first.
- Wheel over the page: `hitTest:` returns the webview, so a wheel goes to the page, as the card scroll rule wants. Over a dimmer or shield view the event goes to gpui and would have to be relayed to the page.

Not tested (needs input, see the list): real keyboard, Cmd+C/V/X/A (WKWebView implements the actions; the Edit menu items must exist), IME and dead keys (WKWebView is its own text input client), and the big risk: Cmd+scroll, pinch, Cmd+drag and middle-drag over a webview card. Those go to the webview, not gpui. An NSEvent monitor that forwards them to the gpui view should work, but I did not test it.

## 5. Page features

Tested:
- Google sign-in page: `accounts.google.com/ServiceLogin` serves the normal "Sign in" page with WKWebView's default user agent AND with a Safari-style UA (`with_user_agent`). An OAuth URL with a fake client id returns `invalid_client`, not `disallowed_useragent`. Limit: Google checks the user agent for real clients, so this does not prove a real sign-in works (`q5-google-defaultua.png`). So `moat.rs` has no WKWebView equivalent needed so far.
- Video: H.264 MP4 plays (currentTime moved, 960x540). `canPlayType`: h264, hevc, vp9, av1 all "probably"; MSE yes. DRM: Widevine `NotSupportedError`, FairPlay (`com.apple.fps`, `com.apple.fps.1_0`) available, ClearKey available. Netflix and Spotify were NOT played (need accounts). Whether they accept an embedded WebKit is unknown.
- `window.open` with a click: `with_new_window_req_handler` got the URL and `width=400,height=300`. We can open a tab instead.
- Link to `slack://`: `with_navigation_handler` sees it and can return false. This is the #668 hook.
- Title and load events: `with_document_title_changed_handler`, `with_on_page_load_handler` (started and finished with the URL). Favicon is not offered by wry.
- Find in page: `findString:withConfiguration:completionHandler:` works (`matchFound` true for text on the page, false for an absent one) once a `WKFindConfiguration` object is passed; nil gave false.
- Download: `with_download_started_handler` fired with the URL and a default path in `~/Downloads`. Completion said `ok=false` because the unbundled debug binary may not write to `~/Downloads` (macOS privacy). Not a conclusion about wry.
- DevTools: `with_devtools(true)` and `open_devtools()` open the Web Inspector, but DOCKED inside the window, and the webview frame is taken over (the page filled the whole window, `q5-after-clicks.png`). `_inspector detach` after `show` did not undock. This needs more work, and it uses private API (`_inspector`).

Not tested, from the wry docs: back, forward, reload, `load_url`, cookies (`set_cookie`), JS evaluation with a callback (used all through the spike), `print`, custom URL schemes (`with_custom_protocol`), init scripts, permission handler, proxy (macOS 14+, `mac-proxy` feature). Needs work: favicon (read `link rel=icon` through a script), tabs of one card (one WKWebView per tab, same store), `target=_blank` and Cmd+click policy (only `window.open` came through the handler), page zoom keys, context-menu items (WKWebView has its own menu), drag and drop of files (`with_drag_drop_handler` exists).

## 6. Cost

Scratch instances, 6 `top` samples, descendants counted. RSS counts shared pages per process, so sums are an upper bound.

| | procs | RSS | CPU (idle, mean of 5) |
|---|---|---|---|
| installed app (CEF), 0 browser cards (a terminal card) | 7 | 679 MB | 2.6% (1.5% without a spike) |
| CEF, 1 browser card | 8 | 730 MB | 3.4% |
| CEF, 5 browser cards | 12 | 1207 MB | 3.0% |
| spike, 0 webviews | 1 | 54 MB | 1.1% |
| spike, 1 webview | 4 | about 165 MB | 4.1% |
| spike, 5 webviews | 8 | 255 MB | 5.2% |

Per extra card: CEF about +120 MB, WKWebView +25 MB (one WebContent process each; GPU and Networking processes are shared). The spike pages are trivial (a form and one script), so a real page costs more on both sides. The spike has no terminals, so the base of 54 MB is not the app's. The comparison that holds is the DELTA: 0 browser cards costs CEF about 420 MB of helpers, WKWebView 0.

One finding on the way: calling `set_bounds` and `setPageZoom` every 16 ms tick, with nothing changed, cost about 15 points of CPU with 5 webviews. The spike now skips unchanged state (`native_signature`). The product needs the same rule.

Size: `Frameworks/Chromium Embedded Framework.framework` is 317 MB of the 367 MB bundle, helpers 2.5 MB more. The spike's release binary is 9.1 MB. New crates for macOS are few (`wry`, `objc2-web-kit`, `dpi` and some HTML parsing crates `wry` pulls in; the lock file also lists GTK and Windows crates that macOS does not build). I did not measure what wry adds to the app binary.

## 7. Windows and Linux (read from docs and source, NOT tested)

- Windows: gpui 0.2.2 `WindowsWindow` implements `HasWindowHandle` (a Win32 HWND); its display handle is `unimplemented!()`, which wry's child path does not need as far as I read. wry creates a WebView2 child window. It will have the same "native window above the gpui swapchain" problem. A child HWND can take `SetWindowRgn` with holes, and hit testing follows the region, which may be better than macOS, but that is an untested guess. User data folder: `WebContext::new(Some(dir))`, so it CAN follow our data dir. Webviews with different environment options need different folders. Needs the WebView2 runtime (present on Windows 11, installable bootstrapper on 10).
- Linux X11: wry says `build_as_child` works on X11 only and needs `gtk::init` and a GTK loop advanced beside our event loop. gpui 0.2.2 `X11Window::window_handle` is `unimplemented!()`, it panics (only an internal `RawWindow` returns an Xcb handle). So it needs a gpui patch. Data folder: `WebContext::new(Some(dir))` maps to webkit2gtk `WebsiteDataManager` base data and cache directories, so it follows our dir. Needs `libwebkit2gtk-4.1` installed: a package dependency for the tarball.
- Linux Wayland: wry's child path "won't work on Wayland". The Wayland alternative is `build_gtk` in a `gtk::Fixed`, but gpui draws no GTK widgets, so there is no container. No known way.
- So an OS webview replaces CEF on macOS only. Windows needs work. Linux would keep CEF or lose browser cards.

## 8. Login sessions: YES, with one trap

wry uses `WKWebsiteDataStore.defaultDataStore` (one global store per bundle id) unless told otherwise. `with_data_store_identifier([u8; 16])` (macOS 14+) uses `dataStoreForIdentifier`. `with_incognito(true)` uses the non-persistent store.

Where the data lives (checked on disk): `~/Library/WebKit/<bundle id or exe name>/WebsiteDataStore/<uuid>/` (Cookies, LocalStorage, IndexedDB, NetworkCache, MediaKeys, ...) and `~/Library/Caches/<id>/WebKit/`. There is NO setting that moves it into `INFINITERM_DATA_DIR`. wry has no data directory on macOS.

Tested with `state.html` (persistent cookie, session cookie, localStorage, IndexedDB), store id from a seed string:

| Case | Result |
|---|---|
| (a) close a card and open a new one, same process | all four kept |
| (c) two cards open at once, same store | both see all four (one shared store) |
| (b) quit and relaunch, same store id (also after SIGTERM) | persistent cookie, localStorage, IndexedDB kept; session cookie GONE |
| another store id | empty (profiles are separate) |
| default store, and incognito | empty and independent of the named store |

The trap: a scratch instance and the real app share the bundle id and so `~/Library/WebKit/<bundle id>/`. They stay apart only if the store id is derived from the data dir (the spike's `store_id(seed)`). A scratch instance must never use the default store. Scratch stores pile up on disk and need `removeDataStoreForIdentifier:` (macOS 14+, not tested). The session cookie is lost on relaunch, as in a normal browser.

Today's CEF profile has cookies encrypted with Chromium's key, and there is no WebKit import path built. The spike shows a new store starts empty, so every user signs in again once. Importing is possible in principle (`set_cookie` takes a cookie; decrypting the CEF database needs the Keychain key), but I did not try it. Windows: `WebContext::new(Some(dir))` is the WebView2 user data folder. Linux: the same call sets the WebKitGTK `WebsiteDataManager` base data and cache directories.

## What the product work would be (macOS)

Rough sizes, one person, my guess after the spike:

1. Engine seam: a `BrowserEngine` interface so `browser_body.rs` can sit on CEF or WKWebView: 3 to 4 days.
2. WKWebView card body: create, frame and zoom sync with a change signature, page events to the omnibox, navigation, tabs (one view per tab), popups to tabs, app-link policy (#668): 1 week.
3. Cover and input layer: cover rects from every overlay and card in front, mask plus a hit-test view with several rects, dim view, snapshot for covered, dragged and far-zoom cards, event forwarding for Cmd+scroll, pinch, Cmd+drag and middle-drag: 1 to 1.5 weeks. This is where the risk is.
4. Keys and Edit menu, find bar on `findString`, downloads with a save path and progress, DevTools (undock or accept docked), context menu: 4 to 5 days.
5. Profiles: store id per data dir, cleanup, a sign-in-again note in the changelog, a Google sign-in check with a real account: 2 to 3 days.
6. Removal of CEF: `infiniterm-browser`, helpers, bundling, signing, notarizing the framework, `moat.rs`, the devtools card, the extension code: 3 to 4 days, and it shrinks the build a lot.
7. Tests and drive scripts for all of it: 3 to 4 days.

Total about 4 to 5 weeks for macOS, with step 3 the one that can grow. Ekin's estimate in #761 was 2 to 3 weeks for macOS. Windows would be a second project, Linux keeps CEF or loses browser cards.

## Needs input (I could not do these without touching the Mac)

1. A real Cmd+scroll, pinch, Cmd+drag and middle-drag over a webview card: does the canvas zoom and pan, or does the page take it? (the main risk).
2. A real keyboard in a page input: typing, Turkish layout dead keys, an IME, Cmd+C, Cmd+V, Cmd+A, and one app chord such as Cmd+K with the page focused.
3. Looking at the zoom by eye on the Air (Retina): is the text sharp, is a pinch smooth, do you see the page reflow.
4. A real mouse click on a covering card over a webview card (the shield view), a wheel over a dimmed card, a drag with the snapshot.
5. A real Google sign-in with an account and 2FA, and a Netflix and a Spotify web player try (accounts needed).
6. DevTools: whether a docked inspector is acceptable, or a window is wanted.
7. A download into a bundled build (the debug binary could not write to `~/Downloads`).

## What was not tested

Retina scale; rounded corners; window resize; a page under many covers; real DRM playback; the HID input paths above; Windows and Linux; memory under real pages; the build-size effect of wry on the app binary; `removeDataStoreForIdentifier:`; the CEF cookie import.
