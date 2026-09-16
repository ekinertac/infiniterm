# infiniterm

Terminal cards on an infinite canvas, with the state of every coding agent visible at a glance.

It started as a way out of iTerm2: ten to fifteen tabs, two or three splits in each, some panes made full screen, and at that point the trail is lost and finding one particular session among twenty to fifty means opening them one by one. Every session on one infinite canvas, where each keeps a place, is what fixes that. Running coding agents made it worse: you cannot tell which are still working and which have finished without cycling through them, so infiniterm also colours each card's border from its agent's state.

Reading that state is the point. Being interrupted by it is not: nothing flashes, nothing steals focus, nothing sends a notification. You switch agents when you are ready, not when one finishes.

Native macOS app in Rust: gpui draws the canvas, `alacritty_terminal` parses the shells, CEF hosts the browser cards. Single binary plus the Chromium framework, no Electron, no account, no telemetry. Early, and in active development.

## What it does

- Real shells in cards on a pan-and-zoom canvas, snapped to a 25 px grid. A new card is 69 x 80 cells, portrait, the same on every display. A new card opens in the free slot nearest the one you were in; rows form from the window's shape, so the canvas stays screen-shaped at any count
- Card borders driven by Claude Code and Pi hooks: sweeping while a turn runs, solid when it is waiting on you, nothing for a plain shell
- Keyboard-first. Every action is a command; `Cmd` is the app's modifier, and everything else goes to the terminal untouched
- Card labels and group names take a colour of their own from the terminal theme's palette, derived from the card's id so it never changes
- Several workspaces, each a canvas of its own, with a dot on the tab when something there is waiting on you
- Groups: a named frame around a set of cards, whose border reports the state of everything inside it. At 5% zoom you cannot read twenty-five card borders, but you can read five frame borders
- Card labels that track what the card is doing: the running process while one runs, the current directory otherwise; editors carry badges for language, read-only and unsaved. Zoomed out, the name is drawn large across the card and the text becomes texture, so a full card reads as full at 10%
- Splits, iTerm2's keys: a card gives up half of itself to a new one, and the halves remember each other, so closing one hands its space back
- Free drag and resize from the card's edges, with alignment guides against the other cards, and a card dropped on another goes back where it started
- Multi-select with `Cmd + Shift + Arrow` or Shift + click, and every command that can sensibly mean several cards acts on all of them
- No mouse needed to reach anything: arrow into an empty slot to get a hollow card you can fill, `Cmd + Shift + T` letters every empty slot, `Cmd + F` letters every card
- Terminals select with the mouse (two clicks a word, three a line), `Cmd + C` copies, links and paths that exist underline when you hold `Cmd` over them and `Cmd + click` opens them beside the card. `htop`, `vim` and friends get the mouse
- Editor cards: a file opens in an editor that is a card like any other, coloured by the terminal theme. Syntax highlighting for fifteen languages through tree-sitter, find and replace, go to line, comment toggle, undo, save; long lines wrap in prose files. No LSP, no completion, on purpose. From `ift <file>` (`file:42` opens at a line), `Cmd + N` for an empty one, the palette, the new-card menu, or `Cmd + click` on a path. A file tree beside the buffer, from `ift <dir>` or `Cmd + K`, keyboard-driven. Unsaved changes survive a quit, the way Sublime keeps them; closing the card discards them. A file changed on disk under a clean buffer is reloaded
- Diff cards: `ift diff` (or `ift diff <path>`) opens the changes against git HEAD as a card: the changed files with their `+12 −5` on the left, one file's diff on the right, read-only, in the theme's red and green, unchanged stretches folded. `Cmd + B` adds a blame gutter: hash, author, age per line
- Browser cards: Chromium inside the card, with the Claude in Chrome extension loaded, so Claude Code can drive a browser that lives on your canvas and sign-ins work. `Browser: open a URL` from the palette, or `Cmd + click` a URL in any terminal. The page zooms with the canvas and asks for more pixels above 100%. `Cmd + Esc` leaves it, `Cmd + =` / `Cmd + -` zoom the page. Popups open as cards beside it; `Cmd + Shift + click` sends a link to the system browser instead
- Transcript cards: `Cmd + I` on a card running Claude Code or Pi opens its session beside it as turns: you / claude on the left with a first line and a time, the chosen turn on the right with every tool call folded under it. Follows the session while it runs. Thinking blocks and subagent chatter are left out
- A red badge on any card sitting in an SSH session, with the destination
- Terminal themes from `.itermcolors` files, applied to open terminals live
- Configuration in `~/.config/infiniterm/`, Sublime style: your overrides beside a commented defaults file, watched and applied without a restart
- The canvas survives a restart: cards, their positions and sizes, groups, workspaces, the viewport. Each card comes back as a fresh shell in its saved directory. The window reopens where and how you left it

