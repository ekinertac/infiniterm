---
title: Terminal cards
description: Font size, find, visual mode, selecting on the zsh command line, and what closing a card does to its program.
---

A terminal card runs your login shell in the card's directory. Every key without Cmd goes to it untouched, so vim, tmux and readline keep working.

## Font size

`Cmd =` and `Cmd -` make the terminal font one point bigger or smaller, in every terminal card at once, and the size is saved. "Terminal: default font size" in the palette goes back to 14. The canvas zoom is `Cmd` + scroll or a pinch.

## Find

`Cmd F` searches the card you are on, the whole scrollback included. The bar opens inside that card, at its top right under the label. Every match is highlighted, the bar shows a count, and it starts on the newest match, so `Enter` steps up the output. `Cmd G` and `Cmd Shift G` step while the bar is open. `Cmd E` searches for the selected text.

Lowercase searches ignore case; a capital letter makes the search match case. `Escape` closes the bar and leaves the current match selected where it is, so `Cmd C` copies it.

## Visual mode

`Cmd Shift C` puts a keyboard cursor over the card's output and scrollback, for copying without the mouse. The card wears a VISUAL tag, and nothing you type reaches the shell while it is on.

- Move with the arrows, `Alt` or `Cmd` + arrows, Home/End and Page Up/Down, or with vim's `h j k l`, `w b e`, `0 $`, `g G`.
- `Shift` + a move selects, the Mac way. `v`, `V` and `Ctrl V` start a character, line or block selection, the vim way.
- `Cmd C` or `y` copies and keeps you in the mode.
- `Escape` is the only way out.

`Cmd Shift` + arrow stays the canvas's selection key, so select to a line end with `Shift End`.

## Selecting on the command line

In zsh, the line you are typing selects like a Mac text field:

- `Shift Left/Right`, `Shift Alt Left/Right` and `Shift Home/End` select.
- Typing, pasting, `Backspace` or `Delete` replace the selection.
- `Cmd C` copies it and `Cmd X` cuts it. A mouse selection in the output wins over it for `Cmd C`.
- `Cmd Backspace` and `Cmd Delete` delete to the start and the end of the line, in Claude Code too.

This comes from the shell integration the app installs, so it works only in shells started after you installed or updated the app, and only in zsh. There is no mouse selection or click-to-move on the command line.

## Closing a card

Closing a terminal card does not kill its program on the spot:

- A build, or an agent in the middle of a turn, runs to its end. Anything that has been quiet for a minute (no output, no CPU under its shell, no agent turn) is then ended.
- A closed card whose agent is waiting for you is kept until the app quits. The status bar says so, "#7 … (closed) is waiting for you: Alt+T", and `Alt T` reopens it. At any other time `Alt T` goes to the terminal as usual.
- `Cmd Z` or `Cmd Shift T` reopen the card with its program still live and the screen redrawn from its scrollback, as long as it has not ended yet.

This needs the default backend (`terminal.backend: "daemon"`). Under `pty` or `tmux` a closed card's program ends at once.

## Settings worth knowing

These go in `settings.json`; every setting is on the [settings reference](../../reference/settings/).

- `"terminal.copyOnSelect": true` copies a mouse selection the moment you let go.
- `"terminal.scrollMultiplier"` sets scroll speed, from 0.1 to 10 (default 1).
- `"terminal.padding"` is the space between a card's edge and its text, 0 to 60 (default 6). Open cards resize to it at once.
- `"terminal.env": ["EDITOR=ift"]` adds environment variables to new cards.
- `"terminal.shell"` takes arguments too, like `"/bin/zsh -l"`.

## Cursor colour

A block cursor draws the letter under it in the theme's cursor text colour, or black or white when that would not contrast. `terminal.cursorColor` and `editor.cursorColor` (a hex colour such as `"#ff9900"`; empty follows the theme) set the cursor's colour, and the letter under it follows.

## Links and paths

Hold `Cmd` over a link or a path that exists and it underlines; `Cmd` + click opens it beside the card, a file in an editor card and a URL in a browser card. A double-click on a link selects the whole address. Links a program marks itself (OSC 8: `ls --hyperlink`, `rg --hyperlink-format`, `gcc`, and the markdown links in an agent's reply) are underlined at rest and open on `Cmd` + click, whatever their visible text says; only `http` and `https` addresses open, since the text of a link can lie. Every new terminal card sets `FORCE_HYPERLINK=1` so Claude Code prints real links; a card that is already open keeps its old environment, and `"terminal.env": ["FORCE_HYPERLINK=0"]` turns it off. Issue and pull request numbers are links too: in a card whose directory is a GitHub checkout, `#349` opens that issue or pull request, and `owner/repo#349` works in any card. A bare number needs a space, bracket or quote before it and no letter after it. Holding `Cmd` over any link draws a strong double underline and turns the pointer to a hand. Dropping a file from the Finder onto a terminal pastes its shell-escaped path.

## Privacy prompts

A program in a card can use what macOS guards: Photos, Contacts, Calendars, Reminders, the camera, the microphone, speech recognition, your location, Bluetooth, other apps (Automation), the Desktop, Documents and Downloads folders, Accessibility, Screen Recording, Input Monitoring and the local network. The first time a program asks, macOS shows its usual dialog. Nothing is asked when you install or launch infiniterm, and infiniterm itself uses none of them.

The grant is listed under infiniterm in System Settings > Privacy & Security, because macOS sees the app, not the program in the card. Each one is asked once; change it there later.

Full Disk Access is never asked for. If a script needs it, for example to read `~/Library/Mail`, add infiniterm by hand in System Settings > Privacy & Security > Full Disk Access.

HomeKit and Siri are not available to programs in a card.
