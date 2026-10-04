# Changelog

What changed, for someone who uses the app. Build numbers are commit counts, the number in the status bar.

How this file is kept: work that has shipped to `master` but not to a release goes under **Unreleased**, one section per day, newest day first, written when it ships. At a release those days fold under the version's heading. Everything before this file existed (2026-09-26) is summarised by release and then by area rather than by day, because the busiest days had forty to sixty commits.

## Unreleased

### 2026-10-04

- "Window: change the title bar and Dock colour" (Cmd+Shift+P) works in every window, your own as well as an `ift connect` one. It picks a colour in two steps: a list of named colours, previewed in the title bar and the Dock icon as you move through it (Escape puts it back), or "Custom hex colour…", which asks for a code like `3b82f6`. In your own window the choice is the setting `ui.windowColor` (a name or a hex code, empty for none), which you can also edit by hand, and it tints the title bar and badges the Dock icon. In an `ift connect` window it is saved for that host; "Default: from the host name" goes back to the automatic colour, and `ift connect user@host --color RRGGBB` sets it too.
- A card in a remote window now starts in the server's home folder, not a Mac folder that does not exist there, and `ift sessions` no longer lists a session whose shell is gone.
- `ift connect user@host` opens a second infiniterm for a server: its own window, Dock icon and canvas, with terminal cards that run on that host over ssh. The shells live in `iftd` on the server, so closing the window leaves them running and connecting again brings the same cards back. Each host gets its own colour (a tinted title bar, a chip with its name, a badge on the Dock icon; `--color` sets it), its own settings (a copy of yours the first time) and no browser cards. `ift connect user@host --check` only tests the host. It needs key login and `ift` and `iftd` installed on the server: static Linux builds (x86_64 and aarch64) are attached to each release from the next one.

### 2026-10-03

- Four `ift` commands to drive cards from a script. `ift send <card> text --enter` types into a card's shell without moving the focus (`--key ctrl-c` and others for keys). `ift read <card> --lines 20` prints what a card's terminal shows, `--all` for its history too. `ift close <card>` closes a card like Cmd+W. `ift run <command-id>` runs any command from `ift commands`. A card is its number (`7` or `#7`) or its id. `ift ls --agents` lists the cards that have an agent with the session id `claude --resume` takes.

## 0.5.2 (build 560), 2026-10-03

### App

- `ift licence <email> <key>` registers this Mac with its commercial licence key, and the About window then says who it is licensed to. Optional: nothing in the app asks for it.

## 0.5.1 (build 549), 2026-10-03

### App

- An About window: the icon, name, version and build, the author and the website, with a Check for Updates button. Open it from the app menu (About infiniterm) or the palette (App: about infiniterm). Asked for by Tarık Kavaz.
- Opening the settings (or the keybindings) focuses your own file alone. Both cards were selected before, so you had to deselect before typing. They still open side by side and close together.

### Canvas

- `ui.backgroundImage` puts a picture behind the canvas: three come with the app (`"dusk"`, `"aurora"`, `"ember"`) or give a path to your own; `ui.backgroundImageFit` is `cover` (default) or `contain`.
- `ui.windowOpacity` (0.1 to 1) makes the window see-through: the canvas, title bar and status bar fade while cards stay solid. `ui.windowBlur` blurs what is behind it.
- `ui.cardOpacity` (0.1 to 1) makes a card's background see-through so the canvas, the grid and a background image show behind it. Text stays solid. Cells a program colours itself (vim's status line) and browser cards stay opaque.
- `ui.showGrid` turns the canvas grid off (default on).
- Two settings for fitting a card (Cmd+1, Cmd+2, Cmd+3): `ui.fitPadding` sets the margin around it in pixels (default 48, 0 lets it touch the window edges) and `ui.fitMagnify` lets a fit zoom past 100% so a card fills the window.
- `ui.workspaceIsolation` (default off): Ctrl+Tab switches between the current workspace's cards only, and the status bar counts this workspace's cards. The palette still lists every card. Asked for by Tarık Kavaz.
- `ui.maximised` from 0.5.0 is gone: always-maximised was a way of working infiniterm is not built for. `Cmd Shift Enter` still maximises one card, and the fit settings above make a fit fill the window.

## 0.5.0 (build 523), 2026-10-02

### App

- A new app icon: four cards whose borders are the four card states, violet, yellow, red and green. Designed by Tarık Kavaz.

### Canvas

