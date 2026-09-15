# infiniterm native port: handover

Written 2026-09-15 at the end of the spike sessions. This is the file to give a fresh session. It holds every decision, every spike result with its numbers, what exists on this Mac, and the port plan file by file. The plan is written so the session can port without asking Ekin what to do next; the one standing instruction is at the top of the next section.

## The rule above every other rule

**`~/Code/infiniterm` is the specification. Read its code, always, for every rule.** No document, this one included, holds the logic; the Tauri app does. Before porting any file: read the source file, read its `.test.ts` beside it, read the paragraph about it in `~/Code/infiniterm/CLAUDE.md` (Non-negotiables and Traps already hit are ~90 rules, and every one that is not about the DOM applies to the port). Port the tests first, then make them pass. If the source and this file disagree, the source wins and this file gets fixed. Do not ask Ekin things the source answers.

Read order for a new session:

1. this file
2. `~/Code/infiniterm/CLAUDE.md` (the reference app's working notes)
3. `docs/port-mapping.md` (file-by-file map; the same text lives at `~/Code/infiniterm/docs/superpowers/specs/2026-09-14-native-port-mapping.md`)
4. `spikes/*/NOTES.md`
5. `CLAUDE.md` in this repo (short standing rules, loaded every session)

## Decisions, all locked

| Decision | What | Why |
|---|---|---|
| Goal | 1:1 native port of the Tauri app; every feature, every rule | Ekin: "map every feature 1:1". The Tauri app stays the shipping app until parity; nothing there changes for the port |
| Time | cost irrelevant | Ekin, 2026-09-14: "ignore the time cost completely" |
| Language and shape | Rust workspace, four crates: `infiniterm-core`, `infiniterm-term`, `infiniterm-browser`, `infiniterm-ui` | gpui is a choice about one crate; if it churns, `ui` is rewritten and nothing else moves |
| UI toolkit | gpui 0.2.2 from crates.io, confined to `infiniterm-ui` | the only GPU toolkit already rendering a terminal and an editor at Zed's scale, in the backend's language. Zed's own terminal/editor views are GPL: read for shape, never copy |
| Terminal engine | `alacritty_terminal` 0.26, `Term` + `vte::ansi::Processor` only | Apache-2.0, complete VT; its `EventLoop` is NOT used (spike 3 below) |
| Output path | reader thread moves bytes into a bounded channel; the UI thread parses a byte budget per frame, round-robin over panes | 1 fps with alacritty's event loop, 120 fps with the app's own shape. `outputScheduler.ts` ports; the mapping's earlier row saying otherwise is overturned |
| Browser card | CEF 152 (cef-rs 152.3.0), Chrome bootstrap, Alloy-style windowless rendering, Claude in Chrome loaded unpacked, painted as a texture | the only embeddable engine that runs Chrome extensions with native messaging and `chrome.debugger`. WKWebView, QtWebEngine, Electron, Servo and "capture a real Chrome" all rejected in the mapping |
| Google sign-in | glass's moat ported (`chrome_moat.rs`): `Emulation.setUserAgentOverride` with brands, plus the `window.chrome` document-start script | Google refused CEF until both were in place; verified with Ekin's own account |
| Zoom of a browser card | CEF lays out at a fixed size; the frame is scaled as a texture | the Tauri app's WKWebView re-lays out per zoom and fights it; the port has no such problem. Above 100% the card should ask CEF for a higher device scale instead of upscaling |
| Cross-platform | macOS first; Linux and Windows kept open by putting `inspect`, the socket, paths and single-instance behind traits | Ekin wants it cross-platform and may one day make the canvas a Linux desktop environment (Wayland compositor on `smithay`; only possible on Linux, confirmed). Consequence for v1: `core` must not know what a card body is |
| Persistence | byte-compatible: same `workspace.json`, drafts, `~/.config/infiniterm/*` | a native build opens the existing canvas |
| Editor | rope + tree-sitter + own view; no LSP, no completion, no lint | Ekin's 2026-09-14 decision in the reference app, carried over |
| Style | global rules apply: header block on every file, comments say why, tests always, YAGNI > KISS > DRY, no attribution trailers anywhere | `~/.claude/CLAUDE.md` |

## Spikes: all five passed

Every spike is a standalone cargo package (its own `[workspace]`) because the CEF build is slow and must not sit in the main workspace until the browser crate is real. Each has a `NOTES.md` with the full account; the numbers here are the ones that matter. Mac mini M4, 4K at 1x, 120 Hz display, release builds.

| # | Spike | Commit | Question | Answer |
|---|---|---|---|---|
| 1 | `spikes/cef-extension` | `a3631de` | does CEF run Claude in Chrome, windowless? | yes: extension loads, native messaging works once the manifests are in `<cache-path>/NativeMessagingHosts/`, Claude Code lists it beside real Chrome and drives it (`select_browser`, `navigate`, `computer screenshot`). Needs `--off-screen-rendering-enabled --use-alloy-style` |
| 2 | `spikes/cef-frame` | `f98843d`, `9d08d05` | can a CEF frame be a gpui texture, zoomed, clicked, with the extension alive? | yes: `on_paint` BGRA to `RenderImage` is a memcpy, 0.7 to 1.5 ms per 1024x768. Click at zoom 1.56 hits the right link. Google sign-in passes with the moat |
| 3 | `spikes/term-zoom` | `9c4f7d1` | 25 `alacritty_terminal` grids under a continuous zoom, idle and flooding? | 85 to 120 fps idle, 118 to 120 flooding at 256 KiB parsed per frame; 5 to 11 ms paint for ~1000 visible lines. alacritty's own event loop gave 1 fps first |
| 4 | (inside 2) | `9d08d05` | Google "not secure" | one CDP UA-metadata override plus one document-start script, sent before the first navigation. Ekin signed in with a 32-character pasted password |
| 5 | `spikes/canvas` | `e08867e` | 12 terminals + 3 browsers in one world, the app's `Cmd+1` / `Cmd+2` / `Cmd+=` / `Cmd+-` / `Cmd+0`, `Cmd+scroll`, `Cmd+drag` | works; Ekin: "it works perfectly". `src/viewport.rs` is `zoomActions.ts` + `zoomAnimation.ts` ported with 12 tests and is the first file of `infiniterm-core` |

What the canvas spike settled beyond the numbers: the Tauri app's viewport maths port as pure functions with no change (`fit_rect` with FIT_PADDING 48, MAX_FIT_SCALE 1, MIN 0.05, MAX 4; `fit_frame` interpolates the centre linearly and the scale geometrically under ease-out-cubic; FIT 240 ms, ZOOM 130 ms; `anchored_viewport` for wheel zoom at the cursor); Cmd+drag is pending until `DRAG_SLOP` 4 px; every card body is one `paint(bounds, scale)` call, which is the trait the mapping asks for.

Never run in the mapping's list: **spike 2, `Keystroke` for Option+J and Cmd+Shift+= on the Turkish-Q layout**. Half answered 2026-09-15 by reading gpui 0.2.2's `platform/mac/events.rs`: `Keystroke` has no key code; `key` is the layout's unshifted character for letters (Option+J gives `j`, good) and the SHIFTED character with shift cleared for punctuation and digits (Cmd+Shift+= arrives as `+`), and on a non-ASCII layout it reads the Cmd layer. `keymap.rs` un-shifts through a US table and a test re-derives every default chord from gpui's shape. Still to check on a device: Turkish-Q, where the physical `[` key is `ğ`; whether Cmd+ğ arrives as `[` decides if `cmd+[` works there without a physical-code hook.

## What exists on this Mac

| Thing | Where | Notes |
|---|---|---|
| CEF binary distribution | `~/.local/share/cef` | 152.0.6, Chromium 152.0.7977.83, macOS arm64 minimal, put there by cef-rs's `export-cef-dir`. Each spike's `.cargo/config.toml` sets `CEF_PATH` to it |
| cef-rs source | `~/Code/cef-rs` | branch `dev` at `4e566e6` (release v152.3.0+152.0.6), cloned 2026-09-15 for `bundle-cef-app`; the crate itself comes from crates.io |
| Claude in Chrome extension, unpacked | `spikes/cef-extension/extension/` | gitignored. A copy of `~/Library/Application Support/Google/Chrome/Default/Extensions/fcoeoabgfenejglbffodgkkbkcdhcgfn/<version>/`; the manifest carries `key`, so the id survives. Recreate with `ditto` if missing (`cp -R` is aliased and skips directories) |
| CEF profile | `spikes/cef-extension/profile/` | gitignored. Holds the claude.ai session (the extension's link to Claude Code is per account), Ekin's Google session, the deviceId, and `NativeMessagingHosts/` with both `com.anthropic.*.json` copied from Chrome's directory. All three spikes with CEF share it |
| Bundled apps | `spikes/*/target/bundle/*.app`, `spikes/term-zoom/target/term-zoom.app` | rebuilt by the commands in each NOTES.md |
| Screenshot helper | `tools/shot.sh`, `tools/winid.swift` | `tools/shot.sh canvas out.png`; builds `tools/winid` on first use |
| `cliclick` | `/opt/homebrew/bin/cliclick` | `cliclick c:x,y t:text kd:cmd ku:cmd` for input when verifying remotely |
| Xcode | installed | gpui needs the Metal shader compiler |

Build and run, the CEF spikes:

```
cd spikes/canvas
cargo build --release
cargo run --manifest-path ~/Code/cef-rs/Cargo.toml -p cef --bin bundle-cef-app -- canvas -o target/bundle
pkill -9 -f "MacOS/canvas"; rm -f ../cef-extension/profile/Singleton*
open --stderr "$PWD/run.log" --stdout "$PWD/run.log" target/bundle/canvas.app
osascript -e 'tell application "canvas" to activate'
```

`bundle-cef-app` reads `[package.metadata.cef.bundle] helper_name` and needs the helper binary (`src/bin/helper.rs`). Check `bundle-cef-app` still accepts the release profile the way it did; the spikes bundled debug builds except term-zoom.

## Verifying without being at the Mac

**Say so first.** Ekin is often on the machine, sometimes testing the same app; a click outside the window deactivates it and keys land wherever focus is. Write one line ("taking over the Mac for ~30 s to run the driver") and wait for a go.

**The driver:** it refuses a locked Mac (`tools/locked`, built from `tools/locked.swift`) and a Mac where another app stays frontmost. `tools/drive/phase3.sh` launches the bundle on a scratch data dir (`/tmp/infiniterm-drive`, a copy of the real canvas; `FRESH=1` for an empty one; `KEEP=1` for a `workspace.json` the scenario wrote itself, as `select.sh` does to get a card small enough to sit at 100%), sends the Phase 3 sequence, screenshots each step into `/tmp/infiniterm-drive/shots/NN-name.png`, and prints the command log. `tools/drive/lib.sh` has the step functions (`cmd t`, `cmd_shift p`, `key_code 36`, `type_text api`, `click x y`, `drag`, `cmd_drag`, `shot name`); a new scenario is a few lines sourcing it. Keys go through System Events, not cliclick: cliclick's typed characters reach gpui with the fn flag and no character, and its Return and Escape never arrive at all (macOS hands non-printing keys to the input context first); cliclick still moves the mouse. Coordinates are window-relative. Screenshots lag the key by ~100 ms, so a 200 ms glide is caught only by luck; trust the `[glide]` log lines (`INFINITERM_KEYLOG=1`).

**Never run the native app on the real data dir while the Tauri app is the shipping one.** `INFINITERM_DATA_DIR` moves the save file, the drafts and the socket; without it the native app reads AND WRITES `~/Library/Application Support/dev.ekinertac.infiniterm/workspace.json`. The first native run did that (2026-09-15, the override missed the layout path after a formatting pass) and Ekin's real canvas got two extra cards, a workspace and a group from the test session; restored from the pre-launch copy. The app now also goes read-only when another instance holds the socket, and `paths.rs` has the guard test.

Ekin is often remote and asked for visual verification ("use computer use"). What worked:

1. Launch through `open` on the `.app`, never the bare binary: a binary started from a shell is not activated and gets no key events.
2. Activate with osascript, then `tools/shot.sh <app> file.png` and read the PNG.
3. Input with `cliclick`. Positions come from the screenshot, at 1x on the 4K display.
4. gpui does not draw while the Mac is locked, and a window created behind the lock screen never resumed. If `run.log` shows one draw and nothing after, ask Ekin to unlock and restart the app.
5. One CEF process per profile. A second one hands off to the first and `initialize` returns 0 ("Opening in existing browser session"). `pkill -9` every cefclient/spike/helper and remove `profile/Singleton*` before the next launch.
6. The output-filtering hook in this environment hides long tool outputs. Use `tail`, `grep`, and truncate `run.log` per run.

## The port plan

Ten phases. Each names the reference files (line counts from 2026-09-15), the target, and what "done" means. The phase is done when its tests pass and the done-check has been verified on screen, not before. Commit per phase at least.

### Phase 0: workspace

Create the four crates at the repo root, `cargo test` green on nothing. Move `spikes/canvas/src/viewport.rs` into `infiniterm-core` as the first module. The spikes stay untouched as reference until the code that replaces each is in a crate. `Cargo.toml` at the root is the workspace; the spikes keep their own `[workspace]` and are not members (CEF build time) until `infiniterm-browser` exists.

Completed 2026-09-15. The root workspace contains `infiniterm-core/`, `infiniterm-term/`, `infiniterm-browser/`, and `infiniterm-ui/`. The empty workspace passed first; with the viewport module, `cargo test --offline` passes all 12 existing tests. The earlier count of 11 was incorrect. The viewport implementation and tests retain the spike's behavior, with formatting and header changes only. The canvas spike keeps its copy until the full canvas has a replacement. All spikes remain separate workspaces. The browser, terminal, and UI crates are placeholders with no external dependencies. This phase has no app window to check; screen checks begin when the UI exists.

Phase 1 completed the viewport comparison against all 20 reference cases, including the near/far centering case. Optional fit padding and `PAN_DURATION_MS` are now present.

### Phase 1: `infiniterm-core`, the pure modules

Every pure TS module becomes `core/src/<name>.rs` with the same test cases in `#[cfg(test)]`. 498 cases across 48 test files today. Order: geometry first, because everything else is tested against it. Read every source file; the notes column is only what the mapping and the spikes add.

| Reference (lines) | Tests | Target | Notes |
|---|---|---|---|
| `grid.ts` (90) | 16 | `grid.rs` | cell maths; `cardSize.test.ts` (6) pins the fixed 69x80 card and belongs to `cards` below |
| `layout.ts` (115) | 15 | `layout.rs` | card placement, first free slot AFTER the active card; other groups' frames are occupied. Not the save file |
| `resize.ts` (59), `cardActions.ts` (22) | 11, 5 | `resize.rs` | keyboard resize reuses `applyResize` from the 'se' edge |
| `navigate.ts` (167) | 17 | `navigate.rs` | arrow focus, `nearestTo` by centre distance, phantom slot checked before a farther card |
| `slots.ts` (50) | 6 | `slots.rs` | free default-sized slots around cards, lettered in reading order |
| `split.ts` (164) | 21 | `split.rs` | soft groups, `reclaim` from rects, `splitFrom` first; keep the 2x2 tests |
| `swap.ts` (78) | 14 | `swap.rs` | whole rect, same group only |
| `multiSelect.ts` (61) | 4 | `multi_select.rs` | extend and reverse |
| `groups.ts` (162) | 20 | `groups.rs` | frame derived from `groupId`; aggregate state, `working` outranks `idle` |
| `workspaces.ts` (96) | 11 | `workspaces.rs` | `cardsOn(workspace)` stays an argument, never a global |
| `zoomActions.ts` (65), `zoomAnimation.ts` (90), `momentum.ts` (54) | 10, 10, 10 | `viewport.rs` (from the spike) + `momentum.rs` | the spike's 12 tests cover most of the first two; port the originals' cases too and diff |
| `panMode.ts` (18) | 4 | `pan_mode.rs` | the one predicate both sides ask |
| `chrome.ts` (94) | 15 | `chrome.rs` | screen-px sizing divided by zoom; smaller natively but the tests stay |
| `formatZoom.ts` (11) | 3 | `format_zoom.rs` | never "0%" |
| `savedLayout.ts` (361) | 35 | `saved_layout.rs` | serde structs, every version migration, `isNewerThanThisBuild`; palette history parsed separately from cards |
| `config.ts` (227), `settingsDoc.ts` (158), `jsonc.ts` (296) | 15, 8, 11+10 | `config.rs`, `settings_doc.rs`, `jsonc.rs` | `stripComments` pads, `patchJsonText` edits in place; `undocumented()` test kept; defaults in three places must agree (config, first paint, terminal fallback) |
| `itermcolors.ts` (77) | 9 | `itermcolors.rs` | or the `plist` crate; keep the tests either way |
| `keymap.ts` (341), `shortcuts.ts` (187), `commands.ts` (49) | 30, 15, 4 | `keymap.rs`, `shortcuts.rs`, `commands.rs` | the reason beside each binding; `isAllowedChord` the one gate; `keybindings.default.json` generated; chord spelling `Cmd Shift T` |
| `editorKeys.ts` (31), `browserKeys.ts` (20) | 2, 1 | `editor_keys.rs`, `browser_keys.rs` | which chords a focused editor or browser card keeps |
| `fuzzy.ts` (75), `palette.ts` (166), `paletteUsage.ts` (99) | 12, 22, 14 | `fuzzy.rs`, `palette.rs`, `palette_usage.rs` | `HINT_PENALTY`; true total for truncation; recency and frequency apart; use recorded on run |
| `agentState.ts` (58), `cardLabel.ts` (102), `labelColors.ts` (94) | 7, 16, 11 | `agent_state.rs`, `card_label.rs`, `label_colors.rs` | two states; label = name, else process, else directory; colour hashed from id, red excluded |
| `ift.ts` (111), `links.ts` (65) | 11, 7 | `ift.rs`, `links.rs` | `OpenPlan` is the one path for `ift <path>`, Cmd+click and the typed prompt |
| `transcript.ts` (48), `blame.ts` (39), `editorTheme.ts` (98) | 9, 4, 7 | `transcript.rs`, `blame.rs`, `editor_theme.rs` | editor theme maps ANSI slots to tree-sitter capture names instead of Lezer tags |
| `sidebar.ts` (44) | 6 | `sidebar.rs` | per-card sidebar width in card px, clamp both halves, beside or above |
| `outputScheduler.ts` (138), `flowControl.ts` (56) | 9, 4 | `infiniterm-term/src/scheduler.rs`, `credit.rs` | the budget controller: 32 KiB floor, 30% cut, four panes per frame, quiet spells not stalls. Goes in `term`, not `core`, but port it in this phase |
| `commands/*.ts` (1852 across 7 files) | | `commands/*.rs` by domain | ~60 commands; `Domain: what it does` labels; a command goes in the module whose prefix it carries; `dev.ts` stress harness ports too |
| `xtermTransform.ts`, `paneRegistry.ts`, `mirror.ts`, `tokens.test.ts`, `version.ts` | | not ported | webview workarounds; the mapping's "What disappears" table says why for each |

Done: `cargo test -p infiniterm-core` green with the ported cases, and a count in the commit message against the 498.

Progress 2026-09-15, end of day: Phase 1's pure modules are COMPLETE. 483 of the reference's 498 test cases are ported plus 18 native checks (501 tests, `cargo test --offline`, clippy `-D warnings` clean). The 15 unported cases are webview mechanics with no counterpart (ipc base64 transport, tokens.test, version, xtermTransform). `docs/phase-1-progress.md` has the file-by-file table and the port decisions. Two things from this phase to know: `saved_layout.rs` round-trips Ekin's real `workspace.json` byte for byte (that test found serde_json parsing floats one ULP off; `float_roundtrip` fixes it), and `itermcolors.rs` accepts `<integer>` components, which the reference's regex drops, so four bundled schemes that load incomplete in the Tauri app load complete here (a reference bug worth fixing there).

`commands/*.ts` is NOT ported in this phase and moves to Phase 3: every command reads and mutates the runes stores and calls IPC, the editor registry and the pane registry, so it is wiring over the stores and ports with them. `commands.rs` (the registry) is here, generic over the app context.

### Phase 2: `infiniterm-core`, the backend

The Rust backend moves in nearly unchanged. Replace `#[tauri::command]` functions with methods on an `App` model and the Tauri `Channel` with an `mpsc` receiver the UI drains each frame. The 70 existing `#[test]`s come along.

| File (lines) | Change |
|---|---|
| `backend/local_pty.rs` (490), `backend/mod.rs` (49) | none. One tagged stream for all panes, `Credit` 256 KiB per pane. The reader thread already does what the term-zoom spike wants |
| `inspect.rs` (473) | none on macOS; behind a trait for `/proc` and Windows later |
| `git.rs` (258), `files.rs` (203), `config.rs` (223), `themes.rs` (91), `layout.rs` (64), `links.rs` (105), `transcript.rs` (272) | none |
| `hooks.rs` (170), `cli.rs` (182) | none on the wire; the verbs `cli.rs` forwarded to the webview call the model directly, so the 2 s wait goes |
| `ipc.rs` (194), `lib.rs` (116), `main.rs` (6) | rewritten: this is the glue |
| `browser.rs` (331) | not ported; it is the WKWebView half. Read it for behaviour (focus scrim, snapshot when unfocused, page zoom keys), not mechanism |
| `crates/infiniterm-cli` (733 incl. `adapters/pi.ts`), `crates/infiniterm-hook` (61) | move into the workspace unchanged; same socket path, same JSON |
| `resources/themes/*` (521 files) | bundled, `seed()` semantics kept |

Done: the socket answers `ift` and hook reports with no window open; `cargo test` across the workspace green.

Completed 2026-09-15. Everything moved as it was, minus the Tauri attributes: `backend/` (8 tests), `paths.rs` (every directory the app touches, one seam for per-OS answers later), `config_files.rs`, `layout_file.rs`, `files.rs`, `links_fs.rs`, `themes_files.rs`, `git.rs`, `inspect.rs`, the transcript parser (merged into `transcript.rs` beside the formatting), `cli.rs`, `hooks.rs`. The glue is `app.rs`: `Backend::start(socket_path)` spawns the PTY backend, the socket listener, the process poller and the config watcher, each feeding an `mpsc` receiver the UI thread drains once per frame (gpui's model is not `Send`, so receivers rather than callbacks). The done-check is a test in `app.rs`: a hook report and an `ift ls` round-trip through a temp socket with no window. `infiniterm-cli` (12 tests) and `infiniterm-hook` are workspace members, unchanged; the Tauri repo's copies stay the shipping ones. Workspace total: 581 tests. Not moved: `browser.rs` (WKWebView) and `menu.rs` (Tauri menu; the gpui menu is Phase 10).

### Phase 3: `infiniterm-ui`, the canvas with blank cards

The Svelte components are wiring; the logic they call is in phase 1. Port `App.svelte` (384), `Canvas.svelte` (545), `CardFrame.svelte` (691), `GroupFrame.svelte` (196), `StateRing.svelte` (83), `Palette.svelte` (282), `ShortcutPanel.svelte` (214), `NamePrompt.svelte` (126), `WorkspaceTabs.svelte` (142), `StatusBar.svelte` (155), `TitleBar.svelte` (71), `SidebarHandle.svelte` (62), and the runes stores (`cards.svelte.ts` 398, `persistence.svelte.ts` 174, `settings.svelte.ts` 176, `theme.svelte.ts` 142, `viewportAnimator.svelte.ts` 165, `workspaces.svelte.ts` 101, `paletteState.svelte.ts` 103, `cardLifecycle.svelte.ts` 97, `groups.svelte.ts` 88, `selection.svelte.ts` 78, `prompt.svelte.ts` 74, `swapAnimation.svelte.ts` 66, `motion.svelte.ts` 46, `uiScale.svelte.ts` 45, `fps.svelte.ts` 42, `notice.svelte.ts` 31, `viewport.svelte.ts` 26, `input.svelte.ts` 12, `env.svelte.ts` 11, `panels.svelte.ts` 5) as gpui entities per the mapping's "Stores" table. Card bodies are a trait with `paint(bounds, scale)` and `input(event)`; this phase implements it with a coloured rectangle.

Start from `spikes/canvas/src/main.rs`: it already has the viewport, focus outline, labels, Cmd chords, pending-drag pan and anchored wheel zoom in gpui.

Progress 2026-09-15 (third session): the model is `infiniterm-core/src/model/` (every store and command, 29 tests, commit `9609111`); the gpui side is `infiniterm-ui/src/` (`main.rs` view and frame loop, `paint.rs`, `input.rs`, `overlays.rs`, `animator.rs`, `field.rs`, `chrome.rs`, `body.rs`; commit `3d12fe2` and after). Verified on a copy of Ekin's real canvas by him and by the driver: fit-all, new card, hints, rename (prompt opens with the suggestion selected, Cmd+A), swap with the glide, split and reclaim on close, group to a free block with its frame, workspaces with their viewports, the palette with sections and chords, the shortcuts panel (scrolls), focus rings, group frames. Bodies are blank until Phase 4. Also verified through the driver (`tools/drive/phantom.sh`, `drag.sh`, `panel.sh`): the phantom and its slot menu, slot picking with letters, Cmd+drag pan with momentum, a top-edge drag with alignment guides and the no-overlap drop, the shortcuts panel. Two additions over the reference at Ekin's ask (the Tauri session was told): alignment guides while dragging (`alignment.rs`) and a card dropped over another put back (`Model::end_gesture`). Not yet checked on screen: the wheel zoom (no tool sends scroll events), the title bar's dots (needs an agent), reduced motion from the system (Phase 10). `tools/bundle.sh` wraps the binary in a `.app` with the 521 themes as resources.

Done: every `Canvas:`, `Card:`, `Focus:`, `Group:`, `Workspace:`, `App:` command from `keymap.ts` works on blank cards; the existing `~/Library/Application Support/infiniterm/workspace.json` loads and saves back byte-compatible; the palette lists and runs commands with the ranking rules; screenshots checked.

### Phase 4: `infiniterm-term`, the terminal body

From `spikes/canvas/src/terminal.rs` and `spikes/term-zoom`. Reference: `TerminalCard.svelte` (388), `paneRegistry.ts` (246, for the theme replay and refit rules, not its mechanism), `links.ts`, `agentState.ts`, `theme.svelte.ts` (selection colours), `dev.ts` (the stress commands). The mapping's Terminal table lists every feature with the file that defines it: fixed 69x80 cells, refit on metric change, theme, selection colours, links confirmed by the filesystem, Alt/Cmd+Arrow as `ESC b/f` and `^A/^E`, mouse reporting for TUIs, cursor styles, bell, inspect badges. Full key encoding table, not the spike's subset. Chord check for Option+J first (see Spikes).

Done: 25 cards of `yes` at 120 fps under `dev.stress.zoom`; `dev.stress.lines` burst numbers at or above the Tauri app's 6 to 11 MB/s; Cmd chords never reach the shell; `ctrl`+digit does reach the app; a TUI (`htop`, `vim`) works with the mouse.

Progress 2026-09-15 (fourth session), commits `d005b3a` to the one after `463e9a3`: real shells run in the cards (`infiniterm-term/src/grid.rs` wraps alacritty's `Term`; `infiniterm-ui/src/terminal_body.rs` paints it; `terminals.rs` spawns, refits and feeds through the scheduler). Verified through the driver (`tools/drive/terminal.sh`, `select.sh`, `stress.sh`): typing reaches the focused shell, `ls`/`fastfetch`/`htop` render, `htop` takes keys and the mouse, Cmd+K clears, links underline, a drag selects text (two clicks a word, three a line) and Cmd+C pastes back with Cmd+V, the cursor blinks in the focused card and stands hollow in the others, unfocused cards carry the dim scrim, `dev.stress.zoom` and `dev.stress.dims` exist (core drives the ten steps from `tick`, the ui logs the fps). Not verified on screen: the spawn-error panel (a missing directory does not fail: portable-pty starts the shell in the home directory instead, in the reference too) and OSC 52 (wired to the clipboard). No bell handling: the reference has none. Numbers, after the damage-tracked frame (`Grid::update_frame` rebuilds only the rows alacritty marks damaged, and links are rescanned only for those): 26 cards flooding paint at 110 fps on the 120 Hz display (reference 45 to 60; it was 22 when every row of every card was rebuilt and trimmed each frame), `dev.stress.zoom` with 26 cards reads 55 to 105 fps per step (reference 60; what remains is gpui rasterising every glyph at each new size of the animation), `dev.stress.lines` at 68 fps during the burst. An idle canvas paints twice a second (the blink); a notice or a pending save no longer holds the frame loop open, `tools/drive/idle.sh` checks. The `[paint]` line in `run.log` under `INFINITERM_KEYLOG` says where a frame goes (feed, frame build, links, shaping, glyphs). The dev profile optimises dependencies (`Cargo.toml`), or the numbers are the debug build of gpui's. Remaining from the done-list: the Option+J chord check on Ekin's Turkish-Q layout (deferred by him).

### Phase 5: persistence, settings, themes, drafts

Persistence gates (`loaded`, `readOnly`, 500 ms debounce), the 1 s config watcher, theme seeding, `patchJsonText` for the theme picker's write, drafts 500 ms after a change and deleted only by `closeCard`, the config pair with read-only defaults, UI scale in `workspace.json`. Mostly wiring phase 1 and 2 pieces to the UI.

Done: a fresh launch on Ekin's real config directory looks like the Tauri app; editing `settings.json` in an external editor applies within a second.

Progress 2026-09-15 (fourth session): most of it was already wired in Phases 2 and 3 (`runtime::startup` rewrites the generated files, seeds the themes, loads the layout behind `loaded`/`read_only`, prunes drafts; `drain_backend` applies the 1 s config watcher; `Effect::SaveSetting` patches the JSON text; `startingDir` is honoured). Added: the window frame in `window.json` beside `workspace.json` (`infiniterm-ui/src/window_state.rs`, written half a second after it stops changing, restored at launch; the mapping's replacement for the window-state plugin), an app menu with Quit on Cmd+Q whose quit flushes both saves, and `INFINITERM_CONFIG_DIR` so a test edits a copy of the config directory rather than the one the Tauri app watches. Verified through `tools/drive/settings.sh`: a `fontSize` and `theme` edit in `settings.json` shows within two seconds (the grid refits and the PTY is resized on the way), and the window comes back at the position it was quit at. Drafts and the config pair are the editor's (Phase 6). Not verified: `uiScale` on labels (the commands and the save exist).

### Phase 6: the editor

`EditorCard.svelte` (643), `Explorer.svelte` (223), `editorRegistry.ts` (59), `editorKeys.ts`, `editorTheme.ts`, `sidebar.ts`. The mapping's Editor table maps every CodeMirror feature to a native piece: `ropey` buffer with own undo, tree-sitter highlighting (pick 20 grammars first), find/replace, go-to-line, comment toggle, untitled buffers, save asks a path, atomic writes, reload clean on disk change, drafts, file tree with dotfiles shown and `.git` hidden, clean-loads / dirty-opens-beside, Cmd+K toggles the tree, badges for dirty/language/readOnly. The biggest phase; no LSP by decision.

Done: `ift <file>`, `ift <dir>`, Cmd+click on a path, the config pair, and drafts across a quit all behave as the Tauri app.

Progress 2026-09-15 (fourth session): built. `infiniterm-editor/` is the logic crate (no gpui): `buffer.rs` (ropey, one cursor, undo grouped by CodeMirror's 500 ms rule, close-brackets, indent, comment toggle, 14 tests), `search.rs` (literal, smart case), `language.rs` (fifteen tree-sitter grammars compiled in, the reference's alias table, JSON on the JavaScript grammar), `highlight.rs` (tree-sitter-highlight spans over the whole text per version), `explorer.rs` (the lazy tree, keys, tested with a fake filesystem), `wrap.rs` (prose wrapping at the word), `diff.rs` (Phase 7). `infiniterm-ui/src/editor_body.rs` draws it: gutter, syntax runs from the theme's rules, selection, matches, bracket pair, block cursor, the find/replace panel, the tree beside or above; `editors.rs` keeps every body in step with its card, the settings and the theme and copies dirty/language/read-only back for the badges. Model: `card.save` on an untitled editor prompts `Pending::SaveAs`, `editor.goToLine` prompts `Pending::GoToLine` (tests in `register.rs`). Verified through `tools/drive/editor.sh`: highlighting and the gutter on a Rust file, typing and undo, find with the current match, the tree opening a file, save-as, the config pair with read-only defaults, prose wrapping on a README, and a draft coming back after a quit. Not built: fold gutter and selection-match highlighting (CodeMirror's free extras), up/down by visual row in wrapped prose, the sidebar drag handle (the `card.sidebar.*` commands work), `ift <file>` against the port (the cli copy talks to `/tmp/infiniterm.sock`, the Tauri app's; Phase 10).

### Phase 7: diff and blame

`DiffCard.svelte` (420), `DiffTree.svelte` (208), `blame.ts`, `git.rs`. Unified merge view over `similar` hunks with the collapse rule, read-only, flat changed-files list with counts, blame gutter in fixed columns.

Progress 2026-09-15 (fourth session): built. `infiniterm-editor/src/diff.rs` turns HEAD's text and the working tree's into rows (context, added, deleted, collapsed) with the merge view's rule (margin 3, minSize 4), tested; `infiniterm-ui/src/diff_body.rs` draws them with the theme's green and red washes, the change bar, the collapse markers, syntax runs on the current file's lines, and the blame gutter from `git.rs`/`blame.rs` on Cmd+B; the changed files list with counts sits where the editor's tree does, Cmd+K walks the same three states. Verified through `tools/drive/diff.sh` on this repo's own changes. Bodies paint under a content mask now; a wash reached the next card before.

### Phase 8: transcript card

`TranscriptCard.svelte` (273), `transcript.ts`, `transcript.rs`. Turns from the agent's session JSONL, re-read on mtime change, cursor stays on the last turn.

Progress 2026-09-15 (fourth session): built, `infiniterm-ui/src/transcript_body.rs`: the turns list (who, preview, time) beside the chosen turn's text wrapped to the width and its tool calls folded behind their summary, Enter unfolds; the file is re-read on mtime every two seconds and the cursor stays on the last turn. Verified through `tools/drive/transcript.sh`, which seeds a card over a real session file since a driver run has no hook events.

### Phase 9: `infiniterm-browser`, the browser card

From `spikes/canvas/src/browser.rs` and `chrome_moat.rs`. What the spikes did not do, listed in the NOTES:

1. `on_before_popup` and windows the extension creates (`chrome.windows`): each becomes a card, never a native window.
2. Device scale: when the card is drawn above 1.0, tell CEF a higher `device_scale_factor` instead of upscaling the texture.
3. The full key map (Windows virtual key codes for everything, not the spike's ten keys), and `sendEvent:` wrapped to set the `CefAppProtocol` flag if nested-run-loop bugs appear.
4. The profile lives in the app's Application Support directory; at startup the app copies the two Anthropic native-messaging manifests from Chrome's directory into `<cache-path>/NativeMessagingHosts/`, so a Claude Code update is picked up on the next launch.
5. Extension loading through `CefRequestContext` or `--load-extension`, from a copy the app keeps.
6. Behaviour from the reference: `BrowserCard.svelte` (214), `browserKeys.ts` (page zoom on the zoom chords when a browser card is focused), the unfocused scrim taking the first click, arrow-focus never giving the page focus, Cmd+click on a URL opening a card beside, Cmd+Shift+click sending to the system, a card saved without a url restored as a terminal.
7. `on_accelerated_paint` (IOSurface) only if 4K cards are wanted; gpui has no public texture path today.

Done: Claude Code drives a card through the extension (`list_connected_browsers` shows it), Google sign-in passes, three browser cards under a zoom animation hold the frame rate.

Progress 2026-09-15 (fourth session): built, NOT yet seen on screen (the Mac was locked). `infiniterm-browser/` is the two spikes made permanent: `process.rs` (framework from the bundle, `execute_process` for helpers, the profile under `<data>/browser/profile` with Chrome's two Anthropic native-messaging manifests copied in at every start, the extension seeded from Chrome's install into `<data>/browser/extension`, `--load-extension`, the pump, shutdown), `surface.rs` (one windowless browser per card: view size and device scale from the card, BGRA frames with a dirty flag, popups refused and queued as urls, title and address, the input encoders, page zoom), `moat.rs` as it was, `app_protocol.rs`. `infiniterm-ui/src/browser_body.rs` paints the frame as a texture, keeps the reference's focus rule (the first click on an unfocused card only focuses), forwards keys and the edit chords; `browsers.rs` opens a surface per browser card, follows `card.url` and `card.zoom`, writes the page's address back, opens popups beside. `main.rs` runs CEF before gpui (a helper invocation exits there; without the framework the app runs and browser cards say so) and pumps it every 4 ms. Items 1, 4, 5 and 6 of the list above are in; 2 (a higher device scale above 100%) and 3 (the full key map beyond the named keys and text; `sendEvent:` wrapping) and 7 wait. `tools/drive/browser.sh` is the check to run first when the Mac is unlocked.

### Phase 10: bundle and install

`.app` with the CEF framework and helpers (`bundle-cef-app` or the same layout by hand), `Info.plist` with an icon key (global rule: launchers drop bundles without one), `ift install`, `ift install-claude-hooks`, `ift install-pi-hooks`, sidecars, single instance by locking the socket path, the app menu (Cmd+H becomes bindable; keep Hide, Quit and the Edit items), `NSWindow` frame autosave, reduced motion from `NSWorkspace`.

Done: `/Applications/infiniterm.app` replaces the Tauri one on Ekin's machine and `ift` works from a shell.

Progress 2026-09-15 (fourth session): `tools/bundle.sh` now goes through cef-rs's `bundle-cef-app` (framework, four helper apps, plist) and adds the themes, the icon keys and `ift` / `infiniterm-hook` as sidecars in `Contents/MacOS`; `make bundle` and `make release`. The app menu is the reference's by hand (`main.rs`: Hide, Hide Others, Show All, Quit; Window: Minimize, Zoom; no File, no Edit), so Cmd+H, Cmd+Alt+H, Cmd+M and Cmd+Q are the unbindable chords here too. One instance: a second launch that finds the socket held activates the first through its bundle id and exits (`another_instance_holds_the_socket`). The window frame is `window.json` (Phase 5). Reduced motion comes from `NSWorkspace` at launch and outranks `ui.animations`. `ift` and the hook binary honour `INFINITERM_DATA_DIR` for their socket, the one divergence from the reference's copies (four lines each; the Tauri app never sets the variable, so its behaviour is unchanged; mirror it there when convenient). `tools/drive/ift.sh` drives a scratch instance headless (ls, name, a file at a line, a diff, two hook events) and passes; it is the check that runs on a locked Mac. Not done: `ift install` against this bundle on Ekin's machine (his shell still points at the Tauri app's `ift`), and replacing `/Applications/infiniterm.app`, which is his call once the browser card is seen working.

## Standing rules while porting

- Stage by file name, never `git add -A`; commit messages say why; no attribution trailers of any kind. Both from the reference repo and the global rules.
- Another Claude session works in `~/Code/infiniterm`. Read there, do not write there. The mapping copy in this repo is the one to edit.
- Every file starts with a header block: responsibility, where it fits, what calls it, related files, constraints.
- Anything with real logic is a pure function in its own file with tests; the gpui elements are wiring. The reference's "where does X live" rule.
- Keep the spikes as they are until their replacement is in a crate; then delete the spike in the same commit.
- Verify on screen before calling a phase done. The tools are in the Verifying section.

## Things that bit during the spikes

Consolidated from the four NOTES files; each is the short form.

- `cefclient --help` opens the GUI and hangs.
- `cp -R` is aliased and skipped a directory; use `ditto`. `ls` is aliased; `command ls`. A zsh `=word` in an echo is a glob error.
- CEF reads native-messaging manifests from `<cache-path>/NativeMessagingHosts/`, not Chrome's or Chromium's directory.
- `--off-screen-rendering-enabled` alone stays on Chrome style and the extension's onboarding tab crashed; `--use-alloy-style` is required for real OSR.
- Both `cef` and `gpui` glob-export `App`, `Window`, `Point`, `MouseEvent`; import gpui by name.
- `RenderImage.scale_factor` is crate-private; `cef_event_flags_t` is a newtype (`.0 as u32`); the `objc` macros need `use objc::{class, msg_send, sel, sel_impl}`.
- CEF aborts on `-[GPUIApplication isHandlingSendEvent]: unrecognized selector` about ten seconds in; `cef_app_protocol::install` adds the two methods at runtime.
- The browser must be created inside `open_window`'s builder: CEF asks for the device scale factor at creation.
- Version constants are `cef::sys::CHROME_VERSION_*`, not `CEF_CHROME_VERSION_*`.
- `navigator.userAgentData` exists only in secure contexts; probe on an https page, not a `data:` URL.
- Spike keys on bare letters stole typing from the page; every app chord sits behind Cmd, as in the reference. Cmd+V had to be `frame.paste()` before a password could go in.
- `alacritty_terminal::event_loop` under flood: 1 fps. See the term-zoom NOTES.

## State of the repos at handover

- `~/Code/infini-rust`: branch `master`, no remote, commits `a3631de` `f98843d` `9c4f7d1` `9d08d05` `e08867e` plus this handover. No crates yet; `spikes/` only.
- `~/Code/infiniterm`: `master` at `67f360f` plus the other session's uncommitted work (browser card as WKWebView, browserKeys, sidebar). The mapping spec there is untracked; the copy at `docs/port-mapping.md` here is the committed one. Its `CLAUDE.md` has a paragraph on the port pointing here.
- The canvas spike app may still be running on the Mac: `pkill -9 -f "MacOS/canvas"`, then remove `spikes/cef-extension/profile/Singleton*` before the next CEF launch.

Paused 2026-09-15 (first session) at Ekin's request to conserve the usage allowance; resumed the same day in a second session, which finished Phase 1's pure modules (commits from `Palette usage history` to `The shortcut list`). No build, app launch or background task is running for this port. The two pre-existing untracked files, `spikes/cef-extension/cefclient.log` and `console.log`, remain untouched.

Next action: run `tools/drive/browser.sh` on an unlocked Mac and fix what it shows (the browser card has never been seen); then `tools/drive/phase3.sh`, `terminal.sh`, `editor.sh` once more on the CEF bundle to be sure nothing regressed under the pump; then the spikes can be deleted (their replacements exist); then Ekin decides about `/Applications`. The Turkish-Q chord check stays open.
