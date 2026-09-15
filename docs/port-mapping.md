# infiniterm native port: feature and file mapping

Written 2026-09-14 against `master` at `e43d786` plus the uncommitted browser-card spike. Numbers are `wc -l` on that tree. Nothing here has been prototyped; every "verify" is a thing to check with a 50-line spike before committing to it. **Status 2026-09-15: the stack is decided and the four spikes at the bottom (plus a combined canvas one) have run and passed; their numbers are in `~/Code/infiniterm/spikes/*/NOTES.md` and the handover is `~/Code/infiniterm/HANDOVER.md`. Rows marked *overturned* below were changed by a spike result.** Since this was written the reference app gained transcript cards (`transcript.ts`, `transcript.rs`, `TranscriptCard.svelte`), the sidebar (`sidebar.ts`, `SidebarHandle.svelte`), keyboard move/resize (`cardActions.ts`), browser page-zoom keys (`browserKeys.ts`) and a Pi hook adapter; the handover's inventory is the current one.

## Recommendation

Rust + gpui, VT engine `alacritty_terminal`. The whole Rust backend (2,791 lines in `src-tauri/src` + 707 in `crates/`) survives as-is except the `#[tauri::command]` glue in `ipc.rs`/`lib.rs` and the two spike files. The 4,833 lines of pure TS modules and their 3,946 lines of tests transliterate to Rust one file at a time, same names, same tests. Only the Svelte components (4,783 lines) and the runes stores (1,801 lines) need real redesign, and those are the thin part.

C++ costs the backend rewrite (PTY, socket, git, config, inspect, links, files, themes: ~2,000 lines to redo) and the port of every tested module into a second language before any UI exists. What it buys is Qt's `QGraphicsView`, which is a zoomable scene of widgets and maps the canvas almost for free, and `QWebEngineView`, a real embedded Chromium. Both are covered below because the browser card is where they matter.

| | Rust + gpui | C++ + Qt 6 |
|---|---|---|
| Backend | ✅ reused, ~150 lines of glue change | ⚠️ ~2,000 lines rewritten (fork/pty, QLocalServer, QProcess for git) |
| Pure modules + tests | ✅ 1:1 transliteration, tests carried | ⚠️ same work, different language, no `vitest`-style runner without picking one |
| Canvas zoom | ⚠️ custom element, text reshaped per zoom (verify glyph atlas behaviour) | ✅ `QGraphicsView::scale`, items are widgets via `QGraphicsProxyWidget` |
| Terminal | ✅ `alacritty_terminal` (Apache-2.0); Zed's `terminal_view` shows the shape but is GPL, do not copy | ⚠️ `libvterm` (C, neovim's) or `libghostty-vt` (C API, new); render loop is yours |
| Editor | ⚠️ nothing off the shelf; rope + tree-sitter + own view, the biggest single item | ⚠️ QScintilla exists but no merge view and looks its age; same hand-build in practice |
| Browser card | ⚠️ WKWebView child view over the Metal layer, the same overlay problem as today; CEF offscreen composites properly at 200 MB | ✅ `QWebEngineView` inside a proxy widget: zooms, layers, known quirks |
| Toolchain | ✅ one language, one `cargo test`, `ift`/hook crates already there | ⚠️ CMake + Qt licence (LGPL dynamic is fine for a private app) + Rust still for `ift`/hook |
| Maturity | ⚠️ gpui is Zed's, API moves, docs thin, macOS is its best platform | ✅ stable for two decades |

Which one? The rest of this file maps both, but the file table assumes gpui.

### Stack decision (2026-09-14, time cost out, everything replaceable)

Rust workspace, four crates, and gpui is confined to one of them:

| Crate | Holds | Depends on |
|---|---|---|
| `infiniterm-core` | the backend as it is today, every pure module and its tests, layout, persistence, config, commands, keymap | serde, portable-pty, tokio: nothing graphical |
| `infiniterm-term` | `alacritty_terminal` wrapped as "a grid of styled cells plus a byte sink", links, the shell key handler | core |
| `infiniterm-browser` | the CEF host: OSR frames out, input in, extension loading, native messaging path | `cef-rs`, or a C++ shim with a C ABI if the binding falls short |
| `infiniterm-ui` | canvas, card frame, palette, editor view, all elements | gpui + the three above |

