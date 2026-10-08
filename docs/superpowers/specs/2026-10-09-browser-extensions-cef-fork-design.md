# Browser extensions, DevTools and our own CEF build

Status: draft for review, 2026-10-09. Nothing in this spec is built yet. The CEF source is checked out and nothing is compiled.

## Why

Extensions load in a browser card, but they cannot work like they do in Chrome. A popup cannot find the page it belongs to, an extension can open a native window outside the canvas, and there is no toolbar. Video in H.264 does not play. A card has no DevTools.

The goal: any MV3 extension works in a browser card, its icon and badge sit in the card's footer status bar, its popup opens from there, DevTools opens beside the page, and common web video plays. Dark Reader was the test case. The design must not be specific to it.

## What was measured (2026-10-07 and 2026-10-08)

Setup: the 0.5.7 bundle, CEF 152.0.6 (Chromium 152.0.7977.83), Alloy style with windowless rendering, in a scratch data dir. The probe extensions are in `spikes/ext-probe/`.

| Check | Result |
|---|---|
| Dark Reader (MV3) loads, filters a page with no click | Works |
| Popup page as an ordinary page (`chrome-extension://<id>/popup.html`) | Renders, reaches its background worker |
| `chrome.tabs.query({})`, `windows.getAll()` | Return `[]` |
| `windows.getLastFocused()` | Error "No last-focused window" |
| `tabs.get(id)`, `scripting.executeScript({tabId})`, `webNavigation` events | Work for our surfaces; `active` is always false and `windowId` is -1 |
| `tabs.onCreated`, `onUpdated`, `onActivated` | Never fire |
| `windows.create`, `tabs.create` | Open a REAL native Chromium window outside the canvas |
| `chrome.action` (badge, title, `onClicked`, `getUserSettings`) | Calls succeed and the values are stored; nothing is drawn |
| `bookmarks`, `history`, `topSites`, `sessions`, `cookies`, `management`, `declarativeNetRequest`, `commands.getAll`, `identity`, `downloads.search` | Work |
| H.264, H.265, AAC, MSE `avc1` | Not supported. VP8, VP9, AV1, Opus, MP3 work |
| Widevine | Not supported |
| Platform authenticator (passkeys) | Not available |

CEF's maintainer closed the related issues (4011, 4255) as not planned: Alloy style supports only the extension APIs the PDF viewer needs. So upstream will not fix this.

One more finding: after `bg.js` and the manifest version changed, the old service worker kept running until the extension id changed. Extension updates need handling (see Open questions).

## Decision: build our own CEF

Options that were weighed:

1. Patch the installed extension copy with a JS shim. Needs a channel to the app, and cannot reach every context.
2. Inject in the render process. CEF 152 has no hook for service workers, so backgrounds stay broken.
3. A custom CEF build. Real fix, and the only way to change codec flags.

We take option 3. Reasons: the codec flags need it anyway, the tab and window patch is small, and we do not need upstream updates often. We pin CEF 152 for a long time.

The source is at `/Volumes/cefbuild/cef-src` (a 250 GB APFS sparse image on an external disk, `Untitled`; remount with `hdiutil attach`). CEF commit `708dc140c`, Chromium `152.0.7977.83`, branch 7977, no Chromium history, about 30 GB.

The GPU box cannot build this: the output must be macOS arm64, and Chromium builds for macOS only on a Mac.

## What the build changes

Each item is a patch in our own patch directory, applied by `automate-git.py`'s patch mechanism (CEF's `patch/patches/`).

**P1. Tab and window model.** Add Alloy surfaces to `TabsQueryFunction::BuildTabList` and to the `windows.*` functions. A new CEF API, on `CefBrowserHost`, tells CEF three things about a surface: its window id, whether it is the active tab of that window, and whether that window is the last focused. The app sets these. Mapping: one window per browser card, the card's tabs are its tabs, the focused browser card is the last focused window. `tabs.onCreated`, `onUpdated` and `onActivated` fire from the same calls.

**P2. No native windows.** `windows.create` and `tabs.create` must not open a native window. They call a new handler on `CefLifeSpanHandler`-style client (`OnExtensionCreateTab`), and the app opens a tab on the card or a card. Default when no handler exists: refuse with an error.

**P3. Action state.** A new observer on the request context reports changes of an extension's action: icon (as bitmaps), badge text and colour, title, enabled state, and "popup path". The app draws them in the footer status bar. `chrome.action.onClicked` is raised by a new call, `CefExtension::ClickAction(browser)`.

**P4. Commands.** A call that dispatches a key chord to `chrome.commands`, so `commands.onCommand` fires. Needed only if the app routes keys to extensions; may move to a later phase.

**P5. Codecs.** Build with `proprietary_codecs=true` and `ffmpeg_branding=Chrome`. This is a build flag, not a patch. Patent licences for H.264 and AAC are our risk as the distributor; Ekin decides whether to accept it before the first public release that carries this build.

Not in scope: Widevine, passkeys, Safe Browsing, Translate, Sync, Payment Request.

## What the app does

**Footer status bar of the browser card.** One bar at the bottom of the card. It holds, left to right: extension icons (with badge and title as a tooltip), then the card's other items (DevTools toggle, later: downloads, permissions, zoom). Icons come from P3. Layout, sizing and the card-radius rule follow `chrome::card_body_quad` and `CardRadius` as the tab strip does.

**Popup.** A click on an icon opens the extension's popup page as a small CEF surface anchored above the footer, closed by Escape or a click outside. It is a normal surface at the extension's own URL, so `chrome.tabs.query` now answers with the right tab through P1. The popup surface is a tab of window 0 only for the API's purposes: it must not count as a user tab.

