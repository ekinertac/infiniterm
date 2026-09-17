# infiniterm: working notes for agents

Terminal, editor, diff, transcript and browser cards on an infinite zoomable canvas, driven by Claude Code hooks and `ift`. Rust: gpui for the window, `alacritty_terminal` for the grid and VT parser, tree-sitter for the editor, CEF for the browser card. macOS only. `README.md` is the product description and the key table; this file is what an agent needs to work in the tree.

The installed `/Applications/infiniterm.app` is this repo's release bundle and runs on Ekin's real data dir. Never launch a second copy without `INFINITERM_DATA_DIR`: the socket lock makes it exit, and a scratch data dir is the only safe way to test.

## Where things are

```
infiniterm-core/src/        every pure module (layout, keymap, palette, config, themes, links, transcript ...), omni/ (the address bar: address parsing, tab-to-search engines, providers, ranking, the history store, the suggest parser), the model (model/: every store and command, tested with a Harness), and the backend (PTY, unix socket, inspect, git, files) behind app::Backend. No gpui.
infiniterm-term/src/        alacritty grid + frames (grid.rs), palette, key/mouse encoding, output scheduler, ack ledger. No gpui.
infiniterm-editor/src/      the editor's logic: buffer (ropey + undo), search, language (15 tree-sitter grammars), highlight, explorer, wrap, diff. No gpui.
infiniterm-browser/src/     CEF: process (framework, helpers, profile, extension, pump), surface (one browser: frames, input, popups), moat (Google sign-in), app_protocol. No gpui.
infiniterm-ui/src/          the gpui app: main.rs (AppView, CEF startup, the menu), runtime.rs (effects, backend drain, startup), paint.rs (one frame), input.rs, keycode.rs (physical key codes from NSEvent), overlays.rs (palette, prompt, panel), body.rs (CardBody trait), omnibox.rs (the address bar overlay), terminal_body.rs + terminals.rs, editor_body.rs + diff_body.rs + transcript_body.rs + editors.rs, browser_body.rs + browsers.rs, bin/helper.rs (the CEF helper), window_state.rs, field.rs, animator.rs, chrome.rs, text.rs
infiniterm-cli/, infiniterm-hook/   ift and the hook binary
assets/                     the app icon
tools/bundle.sh             target/bundle/infiniterm.app: cef-rs's bundle-cef-app plus themes, icon, ift and the hook as sidecars
tools/drive/                the GUI driver: lib.sh, then one scenario per area (terminal, select, editor, diff, transcript, browser, stress, idle, settings, ift ...). cast.sh is the odd one out: not a test but the screencast take, paced for a camera, on its own demo repo and canvas under /tmp/infiniterm-cast (`make cast-seed` once, then `make cast`)
tools/locked.swift, winid.swift   the driver's probes: is the Mac locked, which window belongs to which pid
HANDOVER.md                 the rewrite's log, phase by phase, with the measurements and decisions
docs/                       the file-by-file map from the Tauri app, kept as history
spikes/*/NOTES.md           the four spikes' measurements; their code is gone (git log has it). cef-extension/profile (gitignored) seeds the browser profile once
```

The Tauri app this replaced is archived at `~/Code/infiniterm-tauri` (`ekinertac/infiniterm-tauri`). Its `CLAUDE.md` holds the long-form rationale for the rules below; read it when a rule seems arbitrary. Nothing lands there.

## Rules that shape the code