- `"ui.maximised": true` keeps the focused card filling the window, through new cards, closes, workspace switches and focus moves: one card at a time with the canvas behind it. Cmd+2 still shows the whole canvas, and Cmd+Shift+Enter turns it off for the session.
- The card label can sit in any corner (`ui.cardLabelPosition`: "top right", the default, "top left", "bottom left", "bottom right"), and `ui.hideLabelWhenMaximised` leaves it off a maximised card.
- The cursor is a closed hand while you pan the canvas with Cmd+drag or the middle button.

### Editor

- "Editor: open a file" opens the macOS file panel instead of asking for a typed path; a file opens in an editor card, a folder with its tree.

### Shortcuts and themes

- The shortcuts panel (Cmd+/) shows each command's id, the string keybindings.json binds, and finds a command by it. Up and Down highlight a command and Cmd+C copies its id.
- The theme picker marks the theme you have with "active" and a line under it, then lists the rest A to Z, and keeps its order while you preview; it used to re-sort on every move, so the highlighted row and the theme shown drifted apart.

## 0.4.1 (build 487), 2026-10-02

- Releases and updates come from https://github.com/ekinertac/infiniterm/releases, beside the source, from this version on. Copies on 0.4.0 and earlier found this one where they always looked and move over with it.

## 0.4.0 (build 483), 2026-10-02

### Getting started

- A first launch opens a "Start here" card beside the first terminal: the keys to start with, the mouse gestures, what the border colours mean, and which coding agents on your Mac are wired, with the one command for each that is not. "Help: open the welcome card" brings it back.
- The welcome card is a page: the document rendered, with headings, lists, code and links you can click. It never takes the keyboard; the wheel or the arrows scroll it while every other key stays the canvas's.
- The docs are in the app: "Help: open the docs" opens the same pages as infiniterm.app, with the list of pages beside them and `Cmd F` to search. They come with each build, so they match the version you run, offline.
- The shortcuts panel (Cmd+/) lists every mouse gesture, not six of them.

### Editor

- Editor cards have a status bar: the file's path, and the line and column, the selection size, the cursor count, the indentation, the encoding, the line endings and the language. Its text selects like any text, and `Cmd C` copies it.

### Fixes

- `ift -n <file>` opens the file in a card of its own again; it answered "-n takes a path" when given one.

## 0.3.0 (build 456), 2026-10-01

### Agents

- Codex and OpenCode cards show their agent's state, like Claude Code and Pi. Run `ift install-codex-hooks` (then approve the hooks with `/hooks` inside Codex) or `ift install-opencode-hooks` once. Codex sends nothing when a turn fails, so a failed Codex turn is not shown. After a reboot a lost session offers its own agent's resume command (`codex resume <id>`, `opencode --session <id>`).
- A green (done) card turns grey once you have looked at it for a moment or typed into it, and the workspace tab's dots show every done card you have not looked at yet.
- "Card: clear the state colour" in the palette greys a card's ring and tab dot by hand.
- The transcript card tells speakers apart: a message from another agent session is its own turn with the sender's name, a background task finishing is one faint line, and a message the session sent reads in full.

### Closing and reopening cards

- Closing a terminal card no longer kills what runs in it straight away. A build or an agent mid-turn runs to its end; anything quiet for a minute is then ended. A closed card whose agent is waiting for you is kept, the status bar says so, and Alt+T brings it back. Reopening a card (Cmd+Z, Cmd+Shift+T) before it ends gives you the program back mid-work.
- Cmd+Shift+T reopens the last closed card, as in a browser. The placement menu moved to Cmd+Ctrl+T.

### Canvas

- Drag on empty canvas to draw a selection rectangle; Shift+drag adds to the selection. Cmd+click adds a card to the selection or takes it out, and Shift+click works on a card's label and frame too. Drag any selected card by its frame or label to move the whole selection.
- Dragging a card shows the grid's slots for a card of its size and snaps to one when it is close; tidy puts cards back on that grid, groups and split cards kept together, and group frames hug their cards.
- Cmd+3 fits the selected cards when several are selected, else the focused card's group, else the block of cards around it.
- Cmd+T fills the holes in your grid of cards before it grows the canvas, wherever the grid sits.
- Cmd+Ctrl+Enter grows a card into the gap it sits in, from whichever corner that takes.
- A dialog, the palette, the address bar and the shortcuts panel keep every Cmd and Ctrl shortcut to themselves while they are open.

### Terminal

