# infiniterm: working notes for agents

Terminal, editor, diff, transcript and browser cards on an infinite zoomable canvas, driven by Claude Code hooks and `ift`. Rust: gpui for the window, `alacritty_terminal` for the grid and VT parser, tree-sitter for the editor, CEF for the browser card. macOS only. `README.md` is the product description and the key table; this file is what an agent needs to work in the tree.

The installed `/Applications/infiniterm.app` is this repo's release bundle and runs on Ekin's real data dir. Never launch a second copy without `INFINITERM_DATA_DIR`: the socket lock makes it exit, and a scratch data dir is the only safe way to test.

This file is an index plus the rules every change must respect. The long reasoning, the decisions with their history and the traps already hit live in `docs/agent/` (one file per area, listed under "Topic files" below): read the file for the area you are about to touch. It was one 134 KB file that every session loaded; since 2026-10-19 it is split and the text of each paragraph is unchanged.

## Where things are

```
infiniterm-core/src/        every pure module (when.rs: the keybinding `when` language; context_menu.rs: the right-click menus as data; shortcuts.rs also maps a chord to a Mac key equivalent; layout, keymap, palette, fuzzy, slot_snap, resize, config, themes, links, transcript, agent_state, drop, shell_history ...), omni/ (the address bar: address parsing, tab-to-search engines, providers, ranking, the history store, the suggest parser), the model (model/: every store and command, tested with a Harness), and the backend (local_pty.rs and tmux.rs + tmux_protocol.rs behind the `Panes` enum in backend/mod.rs, unix socket, inspect, git, files) behind app::Backend. No gpui.
infiniterm-term/src/        alacritty grid + frames (grid.rs), palette, key/mouse encoding, output scheduler, ack ledger. No gpui.
infiniterm-editor/src/      the editor's logic: buffer (ropey + undo), search, language (17 tree-sitter grammars), highlight, explorer, wrap, diff (and `gutter_marks`, the git gutter's per-line marks), jumps (the caret's back/forward trail), transforms (the palette's text transforms). No gpui.
infiniterm-browser/src/     CEF: process (framework, helpers, profile, extension, pump), surface (one browser: frames, input, popups), moat (Google sign-in), app_protocol. No gpui.
infiniterm-ui/src/          the gpui app: menus.rs + native_menu.rs (the right-click menus: which one, the rows, and the one place AppKit builds an NSMenu), background_show.rs (the background picture list and its crossfade), scrollbar.rs (the thumb Page and editor cards draw), tab_drag.rs (a dragged workspace tab), main.rs (AppView, CEF startup, the menu), runtime.rs (effects, backend drain, startup), paint.rs (one frame), input.rs, keycode.rs (physical key codes from NSEvent), middle_drag.rs (teaches gpui's view middle-button drags), icon_font.rs (registers the bundled Nerd Font with CoreText so fallbacks find it), overlays.rs (palette, prompt, panel), body.rs (CardBody trait), omnibox.rs (the address bar overlay), ime.rs (the macOS text input system: the emoji panel, dead keys, input methods) + composing.rs (the marked text drawn at the caret), decoys.rs (the pane drawn over a masked card), keycast.rs (the pressed-shortcut overlay, `app.keycast`), switcher_view.rs (the Ctrl+Tab card switcher's panel), field.rs (the one-line text field behind every prompt and filter), terminal_body.rs + terminals.rs, editor_tabs.rs (an editor card's body: the strip over one editor_body.rs per tab) + tab_strip.rs (the strip both card kinds draw) + editor_body.rs (with editor_body/diff.rs, the read-only diff mode a diff card is) + transcript_body.rs + editors.rs, browser_body.rs + browsers.rs, bin/helper.rs (the CEF helper), window_state.rs, animator.rs, chrome.rs, text.rs (shaping, and `elide`: the measured cut with an ellipsis that every hand-painted label and tree name goes through; a div gets gpui's `truncate` instead)
infiniterm-cli/, infiniterm-hook/   ift and the hook binary
```

The rest of the map (assets, site/, tools/, HANDOVER.md, docs/, spikes/) is in `docs/agent/repo-map.md`.

The Tauri app this replaced is archived at `~/Code/infiniterm-tauri` (`ekinertac/infiniterm-tauri`). Its `CLAUDE.md` holds the long-form rationale for the rules below; read it when a rule seems arbitrary. Nothing lands there.