The point of the split is that the choice of gpui is a choice about one crate. If it churns past tolerance, `infiniterm-ui` is rewritten on iced or on wgpu + cosmic-text by hand and nothing else moves. It also keeps the "pure function in its own file with tests" rule from today intact: `core` has no idea what draws it.

Why gpui for that one crate, considered against the alternatives with no time pressure: it is the only GPU UI toolkit that already renders a terminal and a code editor at Zed's scale, in the language the backend and `ift` are in, and its CEF binding is maintained by the Tauri team for production use. Qt is more mature but pairs badly with CEF (two message loops, `QCefView` is the workaround), has no extension-capable webview of its own, and puts the backend rewrite back on the table. AppKit + Metal in Swift is the most native answer and the Ghostty shape (core in one language, shell in Swift), but it means writing the glyph renderer, text layout and editor view that gpui ships. Hand-rolled wgpu is that plus a widget layer; it is the fallback, not the start.

### Cross-platform, and the compositor door (2026-09-14)

Two later goals shape the split above: the app should run on Linux and Windows, and the canvas may one day be a Linux desktop environment, with every application window a card. Neither is v1 work, but both are cheap to keep open now and expensive to reopen later.

Cross-platform status of the pieces: gpui ships Zed on macOS, Linux (Wayland and X11) and Windows; CEF and `alacritty_terminal` run on all three; `portable-pty` covers ConPTY. What is macOS-only today and needs a per-OS implementation behind one trait: `inspect.rs` (`ps` parsing; `/proc` on Linux, `NtQueryInformationProcess` or the ConPTY API on Windows), `links.rs` opening a URL, the unix socket in `ipc.rs`/`cli.rs`/`hooks.rs` (a named pipe on Windows; `ift` and the hook binary share the change), `themes.rs`/`config.rs` paths (`~/.config` is already XDG-shaped), window-frame persistence, single-instance locking, and the WKWebView spike, which does not travel and is dropped anyway. Cmd becomes Super or Ctrl per platform in `isAllowedChord`; the rule "the app owns one modifier the shell never sees" is what carries over, not the key.

The compositor door: on Linux a Wayland compositor receives every client's buffer as a texture, which is exactly what the CEF card does with Chromium's frames. `smithay` is the Rust compositor toolkit; `niri` (scrollable tiling, smithay, Rust) is the nearest living relative of this canvas and worth reading for how it hands surfaces to a renderer and routes input back. As a compositor the browser card is real Chrome, extensions and all, with no CEF; on macOS CEF stays because nothing there lets a third-party app composite other apps' windows.

The one design consequence for v1: `infiniterm-core` must not know what a card body is. A card is a rect, a kind tag, and an id; the body is a trait in `infiniterm-ui` with `paint(texture, rect)` and `input(event)`, implemented by the terminal grid, the editor, the CEF surface, and later a Wayland surface. Layout, groups, workspaces, splits, swaps, focus, slots and persistence operate on rects and never reach into a body. That is already how the pure modules are written; the rule here is that the port does not lose it.

Not the ask, noted once: the cheapest route to "a browser with extensions inside the canvas" is not a rewrite but swapping Tauri's WKWebView for CEF as the app's webview (Tauri v3 is building exactly this on `cef-rs`), keeping every line of Svelte. It gets the extension but keeps xterm.js and the DOM, which is the ceiling the native port exists to leave.

## What disappears in a native port

Not features. Workarounds for being a webview, listed so nobody ports them.