Twenty-six cards all running `yes` at once paint at 110 fps on a 120 Hz display; a zoom over twenty-six idle cards runs at 55 to 105. Three things do that: the PTY reader stops at 256 KiB unacknowledged per pane, so a fast program waits at the kernel's buffer the way it always has on a slow terminal; output is parsed one frame's budget at a time on the UI thread; and only the rows the terminal changed are rebuilt and reshaped. An idle canvas paints twice a second, for the cursor.

Not done yet: scrollback and running processes across a restart (that needs a tmux backend; this restores your layout, not your work), and adapters for Codex and OpenCode.

## Running it

Needs Rust, the CEF binary distribution under `~/.local/share/cef` and a checkout of [cef-rs](https://github.com/tauri-apps/cef-rs) at `~/Code/cef-rs` for its bundler.

```sh
make run          # build, bundle, launch on a scratch copy of your canvas
make check        # fmt, clippy, tests
make release      # optimised bundle in target/bundle/infiniterm.app
```

The app runs only from the bundle: the Chromium framework is loaded from beside the executable, and a bare binary gets no key events from macOS.

To install it:

```sh
make release
ditto target/bundle/infiniterm.app /Applications/infiniterm.app
/Applications/infiniterm.app/Contents/MacOS/ift install      # ift on your PATH
ift install-claude-hooks                                     # agent state from Claude Code
ift install-pi-hooks [DIR]                                   # same for Pi; DIR for a wrapper with its own agent dir
```

After that, `ift` alone launches the app or focuses it, `ift ~/Code/x` opens an editor card with a file tree rooted there, `ift some/file.ts` an editor on the file, `ift diff` the changes against HEAD, and the hooks point at the copy inside the bundle.

## Keys

| | |
| --- | --- |
| `Cmd + T` / `Cmd + W` | new card / close card (a dirty editor asks for a second press) |
| `Cmd + S` / `Cmd + N` | save the editor card's file / new empty editor |
| `Cmd + F`, `Cmd + G`, `Cmd + Alt + F`, `Cmd + /` | in an editor: find, next, replace, comment (elsewhere these are hints, group, and the shortcut list) |
| `Cmd + B` | in a diff: git blame in the gutter |
| `Cmd + K` | clear the terminal; in an editor or diff, show or hide the file list |
| `Cmd + I` | the agent's transcript, as a card beside this one |
| `Cmd + Esc` | leave a browser card's page |
| `Cmd + Alt + T` | new card outside any group |
| `Cmd + Shift + T` | new card, but where: every empty slot around the cards gets a letter, press one |
| `Cmd + D` / `Cmd + Shift + D` | split the card: new card to the right / below |
| `Cmd + Alt + Arrow` or `Cmd + Alt + JKL` | switch cards; into an empty slot shows a hollow card, `Enter` asks what goes in it |
| `Cmd + F` | outside an editor: a letter on every card; press one to go there |
| `Cmd + Shift + Arrow` or `Shift + click` | extend the selection, like Shift + Arrow in a text field; close, split, new card, clear, fit and group then act on all of it |
| `Cmd + Alt + Shift + Arrow` | swap the active card with its neighbour |
| `Cmd + Shift + [` / `Cmd + Shift + ]` | previous / next workspace |
| `Alt + Left/Right`, `Cmd + Left/Right` | word and line movement in the shell, as in iTerm2 |
| `Ctrl + 1` … `Ctrl + 9` | go straight to that workspace |
| `Cmd + Shift + Enter` | maximise / restore |
| `Cmd + 0` / `Cmd + 1` / `Cmd + 2` | actual size / fit card / fit all |
| `Cmd + =` / `Cmd + -` | zoom, centred on the active card (in a browser card: the page) |
| `Cmd + Shift + =` / `Cmd + Shift + -` | interface bigger / smaller |
| `Cmd + Shift + 0` | interface at 100% |
| `Cmd + Shift + R` | rename the active card |
| `Cmd + G` / `Cmd + Shift + G` | group the active card / dissolve its group |
| `Cmd + [` / `Cmd + ]` | go to the previous / next group, with the loose cards as one stop |
| `Cmd + Alt + [` / `Cmd + Alt + ]` | move the active card between groups |
| `Cmd + 3` | fit the active group to screen |
| `Cmd + Alt + R` | rename the active group |
| `Cmd + ,` / `Cmd + Shift + ,` | settings / keybindings, as two editor cards: the defaults read-only beside your file |
| `Cmd + Shift + P` | command palette |
| `Cmd + /` | outside an editor: show every shortcut, searchable |

`Cmd + drag` or middle-drag pans, `Cmd + scroll` zooms; bare scroll and bare drag belong to the card under the cursor. `Cmd + H`, `Cmd + M` and `Cmd + Q` are the menu's and cannot be rebound.

Interface scaling is separate from zoom on purpose. Zoom changes how much canvas you see; scaling changes how big the app's own furniture is on the display you are on. It reaches everything that is the app's own: card labels, group name tabs, the title bar and its workspace tabs, the status bar, the palette, the dialogs and the shortcuts panel, and deliberately not terminal text, card borders or focus rings.

Chords follow the physical key, not the character your layout prints on it, so `Cmd + =` is the key right of `-` on every keyboard, and the shortcuts panel spells them the US way.

`Cmd + Shift + P` runs any command by name, with fuzzy matching: `ntc` finds "New terminal card". The keybinding is searchable too, so typing `Cmd + T` finds what it is bound to. The five most recently run sit in their own section at the top, in the order you used them, so their positions stay put. Everything below is ranked by match, nudged by how often you have run it.

A card closes when its shell exits, so `exit` or Ctrl+D closes it the same way `Cmd + W` does. Closing the last one leaves an empty canvas rather than opening a replacement.

## Configuration

`~/.config/infiniterm/`, four files in two pairs:

```
settings.json              yours, only what you changed
settings.default.json      every setting with its default, commented
keybindings.json           yours, only what you rebound
keybindings.default.json   every binding, commented
```

The `.default` files are rewritten on every launch, which is what makes them read-only in practice. Read them to learn what exists, copy a line across to change it. That split is also why an upgrade never touches your files: new settings appear in the defaults, not in yours.

The `terminal` block covers the shell, cursor and font; `cards` sets how big a new card is and whether it inherits the active card's directory; `canvas` covers zoom sensitivity and pan momentum; `editor` the selection colours, line wash and wrapping; `browser` the page zoom; `ui` chrome sizes, dimming, animation, the centred zoomed-out name and the frame counter. `settings.default.json` is the documentation.

Comments and trailing commas work in both. `Cmd + ,` opens settings and `Cmd + Shift + ,` keybindings, each as a pair of editor cards on the canvas. Both apply on save without a restart.

A binding set to `null` is removed, which is how you give a key back to the terminal:

```json
{ "cmd+k": null }
```

Every chord must include `cmd`, with one exception: `ctrl` plus a digit. A focused terminal consumes ctrl, alt and bare keys and has to keep consuming them or TUI applications break, but `ctrl`+letter always means something to a shell while `ctrl`+digit mostly does not, which is why workspaces are numbered there. On an xterm-compatible terminal `Ctrl+3` sends ESC; if you live in vim, rebind it.

## Themes

All 500-odd schemes from iTerm2-Color-Schemes ship with the app (Catppuccin Mocha is the default). For your own, drop `.itermcolors` files into:

```
~/Library/Application Support/dev.ekinertac.infiniterm/themes/
```

"Switch theme" in the palette opens the picker. Each scheme is applied as you move through the list, because 500 of them cannot be chosen by name. Escape puts back the one you started on; Enter keeps the new one and writes it to `theme` in your settings.

A theme drives the app's own chrome as well as the terminal, the editor's syntax colours and the colour each card and group label takes.

## Persistence

The canvas is saved to `~/Library/Application Support/dev.ekinertac.infiniterm/workspace.json`, half a second after any change and again on quit. Application Support rather than `~/.config` because the app writes it constantly and you should never have to edit it; deleting it resets the canvas without touching your settings. The window's own frame is `window.json` beside it, and the browser's profile and the Claude in Chrome extension live under `browser/` there.

Agent state is deliberately not saved. A restored card starts with no agent and no shell until it spawns one, so nothing can come back claiming to be working days after the agent died. A file from a newer build is never written back; the app runs on it read-only.

One instance at a time: the unix socket `ift` and the hooks talk to is the lock, and a second launch activates the first.

## What it touches

A terminal can reach everything on the machine, so here is everything this one does:

- Your shells, one PTY per card, started as your login shell in the card's directory with `INFINITERM_CARD_ID` in the environment. Nothing is typed into them that you did not type.
- `~/.config/infiniterm/` for settings and keybindings; `~/Library/Application Support/dev.ekinertac.infiniterm/` for the canvas, drafts, themes, the window frame and the browser profile. Nothing else is written.
- A unix socket in the system temp directory, which `ift` and the hook binaries connect to.
- `ps`, `lsof` and `git` run as subprocesses: to label cards with their process and directory, to spot an SSH session, and for the diff and blame cards. `open` hands URLs and files to the system.
- The network only from browser cards, which are Chromium loading the page you asked for, and the Claude in Chrome extension inside them talking to Claude Code the way it does in Chrome. There is no telemetry, no account, no update check, and the app makes no request of its own.

The source is public so all of that can be checked, and a release is a tagged commit built with the CEF version named in `Cargo.lock`, signed and notarized.

## The icon

Two cards on the canvas grid, the front one wearing the border colour an agent working in it would paint. `assets/AppIcon.icns` and `assets/icon.png`; the source SVG is in the archived Tauri repo under `docs/icons/`.

## History

infiniterm was a Tauri and Svelte app until September 2026; that repo is archived as [infiniterm-tauri](https://github.com/ekinertac/infiniterm-tauri) and its notes carry the rationale for most of the rules above. The rewrite is native because a browser card there could be either an iframe that cannot log in anywhere or a native view nothing could paint over or scale, and because twenty-five terminals under a zoom deserve a renderer that draws them. `HANDOVER.md` is the rewrite's log, phase by phase, with the measurements.

## Prior art

`0-AI-UG/cate` and `blueberrycongee/termcanvas` cover adjacent ground. Both are Electron, both are mouse-first, and neither treats agent state as part of a card.

## License

Source-available, not open source. You can read the source, build it, change it and run it on your own machines for free for personal use. Using it for work, including freelance work, needs a paid license, one per person. You cannot hand a compiled build to anyone else: no casks, no release attachments on a fork, no app stores. Full terms in `LICENSE.txt`.
