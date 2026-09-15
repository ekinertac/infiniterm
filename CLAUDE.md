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
infiniterm-term/src/        output scheduler and parsed-byte acknowledgement ledger
infiniterm-browser/src/     browser crate placeholder; no CEF dependency yet
infiniterm-ui/src/          UI crate placeholder; no gpui dependency yet
spikes/cef-extension/       cefclient + Claude in Chrome; the shared profile/ and extension/ (gitignored)
spikes/cef-frame/           CEF frame as a gpui texture; chrome_moat.rs
spikes/term-zoom/           25 alacritty grids under a zoom
spikes/canvas/              all of it on one canvas; viewport.rs is core's first module
tools/shot.sh               screenshot one app's window for remote verification
```

Phases 0 to 2 are complete: every pure module (483 of 498 reference cases plus 18 native checks) and the whole backend behind `app::Backend`, 581 tests. `commands/*.ts` ports with the stores in Phase 3, which is next: the gpui canvas with blank cards. The browser and ui crates are still empty. `docs/phase-1-progress.md` has the pure-module table. The spikes remain untouched until their full replacements exist.

## Commands

```
cargo test --offline                                  # the workspace; 581 tests
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
- Verify on screen before calling something done. Ekin is usually remote: launch through `open` on the `.app`, activate with osascript, screenshot with `tools/shot.sh`, drive with `cliclick`. gpui does not draw while the Mac is locked.
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