| Today | Why it existed | Native |
|---|---|---|
| `xtermTransform.ts` | xterm hit-tests in CSS px, world is scaled | gone; you own hit-testing |
| `outputScheduler.ts`, `flowControl.ts` (JS half) | xterm parses on the UI thread in 12 ms slices | *overturned by the term-zoom spike*: parsing on reader threads under alacritty's `FairMutex` gave 1 fps with 25 panes of `yes`; the app's own shape (reader threads only move bytes into a bounded channel, the UI thread parses a byte budget per frame, round-robin) gave 120 fps. So `outputScheduler.ts` PORTS, as the budget controller in `infiniterm-term`, and the `Credit` in `local_pty.rs` stays for memory |
| Capture-phase key/wheel handlers | xterm's textarea is the event target | the terminal element simply does not consume Cmd chords |
| `Cmd+H` unbindable, menu rebuild too costly | AppKit default menu beats WKWebView | you define the menu (`cx.set_menus`), Edit items included |
| `window.prompt` dead, `NamePrompt.svelte` | wry has no dialog delegate | `NamePrompt` stays as a UI, the reason is gone |
| `index.html` literal background, `tauri.conf.json` `backgroundColor` | white flash before CSS | window background set at creation |
| `chrome.ts` inverse-scale maths, `--inv`, `--ring` on `.chrome` | CSS transform shrank borders, custom property inheritance invalidated spans | paint borders in screen px in the card element's paint pass; nothing to invalidate |
| `mirror.ts`, `overlayMirror.ts`, `overlay.rs`, `overlay.html`, `src/overlay.ts` | only a second window paints above a native webview | still needed with WKWebView (see Browser); gone with CEF OSR or QWebEngine |
| `tauri-plugin-single-instance`, `tauri-plugin-window-state`, `tauri-plugin-opener` | | own: lock the unix socket path (already the liveness signal), `NSWindow` frame autosave, `open`/`NSWorkspace` |
| `paneRegistry.ts` theme replay | xterm copies its theme at construction | terminal element reads the live theme entity on render |
| `motion.svelte.ts` `data-animations` | CSS media query vs JS split | one `animations_enabled()` reading `NSWorkspace.accessibilityDisplayShouldReduceMotion` and the setting |
| `app.reload` | Vite reload killed shells | gone; a dev build restarts |
| `vite.config.ts`, `svelte-check`, `package.json` | | gone |

## Backend: kept

Every file in `src-tauri/src` except the three named at the bottom. Change: replace `#[tauri::command]` functions with plain functions on an `App` struct and the `Channel`/`Emitter` events with a `crossbeam`/`std::sync::mpsc` receiver the UI drains each frame (or `gpui::AsyncApp` spawn + `cx.notify`).

| File | Lines | Change |
|---|---|---|
| `backend/local_pty.rs`, `backend/mod.rs` | 539 | none; `SessionBackend::new()` already returns one receiver for all panes (the tmux constraint) |
| `inspect.rs` | 473 | none |
| `git.rs` | 258 | none |
| `config.rs` | 223 | none; the 1 s watcher stays |
| `files.rs` | 203 | none (atomic writes, drafts, `file_mtime`) |
| `cli.rs` | 182 | none; the verbs it forwards to the webview (`iftHandler.ts`) now call the model directly, so the 2 s wait for the webview goes |
| `hooks.rs` | 163 | none |
| `links.rs` | 103 | none |
| `themes.rs` | 91 | none; `seed()` semantics kept |
| `layout.rs` | 64 | none; same `workspace.json` so existing installs load |
| `ipc.rs`, `lib.rs`, `main.rs` | 314 | rewritten: this is the glue |
| `browser.rs`, `overlay.rs` | 178 | replaced by whatever the browser decision below is |
| `crates/infiniterm-cli`, `crates/infiniterm-hook` | 707 | none; same socket, same JSON |
| `resources/themes/*` (521) | | bundled as before |

## Pure modules: transliterate, tests included

Each becomes `src/<name>.rs` with a `#[cfg(test)] mod tests` holding the same cases. Order matters: geometry first, because everything else is tested against it.

