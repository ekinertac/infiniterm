---
title: Terminal cards
description: Font size, find, visual mode, selecting on the zsh command line, and what closing a card does to its program.
---

A terminal card runs your login shell in the card's directory. Every key without Cmd goes to it untouched, so vim, tmux and readline keep working.

## Font size

`Cmd =` and `Cmd -` make the terminal font one point bigger or smaller, in every terminal card at once, and the size is saved. "Terminal: default font size" in the palette goes back to 14. The canvas zoom is `Cmd` + scroll or a pinch.

## Find

`Cmd F` searches the card you are on, the whole scrollback included. Every match is highlighted, the bar shows a count, and it starts on the newest match, so `Enter` steps up the output. `Cmd G` and `Cmd Shift G` step while the bar is open. `Cmd E` searches for the selected text.

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

Closing a terminal card does not kill its program on the spot. A program keeps running for a minute after the card closes, so `Cmd Shift T` or `Cmd Z` brings the card back with its program still live: an agent comes back mid-work, not as a fresh shell in the same folder.

## Links and paths

Hold `Cmd` over a link or a path that exists and it underlines; `Cmd` + click opens it beside the card, a file in an editor card and a URL in a browser card. Dropping a file from the Finder onto a terminal pastes its shell-escaped path.
