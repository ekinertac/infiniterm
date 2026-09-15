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

### Phase 3: `infiniterm-ui`, the canvas with blank cards

The Svelte components are wiring; the logic they call is in phase 1. Port `App.svelte` (384), `Canvas.svelte` (545), `CardFrame.svelte` (691), `GroupFrame.svelte` (196), `StateRing.svelte` (83), `Palette.svelte` (282), `ShortcutPanel.svelte` (214), `NamePrompt.svelte` (126), `WorkspaceTabs.svelte` (142), `StatusBar.svelte` (155), `TitleBar.svelte` (71), `SidebarHandle.svelte` (62), and the runes stores (`cards.svelte.ts` 398, `persistence.svelte.ts` 174, `settings.svelte.ts` 176, `theme.svelte.ts` 142, `viewportAnimator.svelte.ts` 165, `workspaces.svelte.ts` 101, `paletteState.svelte.ts` 103, `cardLifecycle.svelte.ts` 97, `groups.svelte.ts` 88, `selection.svelte.ts` 78, `prompt.svelte.ts` 74, `swapAnimation.svelte.ts` 66, `motion.svelte.ts` 46, `uiScale.svelte.ts` 45, `fps.svelte.ts` 42, `notice.svelte.ts` 31, `viewport.svelte.ts` 26, `input.svelte.ts` 12, `env.svelte.ts` 11, `panels.svelte.ts` 5) as gpui entities per the mapping's "Stores" table. Card bodies are a trait with `paint(bounds, scale)` and `input(event)`; this phase implements it with a coloured rectangle.

Start from `spikes/canvas/src/main.rs`: it already has the viewport, focus outline, labels, Cmd chords, pending-drag pan and anchored wheel zoom in gpui.

Done: every `Canvas:`, `Card:`, `Focus:`, `Group:`, `Workspace:`, `App:` command from `keymap.ts` works on blank cards; the existing `~/Library/Application Support/infiniterm/workspace.json` loads and saves back byte-compatible; the palette lists and runs commands with the ranking rules; screenshots checked.

### Phase 4: `infiniterm-term`, the terminal body

From `spikes/canvas/src/terminal.rs` and `spikes/term-zoom`. Reference: `TerminalCard.svelte` (388), `paneRegistry.ts` (246, for the theme replay and refit rules, not its mechanism), `links.ts`, `agentState.ts`, `theme.svelte.ts` (selection colours), `dev.ts` (the stress commands). The mapping's Terminal table lists every feature with the file that defines it: fixed 69x80 cells, refit on metric change, theme, selection colours, links confirmed by the filesystem, Alt/Cmd+Arrow as `ESC b/f` and `^A/^E`, mouse reporting for TUIs, cursor styles, bell, inspect badges. Full key encoding table, not the spike's subset. Chord check for Option+J first (see Spikes).

Done: 25 cards of `yes` at 120 fps under `dev.stress.zoom`; `dev.stress.lines` burst numbers at or above the Tauri app's 6 to 11 MB/s; Cmd chords never reach the shell; `ctrl`+digit does reach the app; a TUI (`htop`, `vim`) works with the mouse.

### Phase 5: persistence, settings, themes, drafts

Persistence gates (`loaded`, `readOnly`, 500 ms debounce), the 1 s config watcher, theme seeding, `patchJsonText` for the theme picker's write, drafts 500 ms after a change and deleted only by `closeCard`, the config pair with read-only defaults, UI scale in `workspace.json`. Mostly wiring phase 1 and 2 pieces to the UI.

Done: a fresh launch on Ekin's real config directory looks like the Tauri app; editing `settings.json` in an external editor applies within a second.

### Phase 6: the editor

`EditorCard.svelte` (643), `Explorer.svelte` (223), `editorRegistry.ts` (59), `editorKeys.ts`, `editorTheme.ts`, `sidebar.ts`. The mapping's Editor table maps every CodeMirror feature to a native piece: `ropey` buffer with own undo, tree-sitter highlighting (pick 20 grammars first), find/replace, go-to-line, comment toggle, untitled buffers, save asks a path, atomic writes, reload clean on disk change, drafts, file tree with dotfiles shown and `.git` hidden, clean-loads / dirty-opens-beside, Cmd+K toggles the tree, badges for dirty/language/readOnly. The biggest phase; no LSP by decision.

Done: `ift <file>`, `ift <dir>`, Cmd+click on a path, the config pair, and drafts across a quit all behave as the Tauri app.

### Phase 7: diff and blame

`DiffCard.svelte` (420), `DiffTree.svelte` (208), `blame.ts`, `git.rs`. Unified merge view over `similar` hunks with the collapse rule, read-only, flat changed-files list with counts, blame gutter in fixed columns.

### Phase 8: transcript card

`TranscriptCard.svelte` (273), `transcript.ts`, `transcript.rs`. Turns from the agent's session JSONL, re-read on mtime change, cursor stays on the last turn.

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

### Phase 10: bundle and install

`.app` with the CEF framework and helpers (`bundle-cef-app` or the same layout by hand), `Info.plist` with an icon key (global rule: launchers drop bundles without one), `ift install`, `ift install-claude-hooks`, `ift install-pi-hooks`, sidecars, single instance by locking the socket path, the app menu (Cmd+H becomes bindable; keep Hide, Quit and the Edit items), `NSWindow` frame autosave, reduced motion from `NSWorkspace`.

Done: `/Applications/infiniterm.app` replaces the Tauri one on Ekin's machine and `ift` works from a shell.

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

Next action: Phase 2, the backend. Read `~/Code/infiniterm/src-tauri/src/lib.rs` and `ipc.rs` first (the glue that is rewritten), then move `backend/`, `inspect.rs`, `git.rs`, `files.rs`, `config.rs`, `themes.rs`, `layout.rs`, `links.rs`, `transcript.rs`, `hooks.rs`, `cli.rs` into `infiniterm-core` behind an `App` model with their 70 tests, and bring `crates/infiniterm-cli` and `crates/infiniterm-hook` into the workspace unchanged. Done when the socket answers `ift ls` and a hook report with no window open.
