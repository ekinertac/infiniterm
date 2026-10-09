# Changelog

What changed, for someone who uses the app. Build numbers are commit counts, the number in the status bar.

How this file is kept: work that has shipped to `master` but not to a release goes under **Unreleased**, one section per day, newest day first, written when it ships. At a release those days fold under the version's heading. Everything before this file existed (2026-09-26) is summarised by release and then by area rather than by day, because the busiest days had forty to sixty commits.

## Unreleased

### 2026-10-09

- New setting `ui.glyphBudget` (default 30000): how many terminal cells may be on screen before every card turns to bars. Cards turn to bars when a frame would paint too much text, not at a fixed zoom, so a wall of cards can be bars at 50% while one card reads at 30%. Raise it to read more cards at a smaller zoom; each cell costs about 1.7 microseconds a frame.

## 0.5.11 (build 801), 2026-10-09

### Canvas

- New: reading mode for terminal cards. Press Cmd+1 on a card that is already fitted: the canvas zooms to 150% (`ui.readZoom`) with the card's bottom edge at the bottom of the window, so you read the prompt and the newest lines at a larger size. Cmd+Up and Cmd+Down pan along the card, only while this mode is on and never past the card's edges. Typing and Enter work as usual and bring the view back to the bottom. Cmd+1 again or Cmd+2 leaves; so does panning or zooming by hand. The status bar says "reading" while it is on. `Canvas: read the focused card` is the command.
- New: `Card: slide left / right / up / down to the next card or slot` (Cmd+Ctrl+Alt+Arrow). After a resize leaves a gap, the card moves along its row or column until it is one gutter from the next card in line, or to the next grid slot when nothing is in the way. It never overlaps a card, and Cmd+Z puts it back.
- Double-clicking a terminal card that is drawn as bars (zoomed out too far to read, or too many cards on screen) fits that card, like Cmd+1, instead of selecting a word nobody can see. A readable card keeps its double-click.

### Interface

- New settings `ui.fontFamily`, `ui.fontSize`, `ui.fontWeight` and `ui.fontWeightBold` (Tarık Kavaz): the font of the interface itself, meaning the workspace tabs, card labels, status bar, palette and dialogs. The default is now the macOS system font instead of the terminal's monospace one. Terminal, editor and document text keep their own settings. A change applies at once.
- New: `Help: open the changelog` in the command palette opens this changelog as a card beside the focused one, newest release first. It is the text of the build you run, so an update brings its own.

### Agents

- Cursor reports its state like the other agents (Tarık Kavaz): `ift install-cursor-hooks` wires infiniterm into `~/.cursor/hooks.json`, and a card running Cursor's agent shows working, failed and done, and a lost session resumes with `cursor agent --resume`. It never turns yellow: Cursor has no hook while an approval dialog is open, so the card stays violet until you allow or deny. It has not been tried on a live session yet.

## 0.5.10 (build 775), 2026-10-08

### Window

- Fixed: dragging a window corner resizes the app on macOS 27 when the browser is enabled (thanks to Tarık Kavaz). CEF was left on the native AppKit message pump, which ended the resize at once; it now uses the external pump the app already asked for.

## 0.5.9 (build 767), 2026-10-08

### Right-click menus

