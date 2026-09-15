# infini-rust: working notes for agents

The native port of infiniterm: Rust + gpui for the app, CEF for the browser card, `alacritty_terminal` for the grid and VT parser. Terminal, editor, diff, transcript and browser cards on an infinite zoomable canvas, driven by Claude Code hooks and `ift`.

`HANDOVER.md` is the state of the project and the phase plan; read it first in a new session. `docs/port-mapping.md` is the file-by-file map. `spikes/*/NOTES.md` hold the measurements the stack decision rests on.

## The specification is the other repo

`~/Code/infiniterm` (Tauri 2 + Svelte 5) is the shipping app and the spec for every feature and rule. Before porting a file, read its source and its `.test.ts` there, and the paragraph about it in `~/Code/infiniterm/CLAUDE.md`. Port the tests first. When a document here disagrees with that code, the code wins and the document is fixed. Do not write in that repo: another session works there.

The rules in the reference's Non-negotiables and Traps that are not about the DOM apply here unchanged. The ones that shaped the port most: `Cmd` owns every app binding except `ctrl`+digit; chords are built from the physical key; positions are permanent; every viewport change is animated; one tagged event stream for all panes; nothing is mouse-only; `core` knows rects and ids, never what a card body draws.

## Layout

```
HANDOVER.md                 state, decisions, spikes, the ten phases
docs/port-mapping.md        every reference file and where it lands
Cargo.toml                 workspace for the four native crates
infiniterm-core/src/        every pure module of the reference, and the backend (PTY, socket, git, inspect, files) behind app::Backend
infiniterm-cli/, infiniterm-hook/   ift and the hook binary, as they were
infiniterm-term/src/        alacritty grid + frames (grid.rs), palette, key/mouse encoding, output scheduler, ack ledger; no gpui
infiniterm-editor/src/      the editor's logic, no gpui: buffer (ropey + undo), search, language (15 tree-sitter grammars), highlight, explorer, wrap, diff
infiniterm-browser/src/     CEF: process (framework, helpers, profile, extension, pump), surface (one browser: frames, input, popups), moat, app_protocol; no gpui
infiniterm-ui/src/          the gpui app: main.rs (AppView, CEF startup, the menu), runtime.rs (effects, backend drain), paint.rs (one frame), input.rs, overlays.rs (palette, prompt, panel), body.rs (CardBody trait), terminal_body.rs + terminals.rs, editor_body.rs + diff_body.rs + transcript_body.rs + editors.rs, browser_body.rs + browsers.rs, bin/helper.rs (the CEF helper), window_state.rs, field.rs, animator.rs, chrome.rs, text.rs
spikes/*/NOTES.md           the four spikes' measurements; their code is gone (git log has it). cef-extension/profile (gitignored) seeds the app's browser profile once
tools/shot.sh               screenshot one app's window for remote verification
tools/bundle.sh             wrap the binary in target/bundle/infiniterm.app with the themes
tools/drive/                the GUI driver: lib.sh (drive_start, cmd, key, type_text, click, drag, shot, quit), scenarios phase3 panel drag phantom terminal select stress idle settings editor diff transcript
Makefile                    run, run-fresh, stop, log, test, check, fmt, clippy, drive*, shot, release
```

Every phase is built and checked through the driver, the browser card included (`tools/drive/browser.sh`; Claude Code's `list_connected_browsers` sees the card's extension). The spikes' code is deleted; `spikes/*/NOTES.md` keep the measurements. `make bundle` now needs `~/Code/cef-rs` (its `bundle-cef-app` lays out the CEF framework and helpers) and takes about a minute; the app runs only from the bundle (the framework is loaded from beside the executable). Editor, diff and transcript cards draw from `infiniterm-editor`; `tools/drive/editor.sh`, `diff.sh`, `transcript.sh` check them. Phase 4: real shells run in the cards (typing, htop with mouse mode, fastfetch, clear, underlined links, drag selection with Cmd+C, cursor blink, the inactive scrim, spawn errors, OSC 52, `dev.stress.*`). 26 flooding cards paint at 110 fps and a zoom over 26 idle cards at 55 to 105 (reference 45 to 60 and 60); idle is two frames a second. Open from Phase 4: the Turkish-Q chord check. The browser crate is still empty. `docs/phase-1-progress.md` has the pure-module table; HANDOVER.md has the phase state. The spikes remain untouched until their full replacements exist.

Things decided while drawing terminals, not in the reference:

