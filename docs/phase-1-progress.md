<!-- Phase 1 coverage ledger. Read with HANDOVER.md and the reference source and tests. Records which reference test files are ported and where, not app parity. -->
# Phase 1 progress

Phase 1's pure modules are complete as of 2026-09-15: every reference test file that has a native counterpart is ported, 483 of the reference's 498 cases, plus 18 native checks (501 tests, all passing under `cargo test --offline`). The 15 unported cases are webview mechanics with nothing to port to: `ipc.test.ts` (4, base64 pane transport; bytes cross as bytes here), `tokens.test.ts` (4, chrome colours as CSS tokens; a struct's fields here), `version.test.ts` (1, proves vitest runs), `xtermTransform.test.ts` (6, un-scaling xterm's mouse; hit-testing is ours).

What Phase 1's table listed and is NOT here: `commands/*.ts` (1852 lines, ~70 registrations, no tests of their own). Those are the app model's methods: every one reads and mutates the runes stores (cards, selection, viewport, groups, workspaces, palette state, notices) and calls IPC, the editor registry and the pane registry. They are wiring over the stores and port with the stores in Phase 3, not before; porting them against stand-in state now would mean porting them twice. `commands.rs` (the registry they register into) is here and generic over the app context for exactly that reason.

The reference is the live `~/Code/infiniterm-tauri` worktree; its source wins over comments and planning documents. Nothing there and nothing under `spikes/` was changed.

## Coverage

Reference test paths are relative to `~/Code/infiniterm-tauri/src/lib/`. Rust modules are under `infiniterm-core/src/` unless a crate path is shown.

| Reference tests | Rust module | Reference cases | Native checks |
|---|---|---:|---:|
| `grid.test.ts` | `grid.rs` | 16 | 1 |
| `layout.test.ts` | `layout.rs` | 15 | 0 |
| `resize.test.ts + cardActions.test.ts` | `resize.rs` | 16 | 0 |
| `cardSize.test.ts` | `cards.rs` | 6 | 0 |
| `navigate.test.ts` | `navigate.rs` | 17 | 0 |
| `slots.test.ts` | `slots.rs` | 6 | 0 |
| `split.test.ts` | `split.rs` | 21 | 0 |
| `swap.test.ts` | `swap.rs` | 14 | 0 |
| `multiSelect.test.ts` | `multi_select.rs` | 4 | 0 |
| `groups.test.ts` | `groups.rs` | 20 | 0 |
| `workspaces.test.ts` | `workspaces.rs` | 11 | 0 |
| `zoomActions.test.ts + zoomAnimation.test.ts` | `viewport.rs` | 20 | 3 |
| `momentum.test.ts` | `momentum.rs` | 10 | 0 |
| `panMode.test.ts` | `pan_mode.rs` | 4 | 0 |
| `chrome.test.ts` | `chrome.rs` | 15 | 0 |
| `formatZoom.test.ts` | `format_zoom.rs` | 3 | 0 |
| `agentState.test.ts` | `agent_state.rs` | 7 | 0 |
| `fuzzy.test.ts` | `fuzzy.rs` | 12 | 1 |
| `palette.test.ts` | `palette.rs` | 22 | 1 |
| `paletteUsage.test.ts` | `palette_usage.rs` | 14 | 0 |
| `sidebar.test.ts` | `sidebar.rs` | 6 | 0 |
| `blame.test.ts` | `blame.rs` | 4 | 0 |
| `labelColors.test.ts` | `label_colors.rs` | 11 | 0 |
| `jsonc.test.ts + jsoncPatch.test.ts` | `jsonc.rs` | 21 | 0 |
| `config.test.ts` | `config.rs` | 15 | 0 |
| `settingsDoc.test.ts` | `settings_doc.rs` | 8 | 1 |
| `itermcolors.test.ts` | `itermcolors.rs` | 9 | 2 |
| `savedLayout.test.ts` | `saved_layout.rs` | 35 | 1 |
| `cardLabel.test.ts` | `card_label.rs` | 16 | 1 |
| `ift.test.ts` | `ift.rs` | 11 | 0 |
| `links.test.ts` | `links.rs` | 7 | 0 |
| `transcript.test.ts` | `transcript.rs` | 9 | 0 |
| `editorTheme.test.ts` | `editor_theme.rs` | 7 | 1 |
| `commands.test.ts` | `commands.rs` | 4 | 1 |
| `editorKeys.test.ts` | `editor_keys.rs` | 2 | 0 |
| `browserKeys.test.ts` | `browser_keys.rs` | 1 | 0 |
| `prompt.test.ts` | `prompt.rs` | 6 | 1 |
| `keymap.test.ts` | `keymap.rs` | 30 | 2 |
| `shortcuts.test.ts` | `shortcuts.rs` | 15 | 0 |
| `outputScheduler.test.ts` | `infiniterm-term/src/scheduler.rs` | 9 | 2 |
| `flowControl.test.ts` | `infiniterm-term/src/credit.rs` | 4 | 0 |
| **Total** | | **483** | **18** |