- Cmd+= and Cmd+- change the terminal font size for every terminal card, and the size is saved; the canvas zooms with Cmd+scroll, a pinch or the palette.
- Visual mode (Cmd+Shift+C): a keyboard cursor over the output and the scrollback, with Mac keys and vim keys, to select and copy without the mouse. Escape leaves.
- The zsh command line selects like a Mac text field: Shift with the arrows, Alt+arrows, Home and End selects, typing or pasting replaces the selection, Cmd+C and Cmd+X copy and cut it. Cmd+Backspace and Cmd+Delete delete to the start and end of the line.
- New settings: `terminal.copyOnSelect`, `terminal.scrollMultiplier`, `terminal.padding` and `terminal.env`; `terminal.shell` takes arguments.
- A fresh install opens in the Violite colour scheme.

### Editor

- Multiple cursors: Cmd+click, Ctrl+Shift+Up/Down, Cmd+D, Ctrl+Cmd+G, Cmd+Shift+L; Escape goes back to one.
- Folding by indentation: Cmd+Alt+[ and Cmd+Alt+], with Shift for every top-level block, or a click on a line number.
- A git gutter: green, yellow and red marks beside lines added, changed or removed since the last commit.
- Ctrl+- jumps the caret back to where it was before its last leap, Ctrl+Shift+- forward again.

### `ift`

- `ift usage` lists the commands and mouse gestures you used in the last 30 days and the ones you never did, from a log kept only on your Mac.
- When a hook installer cannot find `infiniterm-hook`, the advice names a build command that works.

### Fixes

- An update downloaded while an older build ran no longer replaces a newer build installed since.
- Leftover sessions from closed cards are reliably ended at launch.
- The status bar's frame counter no longer shows a low number in orange at rest, and is off by default (`ui.showFps`).
- Releases are tagged with plain version numbers (`v0.3.0`).

## 0.2.0 (build 356), 2026-09-26

### Editing

- `ift <file>` inside a terminal card opens the file in an editor laid over that card, ready to type in, and waits until you close it, the way vim does. The prompt comes back when you close it, so `EDITOR=ift` (or `GIT_EDITOR=ift`) works for `git commit`. `ift -n <file>` still opens a card of its own.
- A file that does not exist yet opens empty and the first save creates it; closing without saving leaves nothing on disk. A missing directory is refused before anything opens. Any word that is not one of `ift`'s commands is taken as a file name.
- Closing unsaved work asks Save, Don't Save (Cmd+D) or Cancel, as macOS does. Save closes only once the file is actually written. This replaced a second Cmd+W that discarded work on a double press.
- Saving an untitled buffer starts the field on its directory and `untitled.txt` with only the name selected. An existing file is asked about before it is replaced.
- `file:20` puts line 20 in the middle of the view, with the lines above it showing.

### Canvas

- Cards move to another workspace from the palette: "Card: move to workspace…", then the workspace or a new one. The card takes a free slot there and you stay where you are.
- Dialog buttons can be walked with the arrows and Tab; Enter presses the highlighted one.

### Terminal

- Cmd+F finds in the card you are on, including a terminal's whole scrollback, with every match highlighted and a count. Cmd+G / Cmd+Shift+G step while the bar is open. Cmd+E finds the selected text. Escape leaves the match selected, so Cmd+C copies it.
- Card hints moved from Cmd+F to Cmd+J, and a jump frames the card you land on.

### Fixes

- Cmd+Z no longer closes a card you opened; undo walks back moves, resizes and closes only.
- A hint letter is no longer typed into the card it jumps to.

## 0.1.0 (builds 308 to 344), 2026-09-23 to 2026-09-25

### Releases

- Notarized releases friends can install from a DMG, and an updater that checks every six hours, verifies the signature and installs on the next restart. The status bar shows the build number, and a downloaded update says so until you restart.

### Card states

- Any command colours its card, not only an agent: one that runs 5 seconds or more shows working, then done; any failure shows at once. zsh marks each command (OSC 133) through a small integration loaded without touching your dotfiles; programs can also report progress (OSC 9;4).
- Failed is its own state, in red. Working moved from Claude's clay orange to violet, because red and orange blur at fit-all zoom.
- A Claude card keeps its session's name across a relaunch.

### Canvas