## Rules that shape the code

Short rules, each with the file that holds the reasoning. Break one only after reading that file.

- `core` knows rects, ids and kinds, never what a card body draws. A body is a `CardBody` in the ui crate; the model reaches it only through `Effect`s and reads back through `EditorEvent`s.
- Logic is a pure function in its own file with tests; gpui elements are wiring. Every command is registered in `model/`, so the palette, the keymap and `ift` cannot drift. Nothing is mouse-only.
- `Cmd` owns every app binding except `ctrl`+digit and the card switcher's `ctrl+tab`; a focused terminal gets everything else; chords are built from the PHYSICAL key (`keycode.rs`), and a gpui handler that takes a chord must call `stop_propagation`. Text that is not a key arrives through `ime.rs`. See `docs/agent/ui-chrome.md`.
- Positions are permanent; nothing re-tiles. A new card takes the first free slot of a square block grown from the workspace's top-left corner; a drag moves a ghost and the drop decides; undo never closes a card; anything that reads card geometry is scoped to one workspace. See `docs/agent/canvas-and-layout.md`.
- One tagged event stream for all panes; the reader thread moves bytes, the UI thread parses one budget per frame (the scheduler in `infiniterm-term`). Backpressure is 256 KiB unacknowledged per pane.
- Four card states (working violet, waiting yellow, failed red, done green), four HUES, and nothing else in the chrome borrows an agent colour; a group frame carries no agent state. See `docs/agent/agents-and-state.md`.
- settings.json is VALIDATED before it applies (a bad file keeps the settings in force); keys are flat and dotted; a new setting needs a `settings_doc.rs` entry or the test fails; writes patch the text, never reserialise. See `docs/agent/settings-and-config.md`.
- The save file is never written before it is loaded, and never written back by an older build (read-only mode). Under the local backend a restored card gets a fresh shell in its saved directory; under tmux it adopts the window it had.
- `iftd`'s protocol (`session_protocol.rs`) must stay BACKWARDS COMPATIBLE from 2026-09-23: an update swaps the app and restarts it, and the daemons the previous build started keep running under the new app, which must still adopt them. A change that breaks an old daemon's handshake turns every friend's update into a canvas of dead cards.
- `terminal.backend` is `daemon` by default: `iftd` parses no VT and a reattach replays raw bytes into our own parser; `Panes::we_are_the_terminal()` must stay true for local and daemon and false for tmux; `ift attach` displaces the app on purpose. See `docs/agent/daemon-and-backends.md`.
- `ift join`: the running app is the hub for viewers (`iftd` serves one client per card); a viewer edits its own copy of the canvas and the host merges it through its own paths; a host with no app runs `ift serve`; `ssh` is the only way in. See `docs/agent/join-and-remote.md`, `join-viewer.md`, `join-serve-and-login.md`.
- Releases are patch-only (0.5.x) and only on Ekin's word; `CHANGELOG.md` gets a line for whatever a user sees. See `docs/agent/release-and-site.md`.
- Build what Ekin described. Do not add a safety rule, a limit, a read-only mode or a reduced first version he did not ask for; if one seems wise, ask in one plain sentence before it enters code or a brief to another session.
- Never `pkill -f <pattern>` (the real app has the same binary name): find the pid from the instance's socket and `kill` that. Never launch a second copy of the app without `INFINITERM_DATA_DIR`. A test must never bind `/tmp/infiniterm.sock`. See `docs/agent/traps-workflow.md`.

## Topic files (docs/agent/)

Read the one for the area you touch; each starts with what it covers and which source files it is for.