| TS file | Lines | Rust notes |
|---|---|---|
| `grid.ts`, `layout.ts`, `resize.ts`, `navigate.ts`, `slots.ts` | ~450 | plain structs; `cardsOn(workspace)` scoping stays a function argument, never a global |
| `split.ts`, `swap.ts`, `multiSelect.ts`, `groups.ts`, `workspaces.ts` | ~500 | `reclaim` decides from rects, `splitFrom` first: keep the tests that pin the 2x2 case |
| `zoomActions.ts`, `zoomAnimation.ts`, `momentum.ts` | ~200 | drive from gpui's frame callback (`cx.on_next_frame` / `request_animation_frame`) instead of rAF |
| `chrome.ts` | 94 | shrinks to `fn screen_px(px, zoom) -> f32` |
| `savedLayout.ts` | 337 | `serde` structs + the same version migrations; `isNewerThanThisBuild` → `persistence.read_only` stays |
| `config.ts`, `settingsDoc.ts`, `jsonc.ts` | 661 | `jsonc.rs`: `stripComments` padding + `patchJsonText` exactly as is, this is the "never reserialise the user's file" rule; `SETTINGS_DOC` + `undocumented()` test kept |
| `itermcolors.ts` | 77 | `plist` crate, or port the hand parser |
| `keymap.ts`, `shortcuts.ts` | 514 | bindings table keeps the reason-per-line comments; `keybindings.default.json` still generated from it; chords built from the physical key (see Keys) |
| `commands.ts` + `commands/*.ts` (~60 commands) | ~900 | `struct Command { id, label, run, hint }` registry; keep `Domain: what it does` labels; `isAllowedChord` is still the one gate |
| `fuzzy.ts`, `palette.ts`, `paletteUsage.ts` | 340 | `HINT_PENALTY`, true total for truncation, recency and frequency kept apart |
| `agentState.ts`, `cardLabel.ts`, `labelColors.ts` | ~250 | hash-from-id colour, red excluded |
| `ift.ts`, `iftHandler.ts`, `links.ts` | 250 | `OpenPlan` stays the single path for `ift <path>`, Cmd+click and the typed prompt |
| `editorTheme.ts`, `blame.ts` | 150 | ANSI slot → highlight tag map targets tree-sitter capture names instead of Lezer tags |
| `mirror.ts` | 110 | drop unless the WKWebView route is chosen |
| `tokens.test.ts` | | replaced by a `Theme` struct: chrome colours are fields, so the guard is the type system |

## Stores → gpui entities

The `.svelte.ts` files (1,801 lines) become `Entity<T>` models. `$effect`s that watched a store become `cx.observe(&entity, ...)` or plain method calls at the mutation site.

| Store | Entity | Notes |
|---|---|---|
| `cards.svelte.ts` | `Cards` | `SIZE_MODE` constant and its tests kept; `setCardCells` injection becomes a field |
| `groups.svelte.ts`, `workspaces.svelte.ts`, `selection.svelte.ts` | `Layout` (one entity or three) | group frame still derived from `card.groupId`; phantom exclusive with focus |
| `viewport.svelte.ts`, `viewportAnimator.svelte.ts`, `swapAnimation.svelte.ts` | `Viewport` | every change animated, never assigned |
| `persistence.svelte.ts` | `Persistence` | `loaded` gates save and first card; 500 ms debounce; `readOnly` |
| `settings.svelte.ts`, `theme.svelte.ts` | `Settings`, `Theme` | `loadTheme` stamp kept; selection colours pushed into terminal entities |
| `paletteState.svelte.ts`, `prompt.svelte.ts`, `notice.svelte.ts` | `Overlays` | |
| `cardLifecycle.svelte.ts` | method on `Cards` | `closeCard` is the one exit for both `card.close` and shell exit |
| `uiScale.svelte.ts`, `motion.svelte.ts`, `fps.svelte.ts`, `env.svelte.ts`, `input.svelte.ts`, `panels.svelte.ts` | fields on `App` | |

## Components → gpui elements

| Component | Lines | Element | Notes |
|---|---|---|---|
| `App.svelte` | 361 | `App: Render` root | key dispatch, menu, socket drain, effects |
| `Canvas.svelte` | 543 | custom `Element` | layout: world rect × zoom + pan → pixel rect per card; paint: children + group frames; wheel: Cmd+scroll zoom, bare scroll to the card under cursor; Cmd+drag pending until `DRAG_SLOP`, middle captures at once |
| `CardFrame.svelte` | 686 | `CardFrame` | label + kind badges, identity colour, state outline in screen px, edge-resize handles, maximise without a second child (the second-PTY trap) |
| `GroupFrame.svelte` | 196 | `GroupFrame` | no hit-test except the header |
| `StateRing.svelte` | 83 | paint in `CardFrame` | only `working` animates |
| `TerminalCard.svelte` | 377 | `TerminalElement` over `alacritty_terminal::Term` | see Terminal |
| `EditorCard.svelte`, `Explorer.svelte` | 846 | `EditorElement`, `FileTree` | see Editor |
| `DiffCard.svelte`, `DiffTree.svelte` | 608 | `DiffElement`, flat list | see Editor |
| `BrowserCard.svelte` | 104 | see Browser | |
| `Palette.svelte` | 282 | `anchored()` + `uniform_list` | sources with `preview(None)` on dismiss, scroll-into-view driven by the selection index |
| `ShortcutPanel.svelte`, `NamePrompt.svelte`, `WorkspaceTabs.svelte`, `StatusBar.svelte`, `TitleBar.svelte` | 637 | straightforward `div()` trees | `Cmd Shift T` spelling, one box per key; dots per workspace, never merged |