- `core` knows rects, ids and kinds, never what a card body draws. A body is a `CardBody` in the ui crate; the model reaches it only through `Effect`s and reads back through `EditorEvent`s.
- Logic is a pure function in its own file with tests; gpui elements are wiring. Every command is registered in `model/`, so the palette, the keymap and `ift` cannot drift. Nothing is mouse-only.
- `Cmd` owns every app binding except `ctrl`+digit; a focused terminal gets everything else. Chords are built from the PHYSICAL key: `keycode.rs` keeps an NSEvent monitor that records every key-down's key code and real Shift flag, and `keymap.rs` names the chord from those, so Turkish Q and every other layout get the keys the panel shows. Alt+Arrow and Cmd+Arrow belong to the shell. Inside an editor the find, replace, comment and select-to-boundary chords are the editor's (`editor_keys.rs`); inside a browser the zoom chords are the page's (`browser_keys.rs`).
- Positions are permanent; nothing re-tiles. A new card takes the free slot NEAREST the card it opens from (`layout::nearest_free_slot`); ties go to the slot that keeps the workspace's bounding box closest to the window's aspect, then right, below, left, above. No fixed column count. Groups move to a free block right of everything.
- Anything that reads card geometry is scoped to one workspace. Card placement treats other groups' frames as occupied. A card may not be dropped over another (`Model::end_gesture` puts it back); a drag shows alignment guides on exact matches only. One card is a focus and wears the white ring; in a SELECTION the active card stays white so you can see where you are and the rest wear blue, all at one strength, because a command acts on all of them and a dimmed ring made it unreadable which.
- Every viewport change is animated; the system's reduced-motion switch outranks `ui.animations`. UI affordances are sized in screen pixels then divided by the zoom; the interface multiplier (`ui_scale`, Cmd+Shift+= / -) reaches every piece of chrome (card labels, the title bar and its tabs, the status bar, the palette, the dialogs, the shortcuts panel; `titlebar_h()` / `statusbar_h()` in main.rs) and never terminal text, borders or focus rings. The traffic lights stay where macOS put them.
- One tagged event stream for all panes; the reader thread moves bytes, the UI thread parses one budget per frame (the scheduler in `infiniterm-term`). Backpressure is 256 KiB unacknowledged per pane.
- Three agent states, because `Notification` and `Stop` answer different questions: working (mid-turn), waiting (blocked on you: a permission prompt, a question, a failed turn) and done (the turn finished). They get three HUES, not three brightnesses, because two oranges are one colour at 40% zoom. A group frame carries NO agent state: it holds several sessions and one colour cannot say which of them wants you, so colouring it drew the eye to the box instead of the card that was asking. The workspace dot counts waiting only: counting finished cards lit it after every turn, which is the same as having no dot. Nothing else in the chrome may borrow an agent colour; `warn` exists for that. A card's title is a name somebody chose; the label derives name, else process, else directory.
- Settings: defaults and overrides are separate files, the defaults rewritten at launch; a new setting needs a `settings_doc.rs` entry or the test fails. Writes to the user file patch the text, never reserialise.
- The save file is never written before it is loaded, and never written back by an older build (read-only mode). Under the local backend a restored card gets a fresh shell in its saved directory; under tmux it adopts the window it had.
- tmux is the DEFAULT backend (`terminal.backend`, falling back to local PTYs when tmux is missing). One `tmux -C` control client on the default socket, one session, and a card is a WINDOW with one pane, never a pane inside a shared window: `refresh-client -C '@0:100x30'` gives each window its own size, panes tile and would have to share one. tmux draws nothing; it reports bytes and our emulator renders them, which is why scrollback, selection and the mouse are unchanged. Quitting DETACHES. The ack ledger drives `refresh-client -A '%0:pause'` at the same 256 KiB mark the local backend stalls at, or one flooding card would delay every other through the single socket. See spikes/tmux/NOTES.md for what was measured.

## Decisions taken in this codebase