- Right-click opens a native macOS menu, the same system menu other Mac apps show, with each command's shortcut at the right edge and unavailable rows greyed. On a terminal: Copy, Paste, Find, Search the Web for Selection, Split, New, Clear Buffer, Show Transcript, Rename, Protect, Mask, Size, Move to Workspace and Close. On a card's frame or label, on bare canvas, on a workspace tab and on an editor there is a menu of its own (the editor's has the Transform submenu). A terminal program that uses the mouse (vim, tmux, htop) still gets the plain right-click; hold Shift to open the menu over it.
- The menu covers more places. A right-click on a file or folder in an editor's tree lights the row and opens a menu for it: Open, Reveal in Finder, Copy Path and Copy Relative Path. A right-click on a tab of an editor or a browser card opens a tab menu (New Tab, Close Tab, Reopen Closed Tab, and for a browser Reload, Copy Address and Open in System Browser).
- Right-clicking a workspace tab no longer switches to it; only a command you choose from its menu does, and Move Left / Move Right keep you in the workspace you were in.

### Browser

- Middle-click and Cmd+click on a link open it in a new tab. Middle-click on a tab closes it, in browser and editor cards.
- Cmd+T opens the address bar on the new blank tab, so the tab is never a blank page with nowhere to type.
- The mouse's back and forward buttons move through the page's history. Back on a tab a link opened closes it.
- The page's right-click menu is native, shows Back and Forward only when there is somewhere to go, shows them only on the page itself (not on a link, a selection or a field), and ends with "Open Page in System Browser" (on a link, "Open Link in System Browser").

### Commands

- A command that needs a second action from you ends its label with an ellipsis, in the command palette as everywhere on a Mac: Run a command…, Card: rename…, Workspace: rename…, Group: rename…, Editor: open a file…, Editor: go to a line…, Browser: open a URL…, go to a URL…, address bar…, find in page…, Card: find in this card…, Card: new (choose where)…, Browser: new tab…, Theme: switch… and Group: the selection…. A test finds the commands that leave something open and fails when one has no ellipsis.

### Terminal

- Double-clicking a link selects the whole address; it used to stop at the colon of `https://`.

## 0.5.8 (build 743), 2026-10-07

### Keybindings

- Keybindings can carry a `when`, as in VS Code, so a binding applies only in some situations. A chord takes an object (or a list of them) in `keybindings.json`: `{"cmd+shift+e": {"command": "browser.leave", "when": "editorTextFocus"}}`; the last one whose `when` holds wins, and `"command": null` unbinds the chord in that situation. A key without cmd, such as Esc or F2, can now be bound too, but only with a `when` that is false in a focused terminal (for example `editorTextFocus`), so it can never take a terminal's input. A `when` joins keys with `&&` and `||`, negates with `!`, compares with `==` and `!=`, and groups with parentheses. The keys: cardKind, terminalFocus, editorFocus, editorTextFocus, browserFocus, cardLocked, overlay, phantomFocus, multiSelection, suggestWidgetVisible, findWidgetVisible and editorHasSelection (each described in `keybindings.default.json`). A mistake in a `when` is reported and that binding is ignored. The existing shape still works.
- A new `keybindings.json` starts with commented examples to switch on by deleting the `//`: another chord to leave a locked editor, turning off the single-Escape unlock, F2 for go-to-line with a `when`, moving workspace tabs and stepping through themes from the keyboard, Cmd+O to open a file and Cmd+Alt+C to start Claude Code. Each example is tested to parse and to name a real command.

### Editor

- A locked editor unlocks with a single Escape when Escape has nothing else to do. A completion popup, the find panel, extra cursors and a selection each take the Escape first (and the card stays locked), so closing the popup or the find panel with Esc and leaving with the next Esc works as in any editor. A browser card still takes a double Escape. Cmd+Escape (`browser.leave`, "Card: leave the page or the editor") also unlocks an editor now, and it is a keybinding like any other: bind `browser.leave` to another chord in `keybindings.json` and that chord is the way out while the card is locked.

## 0.5.7 (build 731), 2026-10-06

### Editor

- The editor's file tree can be resized with the mouse: drag the line between the tree and the text. The pointer turns into a resize cursor over it and the line gets thicker, and a press on it no longer locks the card. The keyboard commands for the width are still there.

### Canvas and dialogs

- Rows and buttons you can click show it: the pointer becomes a hand and the row under it lights up, in the command palette, the address bar's list, the shortcuts panel, the dialogs' buttons, the browser's context menu and the workspace tabs. (The hand did not show in the first build with the highlight: the cursor was reset to the arrow every frame.)
- An empty input box (the command palette, the address bar, the find bar, the shortcuts filter) shows a text cursor before its placeholder, so it reads as something to type into.
- A click outside the command palette or the shortcuts panel closes it, as Escape does (a previewed theme goes back).
- Fixed: choosing Browser for an empty slot opened the address bar, but pressing Enter in it opened the kind picker again behind it instead of going to the address. A key typed in any open overlay no longer reaches the empty slot behind it.

## 0.5.6 (build 714), 2026-10-06

### Settings

- Setting a value from the palette (the window colour, for one) no longer breaks `settings.json` when its last entry already ends in a comma: the new line added a second comma and the file stopped parsing. A comment on the last line keeps its comma outside the comment too.
- `settings.json` is checked before it is applied. A file with an error (it does not parse, it is not one object, or a setting holds the wrong kind of value, such as text for a number) is not accepted: the settings you had stay, a notice says what is wrong, and the status bar keeps saying "settings.json not applied" until a valid file arrives. Before, one stray comma reset every setting to its default (font, background, window colour). `keybindings.json` keeps the bindings you had the same way.

### Editor

- Typing in the editor no longer opens a completion popup after every key. It opens for a word being typed in a file it knows (settings.json, or a JSON file with a schema), not after a newline, a space or a comma, and Enter makes a newline unless you chose a row with Up or Down (Tab always takes the row).

### Canvas

- Workspace tabs can be reordered: drag a tab onto another to take its place. The palette has "Workspace: move this tab left" and "move this tab right" for the keyboard. The order is saved and is the one Ctrl+digit and "Workspace: go to N" use.
- With several cards selected, Cmd+Alt+Shift+Arrow moves the whole selection one block over, as one piece: into free space, or trading places with the cards that are there. It stops with "no room to move the selection that way" when a card would be split or the block would land in another group's frame, and Cmd+Z undoes it in one step.
- `ui.cardRadius` rounds the corners of cards and group frames, in pixels at 100% zoom (0 to 40; 0, the default, keeps them square). The border, the focus ring, the label chip in the card's corner, the tab strip and the browser page follow it. The window's own corners stay macOS's.
- `cards.gap` sets the space between cards in pixels (0 to 200, 25 by default). New cards, splits, growing a card, Canvas: tidy and group frames all use it. Cards already on the canvas stay where they are until they are moved or tidied.

## 0.5.5 (build 687), 2026-10-06

### Files

- Every file the app writes keeps a symlink: the canvas file (`workspace.json`), the browser history and the licence now write through a link too, and `ift install-claude-hooks`, `install-codex-hooks`, `install-opencode-hooks` and `install-pi-hooks` write to the file a linked `~/.claude/settings.json` (or the Codex, OpenCode and Pi file) points at, instead of replacing the link. Settings, keybindings and the editor's save already did.

## 0.5.4 (build 681), 2026-10-06

### Editor and Page cards

- The editor completes from a JSON Schema. A `.json` or `.jsonc` file with a `"$schema"` key near the top (a path beside the file, or an `https` address, which is downloaded once and kept in the data folder) offers its keys and values: type a quote for keys, or a quote after a colon for the choices, `true`/`false` and the default. In your `settings.json` the schema is built in, so values complete there too: `"ui.fullscreen": "` lists `cover` and `native`. The list opens when you type, not when you move the caret.
- Setting names complete in `settings.json`: type `"ui.` and a list of the `ui.*` settings opens, each with its default value, and the selected one with its description. Up and Down move, Tab or Enter inserts the key, Escape closes. Letters without a dot match anywhere in the name (`fitpad` finds `ui.fitPadding`). Only in your own `settings.json`.
- Page cards (the Start here card included) and editor cards show a scrollbar at their right edge when there is more than fits, so it is clear that they scroll.
- Saving a file that is a symlink keeps the link and writes the file it points at. Before, the first save of `settings.json` linked from a dotfiles repo, or the first setting changed from the palette, replaced the link with a regular file.
- Scrolled text in the editor no longer runs under the line numbers and git marks: the text is cut at the edge of the gutter, and the numbers stay readable.
- A long line in the editor no longer draws over the file tree when the view scrolls right. The text is cut at the edge of the text area.
- In the editor, typing a quote, backtick or opening bracket over selected text wraps it instead of replacing it: select `foo`, type `(`, get `(foo)`. The text stays selected, so a second press wraps again.

### Settings and look

- A change to `settings.json` or `keybindings.json` made in another editor shows at once. Before, an idle window kept drawing the old settings until the mouse moved.
- `ui.backgroundImage` takes a list of pictures as well as one: `["dusk", "~/Pictures/lake.jpg"]`. They rotate in order with a crossfade, set by `ui.backgroundImageInterval` (seconds a picture stays, 300 by default) and `ui.backgroundImageFade` (seconds of crossfade, 2 by default, 0 cuts). Only the picture on screen and the next one are kept in memory.
- The theme picker no longer dims the canvas, so you see each theme's real colours on your cards while you move through the list.
- `ui.fitSplitSlot` (default on): Cmd+1 on a card that was split frames the whole slot, both halves together, as it always did. Set it to false and Cmd+1 fits the card itself and fills the window with it, split or not.
- With a title bar colour set, the workspace tabs follow it: the selected tab's fill leans toward the colour and the other tab names are brighter, so they stay readable on any colour.

### macOS

- A program in a card can use what macOS guards: Photos, Contacts, Calendars, Reminders, the camera, the microphone, speech recognition, location, Bluetooth, other apps, the Desktop, Documents and Downloads folders, Accessibility, Screen Recording, Input Monitoring and the local network. macOS shows its usual dialog the first time a program asks, and the grant is listed under infiniterm in System Settings > Privacy & Security. Nothing is asked at install or launch. Before, macOS refused most of these without asking. Full Disk Access is never asked for: add infiniterm by hand if a script needs it.
- The app icon is sharp in the Cmd+Tab switcher. It was enlarged from a small bitmap, with stair steps on the edge, and the green dot of a window colour made it worse.

### Canvas and dialogs

- A first launch opens only the "Start here" card, with a new opening: what the canvas is, then `Cmd T` for your first terminal and `Cmd Alt Left` to come back. It used to open a terminal beside it, which took the focus before the card was read.
- The find bar (Cmd+F) opens inside the card being searched, at its top right under the card's label, instead of in the window's corner. In a small or half-hidden card it narrows and stays inside the window.
- The About window has no buttons now: Enter, Escape or a click outside closes it. Check for Updates is in the app menu. In every other dialog, a button with no key of its own no longer shows an empty badge after the highlight moves to it.
- Cmd+Ctrl+Enter also grows an empty slot. Move the arrows onto a free slot (it is the size of the card you came from), press Cmd+Ctrl+Enter, and the slot grows into the free space up to a full card, the way a card does. The card you make from it has that size.

### Servers

- A new `ift connect` host starts with the default settings instead of a copy of yours, and keeps its own from then on. Hosts you already connected keep the copy they got; delete `~/.infiniterm/remotes/<host>/config` to reset one. An app started with its own `INFINITERM_DATA_DIR` keeps its settings in `<data>/config` instead of reading `~/.config/infiniterm`.

## 0.5.3 (build 610), 2026-10-04

### Servers

- `ift connect user@host` opens a second infiniterm for a server: its own window, Dock icon and canvas, with terminal cards that run on that host over ssh. The shells live in `iftd` on the server, so closing the window leaves them running and connecting again brings the same cards back. Each host gets its own colour (a tinted title bar, a chip with its name, a badge on the Dock icon; `--color` sets it), its own settings (a copy of yours the first time) and no browser cards. `ift connect user@host --check` only tests the host. It needs key login and `ift` and `iftd` installed on the server: static Linux builds (x86_64 and aarch64) are attached to each release from 0.5.3.
- `ift connect user@host --install` puts `ift` and `iftd` on a Linux server for you. This Mac downloads the package for its own version from the GitHub release, checks its checksum, and sends it through the ssh connection, so the server needs no internet, no `curl` and no Rust. Root gets `/usr/local/bin`, anyone else `~/.local/bin`. A host that is a Mac uses the app already there. Without `--install`, `ift connect` says what is missing and how to fix it; it also tells you when the server's `ift` is a different version from the app. `ift --version` prints the version. The Linux packages are attached to each release from 0.5.3; `--from <package>` installs from a file.
- Agent hooks and `ift` now work inside a remote window's cards. A Claude or Codex session running on the server shows working, waiting and done on its card, and `ift ls`, `ift send` and the rest reach the window from a server shell. One extra ssh connection carries them. Run `ift install-claude-hooks` (or `install-codex-hooks`) on the server once so the agent there calls the hook.
- A card in a remote window now starts in the server's home folder, not a Mac folder that does not exist there, and `ift sessions` no longer lists a session whose shell is gone.
- Fixed: the welcome card came back as a terminal after a relaunch, which started an extra shell each time. On a remote window that left shells on the server that no card owned.

### App

- "Window: change the title bar and Dock colour" (Cmd+Shift+P) works in every window, your own as well as an `ift connect` one. It picks a colour in two steps: a list of named colours, previewed in the title bar and the Dock icon as you move through it (Escape puts it back), or "Custom hex colour…", which asks for a code like `3b82f6`. In your own window the choice is the setting `ui.windowColor` (a name or a hex code, empty for none), which you can also edit by hand, and it tints the title bar and badges the Dock icon. In an `ift connect` window it is saved for that host; "Default: from the host name" goes back to the automatic colour, and `ift connect user@host --color RRGGBB` sets it too.
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