**Extension management.** `ift install-extension` stays. A new `ift extensions` lists installed ones and their ids. Updating an extension changes its version and, if needed, its id, so the stale service worker problem goes away (see Open questions).

**DevTools.** See the next section. It needs no patch.

**Handlers the app still needs (no fork).** Downloads, file chooser, JS dialogs, permission prompts, HTTP auth, certificate errors, fullscreen. Each is a missing handler in `surface.rs`. They are separate tasks and are listed on the roadmap, not designed here.

## DevTools

Today a card has none. Both paths below were tried on 2026-10-09 with a throwaway spike (not merged).

1. **`ShowDevTools` with a windowless client: does not work.** CEF logs "Windowless rendering is not supported for this DevTools window" (`chrome_browser_delegate.cc`) and opens a real native window, "DevTools - example.com/", outside the canvas. DevTools windows always use the Chrome style path, so a patch would not be small.
2. **Remote debugging port and an ordinary card: works.** With `remote_debugging_port` set and the switch `--remote-allow-origins=http://127.0.0.1:<port>`, a browser card opened on `http://127.0.0.1:<port>/devtools/devtools_app.html?ws=127.0.0.1:<port>/devtools/page/<id>` shows the full DevTools (Elements, Console, Network and the rest), connected to the page. Without the allow-origins switch Chromium rejects the WebSocket and the log names the flag.
3. **Raw protocol** (`ExecuteDevToolsMethod`, message observer): not a UI. Useful for a console or network count in the footer. `moat.rs` already uses it.

We take option 2. The page's target id comes from `GET /json` on the port, matched by url; the `devtoolsFrontendUrl` in that list points to a hosted frontend on the internet, so we build the local URL ourselves. DevTools opens as a card beside the page's card (command `browser.devtools`, palette and a footer button), tied to that page. Closing the page closes it.

The same list also shows the extensions' service workers and background pages, so the same card inspects an extension. That makes this the main way to debug the extension patches below.

Security: an open debugging port lets any local program drive every page, with the cookies they hold. The port binds to loopback and the allow-origins switch names only our frontend's own origin, so a web page cannot attach. A local program still can. Recommendation: a setting `browser.devtools`, default off, read at launch, with a pick of a random free port per launch. The command explains how to turn it on when it is off. Open question below.

## Build and distribution

- The build runs on this Mac, in `/Volumes/cefbuild/cef-src`, with `automate-git.py --arm64-build --no-debug-build --client-distrib`. A first full build takes hours and needs the external disk mounted.
- The output is a CEF client distribution. `cef-rs` reads it through `CEF_PATH`. The `cef-sys` bindings are regenerated from our headers (`update-bindings`) because P1 to P4 add API.
- `tools/bundle.sh` takes the framework from our build and `tools/sign.sh` signs it as it does now. Release builds embed the custom framework; a dev build can still use the prebuilt one with a feature flag until the patch lands.
- The framework version string gets a suffix (`+ift1`) so a log shows which one runs.
- Rebuilds: only when we change a patch or must take a Chromium security update. No schedule. A security update means rebasing the patches onto a newer CEF branch.

## Risks

- **Alloy style may be removed upstream.** Our windowless rendering needs it. Pinning CEF 152 delays this, and a browser that never updates takes security risk. The mitigation is a rebase plan when a Chromium vulnerability matters.
- **Patent licensing** for H.264 and AAC (P5), as above.
- **Build size and disk.** About 100 to 150 GB total. The disk must stay plugged in during builds.
- **Patch maintenance.** P1 to P4 touch Chromium's `tabs_api.cc`, `windows_api.cc`, `extension_action` code and CEF's client API. Each is a few dozen lines, but a rebase conflicts if upstream moves them.
- **Unknowns tested only in part:** whether `contextMenus` items from extensions reach our context menu, whether `notifications` works (the first probe's icon was invalid), whether commands need P4.

## Phases

0. Baseline: build the unmodified source, check the toolchain, measure time, make a client distribution, run the app on it.
1. P5 codecs. Check H.264 and AAC play. Smallest change, biggest user effect.
2. DevTools card (path 2 above, already proven by the spike).
3. P1 and P2. Check with `spikes/ext-probe` that `tabs.query` returns our tabs and `windows.create` opens no native window.
4. P3 and the footer bar. Dark Reader's icon, badge and popup work.
5. P4 and the rest, if needed.

Each phase ends with a check on screen with the driver, on a scratch instance.

## Open questions

- A window id per card, or per workspace? The spec says per card. A workspace-wide window would make "active tab" cross-card and is harder to explain.
- Does a locked and focused browser card decide the "last focused window", or the card with the app's focus? The spec says the focused card, locked or not.
- Extension updates: change the id (installing under a new directory) or force the worker to reload. The probe showed the cache; the fix is not yet chosen.
- DevTools port: default off with a setting and a restart, or on at every launch? Off is safer; on is easier to use. A CEF port cannot be opened after launch.
- Whether to keep the prebuilt framework as a fallback path in the bundle script, for people who build the app from source without our CEF.

## Tests

- `spikes/ext-probe/`: a probe extension whose popup prints the results of the checks in the table above. Run in a scratch instance after each phase. Its page is the regression test for P1 to P4.
- Unit tests for the pure parts: manifest icon and popup path parsing, the footer's item layout, the tab and window id mapping.
- Driver scenario `tools/drive/extensions.sh`: install a fixture extension, click its footer icon, see the popup, check `tabs.query` through the probe page.