- Frames are painted on demand: `needs_frame()` (animation, gesture, pending output, dirty bodies, an expiring notice) gates `request_animation_frame`; a 16 ms poll task drains the backend channels, runs the save debounces and the editors' disk polls. The status bar fps is stale while idle. `frame_reason()` says which condition asked; `[paint]` in `run.log` under `INFINITERM_KEYLOG` says where a frame went.
- The terminal cell is the advance of `M` at the configured weight; `letterSpacing` is ignored. Wide characters put `SPACER` in their second cell. Rows are shaped in 24-cell chunks of ASCII at the cell's x; any other character is shaped alone at its cell. A body keeps its `Frame` and `Grid::update_frame` rebuilds only the rows alacritty damaged; links are rescanned for those rows only. Below a 3 px font, text is drawn as bars.
- A text selection is alacritty's, in grid points, painted in the app's selection pair. A body that starts a drag says so (`captures_drag`) and gets the moves and the release wherever the pointer goes.
- The cursor blinks from the frame clock; `wants_frame(now)` reports the next flip.
- The editor is ours: a rope with one cursor, undo grouped by CodeMirror's 500 ms rule, whole-text tree-sitter spans per buffer version, visual rows for wrapped prose (up/down still move by logical line), drafts half a second after a change, a two-second disk poll. JSON files use the JavaScript grammar because the config files are JSONC.
- The diff card's rows come from `similar` with the merge view's collapse rule (margin 3, minSize 4). Bodies paint under a content mask.
- The browser lays out at the card's world size and is painted as a texture; the device scale follows the zoom in half steps above 100%. CEF starts before gpui, pumps every 4 ms, and reads its profile and the extension from `<data>/browser/`, seeded from Chrome's install and, once, from the spike's signed-in profile. Popups are refused and opened as cards. The first click on an unfocused browser card only focuses it.
- The window frame is `window.json` beside the save file. The socket is the single-instance lock. The app menu is built by hand (no File, no Edit), so Cmd+H, Cmd+Alt+H, Cmd+M and Cmd+Q are unbindable.
- The omnibox (Cmd+L, `omni/`) is not the palette: the palette fuzzy-matches a fixed list of commands, this takes free text and decides whether it is an address or a search. Ranking IS the section order, never a score: what you typed, suggestions, history, open cards, with the best history prefix lifted to a "best match" row so Enter goes where the inline completion says. On a browser card it opens holding that card's address and Enter navigates in place; anywhere else it makes a card, and the card it was opened on is never one of its own results. History is ours (`history.json`, frecency, written on the layout's debounce) rather than Chromium's, because reading the CEF profile's SQLite means a dependency and a lock fight with the browser that owns it; every navigation is seen in `browsers.rs`, which is why it is recorded there. Google's suggestions are off unless `browser.suggestions` is on and are fetched by a thread running `curl`, so a name lookup cannot hold a frame and no HTTP crate is in the tree. `ift omni <term>` prints the ranking from the running app.
- A file dropped from the Finder does what the thing under the pointer IS (`drop.rs`): a terminal gets the shell-escaped path as a paste, because a terminal is a terminal and the drop has to work mid-command; a browser goes to the file; everything else opens a card through `open_plan`, the same decision `ift <path>` reaches by typing. `shell_quote` is the whole security story: a file called `; rm -rf ~` reaches the prompt as text.
- One dialog, three shapes (`prompt.rs`, drawn by `render_prompt`): a text prompt (label, field, the two keys captioned under it), a confirm (question, Cancel and the action's verb as buttons, `confirm(label, action, pending)`), and an alert (message, OK; unused so far). All sit under a dim sheet that cancels on a click outside; buttons click through `prompt_settle`. Enter and Escape are the primary path; the buttons exist so a mouse is not refused. `tools/drive/dialogs.sh` shows them.
- The shortcuts panel is a centred filterable overlay. `ui.midZoomLabel` draws the card's name large below 60% zoom; corner labels are drawn at 1.2x `ui.cardLabelSize`.

## Commands

```
make                    # lists the targets
make run                # build, bundle, launch on a scratch copy of the real canvas at /tmp/infiniterm-dev
make check              # fmt (the app crates only; cli and hook keep the reference's text apart from the socket override) + clippy -D warnings + test
make release            # optimised bundle; ditto it into /Applications/infiniterm.app
make drive              # a scripted GUI run with screenshots; tools/drive/<scenario>.sh for the rest
tools/drive/ift.sh      # headless: drives a scratch instance with ift and the hook binary over the socket
make run DATA=/tmp/x    # any data dir; INFINITERM_CONFIG_DIR moves ~/.config/infiniterm the same way
```

`make bundle` needs `~/Code/cef-rs` and `CEF_PATH=~/.local/share/cef` (`.cargo/config.toml`) and takes about a minute. The dev profile optimises dependencies so the driver's frame rates are real. Tests run without a window, CEF included.

## Working here

- Every file starts with a header block: responsibility, where it fits, what calls it, related files, constraints. Comments say why. Numbers are named constants with the reason beside them.
- Tests always, unless Ekin says "skip the tests". Stage by file name; messages say why; no attribution trailers of any kind.
- Verify on screen before calling something done, with `tools/drive/`. Before sending any input to the Mac, say so and WAIT for a go: Ekin is often on it, and a message from him describing what he sees means he is at the keyboard. The driver refuses a locked Mac and a foreign frontmost app, and addresses its own instance by pid because the installed app has the same name and bundle id. Keys go through System Events, never cliclick. gpui does not draw while the Mac is locked.
- Style for anything a human reads is in `~/.claude/CLAUDE.md`: plain, specific, no em-dashes, say less.

## Traps already hit

- Both `cef` and `gpui` glob-export `App`, `Window`, `Point`, `MouseEvent`: import gpui by name.
- CEF aborts on `isHandlingSendEvent` unless `app_protocol::install` runs before `initialize` (gpui owns the NSApplication subclass). A helper invocation must run `execute_process` and exit before gpui starts.
- `alacritty_terminal::event_loop` gives 1 fps under flood; the scheduler is why it is not used.
- A binary started from a shell is not activated and gets no key events, and has no framework beside it. Bundle and `open`.
- CEF reads native-messaging manifests from `<cache-path>/NativeMessagingHosts/` only. `navigator.userAgentData` exists only in secure contexts; never probe the moat on a `data:` URL.
- gpui's `Keystroke` has no key code and delivers shifted punctuation shifted with the flag cleared: on Turkish Q, Cmd+= arrived as Cmd+Shift+0 and Cmd+Shift+= collided with it. Hence `keycode.rs`: the code and the Shift flag come from the NSEvent, not the keystroke. Checked on Ekin's keyboard.
- macOS delivers a Cmd key first as a key equivalent and, unless that is marked handled, again as a key down; gpui drops the repeat only when both parse identically, which Cmd+= on a non-US layout does not. Cmd+= zoomed twice per press until a handled chord called `stop_propagation`. Any new key path that takes a chord must do the same.
- A chunk of shaped text drifts after any non-ASCII glyph (an icon, an emoji has its fallback font's advance, not the cell's); the prompt's seconds digit vanished under a powerline segment. Only ASCII is chunked; everything else is one glyph at its cell.
- `terminal.fontWeight` and `fontWeightBold` were parsed and never read for two days. A setting that reaches the config struct is not applied until something reads it; `settings_doc.rs` cannot catch that.
- The app inherits the environment of whatever launched it. Launched from a WezTerm shell, every card claimed to be a WezTerm pane. `local_pty.rs` removes the parent terminal's identity variables and sets `TERM_PROGRAM=infiniterm`.
- `serde_json` parses floats one ULP off without `float_roundtrip`; `Number` 14 and 14.0 are not equal, compare parsed `Config` values.
- A test must never bind `/tmp/infiniterm.sock`: the installed app holds it. `hooks::listen` takes the path; tests use a temp one.
- `cargo fmt` reformats a file under an edit made from a stale copy, and a text patch then silently matches nothing. Re-read after fmt; never `cargo fmt --all` (it touches the cli and hook crates).
- cliclick keys never arrive as gpui keystrokes. `screencapture -o` drops the window shadow, which offset every click by ~58 px.
- An effect that needs state from before the command (swap animation's old rects) must carry it.
- A notice or a pending save must not hold the frame loop open: a launch notice ran the app at 120 fps for five seconds.
- The palette matches "stress flood" to the calm command first; the driver types "run yes". Palette queries in scenarios must match a real label ("browser open a url", not "new browser"; "workspace: close", not "close workspace").
- System Events `keystroke "0" using {command down, shift down}` drops Shift to produce the character; use `key_code 29 "command down, shift down"` (and 24 for Equal, 27 for Minus) when a scenario needs a shifted chord. Chords are physical now, so scenarios press key codes, not characters.
- `resized` on a body was never called until the browser needed it; `paint.rs` now calls it when a card's rect changes. The terminal ignores it: `terminals.rs` refits and resizes the PTY together.
- Two apps named infiniterm with the same bundle id: a driver that finds windows or processes by name typed into Ekin's real canvas once.