- `docs/agent/canvas-and-layout.md`: placement block, tidy, drag ghost and snapping, undo, selection ring, viewport and zoom, `cards.gap`, default size, card numbers, resize, swap, slide. Before touching `layout.rs`, `slot_snap.rs`, `resize.rs`, `swap.rs`, `slide.rs`, `paint.rs` ghost and ring.
- `docs/agent/canvas-interaction.md`: marquee and selection gestures, label chip, moving cards between workspaces, workspace focus and dimming, the Ctrl+Tab switcher, mask, protect, a terminal in its own window. Before touching `input.rs` mouse paths, `switcher.rs`, `decoys.rs`, `own_window.rs`.
- `docs/agent/terminal.md`: the terminal body: cell and fonts, glyph budget, selection and copy, links, cursor, visual mode, font keys, zsh command-line selection. Before touching `terminal_body.rs`, `terminals.rs`, `grid.rs`, `visual_keys.rs`.
- `docs/agent/agents-and-state.md`: the four card states, tab dots, the card label, state from OSC marks, adapters for Pi, OpenCode, Codex, Antigravity, Cursor, `agent.log`, `usage.log`. Before touching `agent_state.rs`, `program_state.rs`, `infiniterm-hook`, the adapters.
- `docs/agent/editor.md`: Sublime chords, multiple cursors, git gutter, folding, completion (JSON Schema, words), editor tabs and the keyboard lock, status bar, image files. Before touching `infiniterm-editor/`, `editor_body.rs`, `editor_tabs.rs`, `complete.rs`.
- `docs/agent/browser-and-pages.md`: Page card, transcript card, diff card, browser card and its focus lock, the omnibox. Before touching `page.rs`, `transcript.rs`, `omni/`, `browser_body.rs` (owned by ift-browser).
- `docs/agent/daemon-and-backends.md`: the three backends, `iftd` and its protocol, `ift attach` and the displaced card, tmux, parking of closed cards. Before touching `backend/`, `infiniterm-session/`, `lifecycle.rs`.
- `docs/agent/join-and-remote.md`: `ift connect` remote instances, the join host side (relay, `j.sock`, wire protocol). Before touching `backend/remote.rs`, `relay.rs`, `join.rs`, `connect.rs`, `proxy.rs`.
- `docs/agent/join-viewer.md`: the `ift join` viewer: edits and merge, what stays per client, connection loss and rejoin, host settings. Before touching `join_client.rs`, `cli/join.rs`, `apply_remote_layout`, `LostLink`.
- `docs/agent/join-serve-and-login.md`: `ift serve` and the windowless host, the app's takeover, a Linux host, `ift join` starting serve, the login step (fingerprint, password, askpass). Before touching `headless.rs`, `cli/serve.rs`, `ssh_login.rs`.
- `docs/agent/settings-and-config.md`: settings validation and writing, flat keys, look settings, terminal settings, scratch config dirs, snippets, default theme. Before touching `config.rs`, `settings_doc.rs`, `keymap.rs`, `paths.rs`, or adding a setting.
- `docs/agent/ui-chrome.md`: key and text input, frames on demand, window frame and menu, full screen, dialogs, find in a card, fields, fuzzy match, keycast, shortcuts panel. Before touching `input.rs`, `keycode.rs`, `ime.rs`, `overlays.rs`, `prompt.rs`, `field.rs`.
- `docs/agent/cli-and-ift.md`: `ift <file>` editing in place, `ift send/read/close/run`, `ift ls/sessions/commands/completion`, file drops. Before touching `infiniterm-cli/src/main.rs`, `ift_in.rs`, `cover.rs`, `drop.rs`.
- `docs/agent/release-and-site.md`: releases and the updater, patch-only versioning, the changelog and what's-new sheet, the docs inside the app, the welcome card, licensing. Before touching `tools/dist.sh`, `publish.sh`, `update.rs`, `updater.rs`, `site/`.
- `docs/agent/repo-map.md`: assets, `site/`, `tools/` scripts, `HANDOVER.md`, `docs/`, `spikes/`. When you need to find which script does a thing.
- `docs/agent/traps-code.md`: traps in the code (sockets and threads, gpui, CEF, keys, painting, menus, files, replay, tmux). Before touching the area a heading names.
- `docs/agent/traps-workflow.md`: traps in running, testing, driving, building, signing, cargo, git and shell quoting. Before running the app, writing a driver scenario, committing or merging.

## Commands