## Terminal

`alacritty_terminal` gives `Term<T>` (grid, scrollback, VT parsing via `vte`), `EventListener` for title/bell/clipboard, selection, and `RenderableContent` iteration. It does not render or own the PTY; ours does (`local_pty.rs`, keep it, feed bytes to `Term::processor`).

What has to be built on top, with the current file that defines the behaviour:

| Feature | Today | Native |
|---|---|---|
| Fixed 69 × 80 cells for a new card | `cards.svelte.ts` `SIZE_MODE` | same; cell size from the shaped font at zoom 1 |
| Refit on font/letterSpacing/lineHeight change | `applyOptionsAll` → `refit` | resize `Term` and PTY when the cell metric changes |
| Theme, 16 ANSI + fg/bg/cursor | `theme.svelte.ts` | colour table on the element |
| Selection colours from the theme yellow / `editor.selection*` | `applySelectionColors` | same source, both elements |
| Links underlined after fs confirms, Cmd+click opens, plain click focuses | `links.ts` + `links.rs` | hover → `links.rs` check → underline run; same rule |
| `Alt+Arrow` / `Cmd+Arrow` as `ESC b/f`, `^A/^E` | custom key handler in TerminalCard | in `TerminalElement::key_down` before the byte encoder |
| Mouse reporting for TUIs | xterm | `alacritty_terminal` mouse mode + own encoder (Zed's is GPL, wezterm's `termwiz` is MIT if you want a reference) |
| Cursor styles, blink, bell | xterm | own paint; blink gated on `animations_enabled` |
| Foreground process, cwd, ssh badge | `inspect.rs` | unchanged |
| Scrollback search | none today | none |

Options to weigh: `wezterm-term` (MIT, more complete mouse/kitty-protocol support, heavier), `vt100` (small, no image or kitty protocol, fine for a shell, thin for TUIs). "cte" is not a crate I can identify; if it is a specific engine, name the repo.

Glyph rendering at zoom: gpui shapes text at a pixel font size, so a continuous zoom means reshaping every visible line at a new size each frame and new rasterised glyphs in the atlas. Verify with 25 cards of `yes` at a pinch-zoom before believing 60 fps. *Verified 2026-09-14 (term-zoom spike): 85 to 120 fps idle, 118 to 120 flooding, 5 to 11 ms paint for 1000 visible lines; neither escape hatch is needed.* Two escape hatches if it stalls: quantise the zoom to steps during the animation and land on the exact value, or paint each terminal to a texture at its last settled zoom and scale the texture while animating.

## Editor and diff

The biggest item and the one with no crate to lean on. What CodeMirror 6 gives today and where it comes from natively:

| Feature | CM6 package | Native |
|---|---|---|
| Buffer, multi-cursor, undo | `@codemirror/state`, `commands` | `ropey` + own undo stack; single cursor is enough for v1 (verify: is multi-cursor used anywhere? The chord list says no) |
| Syntax highlighting, ~150 languages lazy | `language-data`, `@lezer/highlight`, `codemirror-lang-svelte` | `tree-sitter` + grammars compiled in; pick 20 first, the rest on demand is a build question |
| Theme from terminal ANSI slots | `editorTheme.ts` | same map, capture names instead of tags |
| Find/replace, go-to-line | `@codemirror/search` | own overlay; the palette commands `editor.find`, `editor.goToLine` keep existing |
| Comment toggle, indent | `commands` | own, per-language comment token from the grammar |
| Unified merge view, collapsed unchanged, read-only | `@codemirror/merge` | `similar` crate for hunks; own rendering of `+`/`−` lines with the collapse rule |
| Blame gutter | own, `blame.ts` + `git.rs` | unchanged data, own gutter |
| Dirty/language/readOnly badges | `CardFrame` | same |
| Reload clean buffer on disk change, keep dirty, re-base `saved` | `file_mtime` poll 2 s | unchanged |
| Drafts 500 ms after change, deleted by `closeCard` only | `files.rs` | unchanged |
| Untitled → save asks a path | `NamePrompt` | same |
| File tree, lazy, dotfiles shown, `.git` hidden, clean-loads / dirty-opens-beside | `Explorer.svelte`, `dir_list` | same rules |
| Config pair, defaults read-only, one close closes both | `configPairOf` | same |
| Text metrics copied from the terminal's drawn cell | `terminalTextMetrics()` | both elements shape with the same font and size, so the trap disappears |