- Cmd+T fills a square block from the top-left, whichever card you are on, and a closed card's slot is filled first. New cards are 16:9 and sized to the window (`cards.shape`, `cards.width`, `cards.height`).
- Card numbers are the lowest free number, so they stay small.
- "Canvas: tidy the cards into a block" puts hand-scattered cards back in order.
- Full screen covers the notch instead of leaving a black strip (`ui.fullscreen`).
- Middle-button drag pans; holding the left button and clicking the right fits everything.
- Card labels grow less as you zoom out.
- Cmd+Ctrl+Enter grows a card the other way when right and down are full.

### Terminal

- Prompt icons draw on any Mac: a Nerd Font is bundled as the fallback.
- Emoji built from several characters (⚠️, 🏃‍♀️) draw as emoji.
- Text stays text at fit-all on a screen full of cards, and on Retina down to smaller sizes; it turns to bars only when there is too much of it to draw quickly.
- Cmd+click opens a domain written without `https://`.

### Editor

- The first batch of Sublime's defaults: line moves and duplicates, brackets that close themselves, selection growing by word, line and bracket, and palette transforms (case, sort, unique, trim).
- An editor you are not in takes no keys; Enter or a click is the way in.

### Fixes

- A card is dimmed once when another app is in front, not twice.

## Before the first release, 2026-09-14 to 2026-09-22

The Rust app replaced the Tauri one over these nine days: spikes on the 14th, the port in phases on the 15th and 16th, then daily use.

### Canvas and cards

- An infinite zoomable canvas of terminal, editor, diff, transcript and browser cards, with workspaces, groups, selections, phantom slots for new cards and a keyboard route to everything.
- Dragging a card moves a ghost that snaps to slots; dropping on a card swaps them; Escape cancels. Alignment guides, and a frame that shows it can be dragged.
- Cmd+Z / Cmd+Shift+Z for moves and resizes, and for closed cards. Closing returns to the card you were on before.
- Cmd+Ctrl+Enter grows a card into the free space; Cmd+Alt+S picks a size; Cmd+Ctrl+W closes a card and leaves its space free.
- Ctrl+Tab switches cards the way Cmd+Tab switches apps, most recent first.
- Every card has a number; the palette finds cards and workspaces by name; double-click fits a card or everything.
- Workspace tabs carry one dot per card that lights when its agent works, waits or finishes while you are away.
- Cmd+Shift+H masks a card with a decoy while somebody reads your screen; Cmd+Shift+L locks a card against closing.
- Snippets: plain files in a folder, pasted into the focused card from the palette (Cmd+Ctrl+S).
- A keycast overlay for recordings, showing each shortcut and the command it ran.

### Terminal and sessions

- Real shells in cards on alacritty's grid, 26 cards flooding at 110 fps on a 120 Hz display.
- Sessions outlive the app: our own daemon, `iftd`, keeps each card's shell running across a quit and its scrollback across a reboot, with `claude --resume` ready as the card's last history line. tmux was tried as the backend for one evening and kept only as an option.
- `ift sessions` and `ift attach 7` reach a card's shell from outside the app, even with the app closed.
- Shift+Enter breaks the line in Claude Code; Turkish characters through Option; the emoji panel, dead keys and input methods; one zsh history per card.
- Selection with Shift+click and edge scrolling, and a copy across a wrapped paragraph reads as one line.
- Physical-key shortcuts, so every keyboard layout gets the keys the panel shows.

### Agents

- Claude Code and Pi colour their cards through hooks: working, waiting on you, done. Every change is logged to `agent.log`.
- A card takes the name its agent gave the session.

### Editor, diff and transcript cards

- An editor with tree-sitter highlighting for seventeen languages, find and replace, undo, a file tree, tabs with a focus lock, drafts that survive a quit, and pictures shown instead of read.
- A diff card with the changed files and blame; a transcript card for an agent's session.

### Browser

- Chromium in a card with the Claude in Chrome extension, Google sign-in, tabs with a focus lock that gives the page Chrome's own shortcuts, an address bar (Cmd+L) that decides between address and search, back and forward, find in page, and a right-click menu.
- More than one extension can be loaded; `ift install-extension` adds one.

### `ift`

- `ift <path>` opens a card; `ift ls`, `ift diff`, `ift name`, `ift group`, `ift omni`, `ift commands`; tables on a terminal and tabs into a pipe; zsh completion.

### App

- Settings as flat dotted keys (`terminal.fontSize`) with a documented defaults file, themes from any `.itermcolors`, an interface size multiplier, a signed and notarized bundle, and Cmd+Ctrl+Shift+R to restart.