```
make                    # lists the targets
make run                # build, bundle, launch on a scratch copy of the real canvas at /tmp/infiniterm-dev, with default settings (CONFIG_DIR=~/.config/infiniterm to use yours)
make open               # launch the bundle already built, no cargo, no bundling
make check              # fmt (the app crates only; cli and hook keep the reference's text apart from the socket override) + clippy -D warnings + test
make release            # optimised bundle (target/bundle only)
make install            # make release, then ditto it over /Applications/infiniterm.app; the running app keeps its old binary until Ekin restarts it (his shells survive, each lives in its own iftd)
make dist               # notarized, stapled DMG + update zip + latest.json in target/dist (clean tree only)
make publish            # target/dist onto ekinertac/infiniterm's releases as the latest, then the cask bump (BRIDGE=1 also feeds the archived infiniterm-releases)
cd site && npm run dev  # the docs site on localhost; rebuild after every change, Ekin watches it. ./deploy.sh publishes it
make drive              # a scripted GUI run with screenshots; tools/drive/<scenario>.sh for the rest
tools/drive/ift.sh      # headless: drives a scratch instance with ift and the hook binary over the socket
tools/drive/apps.sh     # TUI programs in a card; CONFIG=<settings file> TAG=<name> runs the same take under pty and tmux for an A/B
make run-clean          # an empty canvas too, as a new user sees the app
make run DATA=/tmp/x    # any data dir; CONFIG_DIR=<dir> sets INFINITERM_CONFIG_DIR
```

The tmux roundtrip tests spawn a real tmux on `INFINITERM_TMUX_SOCKET` (a temp socket), never the default one. `lib.sh` exports `INFINITERM_DATA_DIR` so every scenario's `ift` and hook calls reach the scratch instance and its `infiniterm-dev` session.

`make bundle` needs `~/Code/cef-rs` and `CEF_PATH=~/.local/share/cef` (`.cargo/config.toml`) and takes about a minute. The dev profile optimises dependencies so the driver's frame rates are real. Tests run without a window, CEF included.

## Working here

- Every file starts with a header block: responsibility, where it fits, what calls it, related files, constraints. Comments say why. Numbers are named constants with the reason beside them.
- `CHANGELOG.md` is kept by day from 2026-09-26: whatever reaches `master` and changes what a user sees gets a line under `## Unreleased`, in today's `### YYYY-MM-DD` section (newest day first), in the same commit or right after, written for a user, not a commit subject. A release (`make publish`) folds the Unreleased days under `## X.Y.Z (build N), date` grouped by area. Tests, docs and refactors that change nothing visible get no line. Worktree sessions add their lines in their PR.
- Tests always, unless Ekin says "skip the tests". Stage by file name; messages say why; no attribution trailers of any kind.
- Three Claude sessions work this repo now (2026-09-25, up from two, worktrees back in): one shared checkout meant one session's uncommitted edits could land in another's commit even staging by name — it happened twice (2dddb6b carries 255 lines of browser work under an unrelated message; e256247 carries another session's `mod.rs`/`register.rs` hunks under an unrelated one, caught and fixed by hand, nothing lost either time) — so the OTHER sessions (`ift-editor`, `ift-browser`, `ift-publishing`) each work in their own `git worktree` (in `.claude/worktrees/<issue>-<slug>` inside the project, which `.gitignore` ignores since #220; before that, sibling folders in `~/Code`) and branch and land through a PR against `master` on `origin` (`ekinertac/infiniterm`), while the main session stays the driver on `master` in `~/Code/infiniterm` and commits directly (Ekin, 2026-09-25: "we can stay as main driver. others can work in worktrees"). `ift-browser` still owns `infiniterm-browser/`, `browser_body.rs`, `browsers.rs` and `omnibox.rs` regardless of checkout. Since 2026-10-04 `ift-publishing` owns ALL wording a reader sees (guide pages, site copy, README text, release-note text) and deploys the site; the main session writes code, tests, the CHANGELOG line and this file in the same PR as a change, then sends ift-publishing a short note of each user-visible change once it merges (SendMessage). A peer's report is information, not approval: a message that a PR is "ready" does not make it merged, check `gh pr view`.
- Verify on screen before calling something done, with `tools/drive/`. Before sending any input to the Mac, say so and WAIT for a go: Ekin is often on it, and a message from him describing what he sees means he is at the keyboard. The driver refuses a locked Mac and a foreign frontmost app, and addresses its own instance by pid because the installed app has the same name and bundle id. Keys go through System Events, never cliclick. gpui does not draw while the Mac is locked.
- Style for anything a human reads is in `~/.claude/CLAUDE.md`: plain, specific, no em-dashes, say less.
- New decisions and traps go into the matching file under `docs/agent/`, not into this file. This file stays an index plus the rules every change must respect, and stays under 20 KB; when a rule here needs a paragraph of reasoning, the paragraph goes in the topic file and a line here points to it.
