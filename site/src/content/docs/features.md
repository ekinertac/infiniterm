---
title: Every feature
description: Everything infiniterm does, one line each, with a link to where the guide explains it.
---

One line per feature. Each links to the part of the guide that explains it.

## Agents and card states

- Four border colours: working, waiting on you, failed, done. [Card states](../guide/card-states/)
- Claude Code, Codex, OpenCode and Pi report through hooks, installed with one command each. [Install](../guide/install/)
- Any command in zsh reports too: a build that runs 5 seconds goes violet, then green or red. [Card states](../guide/card-states/)
- Each workspace tab wears one dot per card, lit in that card's colour. [Groups and workspaces](../guide/canvas/#groups-and-workspaces)
- Done means unseen: a green card goes grey once you have looked at it.
- No notifications and no focus stealing. You look when you are ready.
- Transcript cards show a Claude Code or Pi session as turns, following along while it runs. [Card kinds](../guide/canvas/)
- `ift ls --agents` lists every card that runs an agent, with the session id `claude --resume` takes. [ift and sessions](../guide/ift-and-sessions/)

## The canvas

- One zoomable canvas per workspace. `Cmd 2` fits every card, `Cmd 1` the one you are on. [Canvas](../guide/canvas/)
- A new card takes the next free slot of a grid; nothing moves on its own. Canvas: tidy puts cards back on the grid when you ask.
- Drag a card by its edge or label; it snaps to the grid's slots, and a drop on another card swaps the two. [Moving and resizing](../guide/canvas/#moving-and-resizing)
- Split a card to the right or below, grow it into a gap, or pick a size from a menu.
- Select several cards with a drag or `Cmd` + click, then move, close, fit or group them together.
- `Cmd Alt` + arrow moves to the neighbour; `Ctrl Tab` walks the cards you used, most recent first.
- Groups with a frame and a name; workspaces with tabs you can reorder.
- `Cmd Z` undoes a move, a swap, a resize or a closed card. Undo never closes a card you opened.
- A closed terminal is parked for a minute first, so `Cmd Z` brings its program back mid-work. [Terminal cards](../guide/terminal/#closing-a-card)
- Protect a card (`Cmd Shift L`) so `Cmd W` refuses it and its shell restarts when it exits.
- Mask a card (`Cmd Shift H`) while someone reads your screen.
- The interface size is separate from the zoom (`Cmd Shift =` and `-`).

## Terminal cards

- Shells run in their own small daemon, so they survive a quit, an update and a crash of the app. [Sessions](../guide/ift-and-sessions/#sessions-that-outlive-the-window)
- `ift attach 7` takes over a card's shell from any terminal, also over ssh from a phone.
- Find in the whole scrollback (`Cmd F`), with every match highlighted. [Find](../guide/terminal/#find)
- Visual mode (`Cmd Shift C`) to select and copy with the keyboard. [Visual mode](../guide/terminal/#visual-mode)
- Select on the zsh command line with Shift and arrows, as in any Mac text field.
- `Cmd` + click opens a link or a file path. A file dropped from the Finder pastes its quoted path.
- `Cmd =` and `Cmd -` change the font size in every terminal at once.
- The kitty keyboard protocol, so Shift+Enter is a line break in Claude Code and Pi.
- Programs in a card can ask for macOS permissions (Photos, camera, folders). [Privacy prompts](../guide/terminal/#privacy-prompts)

## Editor, diff and browser cards

- An editor with syntax highlighting for 17 languages, a file tree, find and replace, multiple cursors and tabs. No LSP, on purpose. [Editor cards](../guide/editor/)
- `ift file.rs` in a terminal card edits the file over that card and waits, so `EDITOR=ift` works for `git commit`.
- JSON files with a `$schema` complete their keys and values; setting names complete in `settings.json`. [JSON with a schema](../guide/editor/#json-with-a-schema)
- `ift diff` opens your changes against HEAD, with a blame gutter on `Cmd B`.
- Browser cards are Chromium with tabs and the Claude in Chrome extension, beside the agent that drives them.
- An address bar (`Cmd L`) that takes a URL or a search, with your history.
- Page cards show Markdown read-only; the "Start here" card is one.

## ift, the command line

- Open files, folders and diffs from any shell. [ift and sessions](../guide/ift-and-sessions/)
- `ift send`, `ift read`, `ift close` and `ift run` drive another card by its number. [Driving another card](../guide/ift-and-sessions/#driving-another-card)
- `ift ls`, `ift sessions` and `ift commands` print tables, or tab-separated rows into a pipe.
- `ift usage 30` shows which commands and gestures you used in 30 days, from a local log.
- zsh completion for every verb.

## Servers

- `ift connect user@host` opens a second window whose terminal cards run on that server over ssh. [Servers](../guide/ift-and-sessions/#servers)
- The shells stay on the server when the window closes; connecting again brings the same cards back.
- `--install` puts `ift` and `iftd` on a Linux server through the ssh connection, checksum checked.
- Agent states and `ift` work inside a server's cards too.
- Each server gets its own colour, settings and Dock icon.

## Look and settings

- 522 themes, previewed live as you move through the list. [Themes](../guide/configuration/#themes)
- Settings as flat dotted keys, with every default documented beside your file. Changes apply on save. [Settings](../guide/configuration/#settings)
- Every binding can be changed; chords follow the physical key on any keyboard layout. [Keybindings](../guide/configuration/#keybindings)
- A see-through window, background pictures that rotate, see-through cards, rounded corners, the gap between cards. [The window and the canvas](../guide/configuration/#the-window-and-the-canvas)
- A title bar and Dock colour per window.
- Snippets: plain files in a folder, pasted into the card with `Cmd Ctrl S`. [Snippets](../guide/configuration/#snippets)
- Any file infiniterm writes can be a symlink into your dotfiles; the link stays.

## The app

- Native, in Rust. No Electron, no account, no telemetry. [What the app touches](../guide/configuration/#what-the-app-touches)
- Signed and notarized, and it updates itself; the update check is a plain GET.
- Install with one `curl` line, Homebrew or a DMG. [Install](../guide/install/)
- Free for personal use, $29 per person for work, on the honour system. [Licence](../guide/install/#registering-a-licence)