Scope ceiling: this is a code viewer with saving, not an IDE. No LSP, completion or lint by the 2026-09-14 decision; that decision is what keeps this at weeks rather than months.

## Browser card

Decision input added 2026-09-14: the browser must run Chrome extensions, specifically Claude in Chrome, and time cost is not a factor. That removes WKWebView, QtWebEngine (Chromium inside, but Qt ships no extension support), Servo and Electron (`session.loadExtension` covers a subset of `chrome.*`, no native messaging, no `chrome.debugger`). Two routes remain.

**Settled 2026-09-14: route 0 passed every check below in the `cef-extension` and `cef-frame` spikes, and Google sign-in passes with glass's moat ported (`chrome_moat.rs`). Route B is not needed.** The unknowns as they were written, kept for the record:

**Route 0, the pick: CEF with the Chrome bootstrap, off-screen rendering, extension loaded unpacked.** CEF's Chrome bootstrap links the `//chrome` layer, which is where the extension APIs live; the old Alloy bootstrap's extension support was removed and is not what this relies on. The unknowns, each a half-day check against the prebuilt `cefclient` before writing any host code:

1. `cefclient --off-screen-rendering-enabled --load-extension=<dir>` with the Claude in Chrome extension unpacked: does the background service worker start? (Fetch the CRX by its Web Store URL, unzip, add the `key` field from the CRX header to `manifest.json` so the extension id matches the one the native messaging host's `allowed_origins` names.)
2. Native messaging: Claude Code writes its host manifest under `~/Library/Application Support/Google/Chrome/NativeMessagingHosts/`. A Chromium-branded binary reads `~/Library/Application Support/Chromium/NativeMessagingHosts/` (verify which path CEF uses); a symlink is the likely fix. The MCP server supports several connected browsers at once (`list_connected_browsers`, `select_browser`), so real Chrome and the card can both be attached.
3. Which `chrome.*` APIs the extension uses for the automation tools (`chrome.debugger`, `tabs`, `scripting`, `captureVisibleTab`) and whether each works in OSR. The side panel (`chrome.sidePanel`) is browser UI and will not exist in an OSR card; the MCP-driven tools do not need it (verify by driving the card from Claude Code with the side panel absent).
4. Frame path: `OnPaint` hands a BGRA buffer per dirty rect; upload as a gpui `RenderImage` each frame. Shortcut: a full 1080p upload is ~8 MB per frame, fine on an M4; the ceiling is several cards at 4K, at which point `OnAcceleratedPaint` (IOSurface) needs a gpui texture path that does not exist today.
5. Bundle: CEF on macOS wants the `Chromium Embedded Framework.framework` and four `infiniterm Helper*.app` bundles under `Contents/Frameworks`, ~200 MB. `ift install` and the sidecars are unaffected.

Language: CEF is C/C++ first. `cef-rs` (the Tauri team's bindings) is the Rust route; if it lacks something, the browser host becomes one small C++ shim with a C ABI (`browser_host/`) and everything else stays Rust. Either way the gpui recommendation stands: the browser is one element that receives a texture and forwards input.

**Route B, the fallback if 1 to 3 fail: real Chrome as a separate process, composited by capture.** Launch Google Chrome with its own `--user-data-dir` (Chrome 136+ refuses remote debugging on the default profile) and `--remote-debugging-pipe`, park its windows behind ours, capture each with ScreenCaptureKit at 60 fps into the card texture, forward the mouse and keyboard through CDP `Input.dispatch*`. Extensions are then simply Chrome's: Web Store, sync, Claude in Chrome with nothing to fiddle. Costs: a visible Chrome process in the Dock and Cmd+Tab, capture latency (one frame), and CDP input rather than native. It zooms and layers correctly because the card only ever paints pixels.

The earlier three routes, kept for the record:

1. **CEF offscreen rendering** (`cef-rs` or the C API through FFI). Chromium paints into a buffer you upload as a gpui texture inside the card element. It zooms, it layers under the palette, the mouse is yours to forward. Cost: the CEF framework bundle (~200 MB), a build-time download, and an event-forwarding layer (~1,500 lines). Kills `mirror.ts` and the overlay window entirely.
2. **WKWebView child `NSView`**, what the spike does today. Reachable from gpui through `raw-window-handle` on the window (verify the exact accessor in the gpui version you pin). Same limits Ekin already accepted: does not scale with zoom, paints above everything, needs the overlay-window mirror for the palette and labels over it. `browser.rs` ports almost line for line with `objc2-web-kit`. Cheapest, and a known dead end for zoom.
3. **Servo** via its embedding API. Rust, composites to a texture like route 1, ~60 MB. Page compatibility is the problem: expect broken sites.

Qt's answer is `QWebEngineView` inside a `QGraphicsProxyWidget`, which is route 1 for free, with documented repaint glitches under transforms. It is the strongest argument for the C++ column.

## Keys and input

| Rule | Native form |
|---|---|
| Chords from `e.code`, not `e.key` (Option+J is `∆`) | gpui's `Keystroke` carries the unmodified key on macOS; verify Option+J and a non-US layout in a spike before porting `keyName` |
| `Cmd` owns every app binding, `ctrl`+digit excepted; `isAllowedChord` decides | same function, run when registering bindings; terminal element passes Cmd chords up untouched |
| Editor keeps Cmd+F/G/Shift+Arrow (`editorKeys.ts`) | gpui key contexts: bindings on the `Editor` context shadow the app ones |
| Hint mode swallows every bare key; phantom is the one time a bare key reaches the app | a focused `Hints`/`Phantom` handle takes focus from the terminal; same exclusivity |
| Cmd+click link vs Cmd+drag pan decided by `DRAG_SLOP`, not pointerdown | same state machine in the canvas element |
| Shift+click extends and must not start a text selection | canvas handles it before the terminal sees mouse down |
| Menu equivalents | you own the menu, so `Cmd+H` becomes bindable; keep Hide/Quit/Edit anyway |

## Persistence and config

Byte-compatible on purpose so a native build opens the existing canvas: `~/Library/Application Support/infiniterm/workspace.json` (`savedLayout.ts` schema, versions, migrations), `drafts/<cardId>.txt`, `~/.config/infiniterm/settings.json` + `settings.default.json` + `keybindings.json` + `keybindings.default.json`, `themes/*.itermcolors`. Palette history parsed separately from cards, as today. UI scale stays in `workspace.json`.

## Estimate (Rust + gpui, solo)

| Step | Concrete unit |
|---|---|
| 1. Glue: `App` struct, socket drain, `ift`/hooks routed to the model, no UI | 3 days |
| 2. Pure modules + tests transliterated (`cargo test` green on ~200 ported cases) | 1 week |
| 3. Canvas + card frame + groups + palette + status/tabs/prompt, with placeholder card bodies | 2 weeks |
| 4. Terminal element on `alacritty_terminal`, links, key handler, theme, 25-card stress at zoom | 1.5 weeks |
| 5. Persistence, settings watcher, theme seeding, drafts, config pair | 3 days |
| 6. Editor: buffer, highlighting, find, save, tree | 3 weeks |
| 7. Diff + blame gutter | 1 week |
| 8. Browser: route 2 (WKWebView) 1 week, route 1 (CEF) 3 weeks |
| 9. Bundle, `ift install`, sidecars, single instance | 3 days |

About 3 months to parity with route 2, 3.5 with CEF. Steps 1 to 3 give a usable canvas of blank cards in under a month, which is the point at which the gpui zoom question is answered for real.

## Spikes before committing (each under a day)

All run 2026-09-14, results in `~/Code/infiniterm/spikes/*/NOTES.md`: 1 passed (120 fps), 3 dropped (WKWebView is not the route), 4 passed (~1 ms per frame, ~200 MB bundle). **2 was never run** and is the first thing the terminal element phase must check.

1. gpui window with one `alacritty_terminal` element at 3 zoom levels: does reshaping hold 60 fps with 25 cards of `yes`?
2. `Keystroke` for Option+J and Cmd+Shift+= on the Turkish-Q layout: does the physical key come through?
3. A WKWebView added as a child of gpui's `NSWindow`: does it appear, does it take the mouse, can it be moved every frame?
4. `cef-rs` OSR hello-world painting into a gpui `img`: does it build, how big is the bundle?