## Port decisions

Pure geometry uses `f64`, matching TypeScript numbers. `grid.rs` owns `Rect`, `Point`, `Size`. UI code converts to rendering coordinates at the boundary.

`saved_layout.rs` builds the file as a `serde_json::Value` in the reference's key order with numbers written as `JSON.stringify` writes them (`25`, not `25.0`), and `layout_text` pretty-prints with two spaces. A test round-trips Ekin's real `workspace.json` byte for byte (skipped where the file is absent). That test caught serde_json parsing a coordinate one ULP off; the `float_roundtrip` feature fixes it. Without it every load/save cycle would drift positions by a bit.

`config.rs` is a typed struct; `settings_doc.rs` renders it through a `Value` (serde_json's `preserve_order` keeps the reference's key order) and prints whole numbers without a decimal point, since people copy from that file.

`itermcolors.rs` deliberately differs from the reference in one place: iTerm writes an exact 0 or 1 component as `<integer>`, and the reference's `<real>`-only regex drops the slot, so Catppuccin Latte, Profile - Default, iTerm Default and Profile - tmux load incomplete in the Tauri app. The port accepts `<integer>`; a test checks all 521 bundled schemes and pins Black Metal (Dissection), which has no ANSI slots, as the one incomplete scheme. Worth fixing in the reference.

`palette_usage.rs` keeps entries in insertion order (a `Vec`), because the reference's ties on equal timestamps resolve by object order.

`links.rs` uses the `regex` crate with the reference's three patterns, `\w`, `\b` and `\d` pinned to ASCII as JavaScript's are. Offsets are byte offsets; the terminal element maps them to cells.

`editor_theme.rs` emits tree-sitter capture names where the reference emitted Lezer tags, and resolved hex where it emitted `color-mix` and CSS variables; the chrome fallback palette is passed in as `Chrome` so one palette feeds the editor and the card frame.

`keymap.rs`: gpui's `Keystroke` has no physical key code. For letters it gives the layout's unshifted character with the shift flag kept; for punctuation and digits it gives the SHIFTED character with the flag cleared; arrows are `left`/`right`/`up`/`down`. `key_name` takes a physical code when the platform has one (the reference's `e.code` path, kept intact) and otherwise un-shifts through a US-layout table; a test re-derives every default chord from what gpui would deliver. On a non-ASCII layout gpui reads the layout's Cmd layer (see `parse_keystroke` in gpui's `platform/mac/events.rs`). Turkish-Q, where the physical `[` key is `ğ`, is the case still to check on a device: whether Cmd+ğ arrives as `[` (Apple's Cmd layer) or `ğ`.

`commands.rs` is generic over the app context and logs every run; the registry knows nothing about cards.

`prompt.rs` and the scheduler keep their reference shapes even where the webview reason is gone (a fire-and-forget prompt, a per-frame byte budget), because the shape is what the rest of the app is written against.

Dependencies added to `infiniterm-core`: `serde`, `serde_json` (`preserve_order`, `float_roundtrip`), `regex`, `chrono` (clock only). All from the local registry cache; `--offline` still works.

## Validation

`cargo test --offline`: 501 tests pass. `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --offline -- -D warnings` pass. No app window exists yet, so this checkpoint has no on-screen result; screen checks start in Phase 3.

## Next

Phase 2: the backend. Move `src-tauri/src` into `infiniterm-core` behind an `App` model (`backend/local_pty.rs`, `inspect.rs`, `git.rs`, `files.rs`, `config.rs`'s watcher, `themes.rs`, `layout.rs`, `links.rs`, `transcript.rs`, `hooks.rs`, `cli.rs`), replacing `#[tauri::command]` with methods and the Tauri channel with an `mpsc` receiver; bring `crates/infiniterm-cli` and `crates/infiniterm-hook` into the workspace unchanged. The 70 existing Rust tests come along. Done when the socket answers `ift` and hook reports with no window open.
