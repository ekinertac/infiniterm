# Changelog

What changed, for someone who uses the app. Build numbers are commit counts, the number in the status bar.

How this file is kept: work that has shipped to `master` but not to a release goes under **Unreleased**, one section per day, newest day first, written when it ships. At a release those days fold under the version's heading. Everything before this file existed (2026-09-26) is summarised by release and then by area rather than by day, because the busiest days had forty to sixty commits.

## Unreleased

### 2026-09-27

- "Card: clear the state colour" in the palette greys a card's ring and tab dot once you have seen what it had to say.

### 2026-09-26

- Tidy keeps groups and split cards together: each moves as one block with its inside arrangement, where it used to spread their cards across the canvas.
- Cmd+Ctrl+Enter grows a card into the gap it sits in, from whichever corner that takes, instead of always from its top-left; a half moved under another card grew down into open canvas.
- An update downloaded while an older build ran no longer replaces a newer build installed since; the restart just restarts. It had put 0.2.0 back over a newer install.
- Cmd+T fills the holes in your grid of cards, top row first, before it grows the canvas, wherever the grid sits; it used to count only from the canvas origin and kept growing past holes above and left of it.
- The transcript card tells speakers apart: a message relayed from another agent session is its own turn, labelled with the sender's name, and a background task finishing is one faint notice line. Both used to read as your own turns, tags and all. A message the session sent reads in full as `→ name` inside the agent's turn instead of a tool line cut at 400 characters.

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