- Frames are painted on demand, not every tick. `needs_frame()` (animation, gesture, pending output, dirty bodies, notice) gates `request_animation_frame`; a 16 ms poll task drains the backend channels. The status bar fps is a stale number while idle.
- The cell width is the advance of `M` at the font size; `letterSpacing` is ignored (a negative one made the cell narrower than the glyph and rows overlapped). Wide characters put `SPACER` in their second cell.
- Rows are shaped in 24-cell chunks positioned at the cell's x and cached per row by hash; one shaped line per row drifted on long rows. Glyphs cost about 1 µs each in gpui and are skipped under a 3 px font.
- Free drag and resize stay, with alignment guides (`alignment.rs`, exact matches only) and a no-overlap rule on release (`Model::end_gesture`); the moving card paints last with a red outline while it overlaps. The reference app mirrored both on 2026-09-15.
- The shortcuts panel is a centred filterable overlay, not a sidebar.
- A text selection lives in the grid (alacritty's `Selection`, grid points), so it stays on its text through a scroll; selected cells take the app's selection pair in `Grid::frame`, never the theme's. A body that starts a drag says so (`captures_drag`) and gets the moves and the release wherever the pointer goes (`AppView::body_drag`).
- The cursor blinks from a clock the body gets at paint (`now`); `wants_frame(now)` reports the next flip so the poll task requests a frame then and not every tick.
- The editor's logic is a crate without gpui so it tests headless; the body draws visual rows (a prose line wraps to several) and the hit test reads the same row table. Highlighting is a whole-text tree-sitter pass per buffer version, milliseconds for a few thousand lines; a parse longer than a frame would move off the ui thread, not get cleverer.
- Editor, diff and transcript bodies report back through `EditorEvent` (a notice, a path change) and `editors.rs` copies dirty/language/read-only onto the card each frame; the model never reaches into a body.
- A body keeps its `Frame` and `Grid::update_frame` rebuilds only the rows alacritty damaged (a selection, scroll, resize or palette change is a full rebuild); links are rescanned for those rows only. Rebuilding every row of 26 cards was 17 ms a frame and trimming them for links another 10. The dev profile optimises dependencies so the driver's numbers are gpui's real ones.

## Commands

```
make                    # lists the targets
make run                # build, bundle, launch on a scratch copy of the real canvas (never the real data dir)
make check              # fmt (the port crates only; cli and hook stay copies of the reference, except the INFINITERM_DATA_DIR socket override) + clippy + test
tools/drive/ift.sh      # headless: drives a scratch instance with ift and the hook binary over the socket; runs on a locked Mac
make drive              # scripted GUI run with screenshots (say so before running it); drive-drag, drive-panel; tools/drive/<scenario>.sh for the rest
make run DATA=/tmp/x    # any data dir; the default is /tmp/infiniterm-dev
INFINITERM_CONFIG_DIR   # moves ~/.config/infiniterm the same way; tools/drive/settings.sh edits a copy
cd spikes/<name> && cargo build --release             # a spike; each is its own workspace
cargo run --manifest-path ~/Code/cef-rs/Cargo.toml -p cef --bin bundle-cef-app -- <bin> -o target/bundle
open --stderr "$PWD/run.log" --stdout "$PWD/run.log" target/bundle/<bin>.app
tools/shot.sh <app-name> out.png
```

`CEF_PATH` is `~/.local/share/cef` (each spike's `.cargo/config.toml`). Only one CEF process per profile: `pkill -9` the old one and remove `spikes/cef-extension/profile/Singleton*` first.

## Rules

- Every file starts with a header block: responsibility, where it fits, what calls it, related files, constraints. Comments say why.
- Logic is a pure function in its own file with tests; gpui elements are wiring.
- Tests always, unless Ekin says "skip the tests".
- Stage by file name; commit per phase at least; messages say why; no attribution trailers of any kind.
- Verify on screen before calling something done, with `tools/drive/`. Announce before sending any input to the Mac and wait for a go: Ekin may be on it. The driver refuses a locked Mac and a Mac where another app stays frontmost; `tools/drive/ift.sh` is the check that needs no screen. Keys through System Events, never cliclick (see HANDOVER.md). gpui does not draw while the Mac is locked.
- Never run the native app without `INFINITERM_DATA_DIR` while the Tauri app is the shipping one: it would save over the real canvas (it did once).
- A spike stays as it is until its replacement is in a crate, then the spike is deleted in the same commit.
- Style for anything a human reads is in `~/.claude/CLAUDE.md`: plain, specific, no em-dashes, say less.

## Traps already hit

Short forms; the NOTES files have the accounts.

- Both `cef` and `gpui` glob-export `App`, `Window`, `Point`, `MouseEvent`: import gpui by name.
- CEF aborts on `isHandlingSendEvent` unless `cef_app_protocol::install` runs first (gpui owns the NSApplication subclass).
- Create the browser inside `open_window`'s builder; CEF reads the device scale at creation.
- `alacritty_terminal::event_loop` gives 1 fps under flood. Reader threads move bytes, the UI thread parses a budget per frame.
- A binary started from a shell is not activated and gets no key events. Bundle and `open`.
- CEF reads native-messaging manifests from `<cache-path>/NativeMessagingHosts/` only.
- `--use-alloy-style` is required with `--off-screen-rendering-enabled`, or the extension's tabs crash.
- `navigator.userAgentData` exists only in secure contexts; never probe the moat on a `data:` URL.
- gpui's `Keystroke` has no key code and delivers shifted punctuation as the shifted character with shift cleared; `keymap.rs` un-shifts it. Turkish-Q (Cmd+ğ on the `[` key) is still unchecked on a device.
- `serde_json` parses floats one ULP off without `float_roundtrip`; the save file needs it on.
- A test must never bind `/tmp/infiniterm.sock`: the running Tauri app holds it. `hooks::listen` takes the path; tests use a temp one.
- `cargo fmt` reformats a file under an edit made from a stale copy, and a text patch then silently matches nothing. Three edits were lost this way, one of them the `INFINITERM_DATA_DIR` override, which is how the real canvas got overwritten. Re-read after fmt, and `cargo fmt --all` is never run: it touches the cli and hook crates.
- cliclick keys never arrive as gpui keystrokes (typed characters carry the fn flag and no `key_char`; Return and Escape are swallowed by the input context). System Events `keystroke` / `key code` work. `screencapture -o` drops the window shadow, which offset every click by ~58 px.
- An effect that needs state from before the command (swap animation's old rects) must carry it; reading the model after the command sees the new positions.
- `serde_json::Number` 14 and 14.0 are not equal; compare parsed `Config` values, not raw JSON.
- gpui's `Pixels.0` is private; `AppContext` must be imported for `cx.new`.
- The palette matches "stress flood" to the calm command first; the driver types "run yes".
