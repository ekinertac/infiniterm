---
title: Editor cards
description: Editing files on the canvas, ift as your EDITOR, multiple cursors, tabs, and saving.
---

An editor card is a file on the canvas: syntax highlighting for 17 languages through tree-sitter, find and replace, go to line (`Ctrl G`), comment toggle, and a file tree beside it (`Cmd K`). There is no LSP, on purpose; the one completion is for JSON files with a schema (below). A scrollbar at the right edge shows when the file is longer than the card.

## Opening files

- `ift some/file.rs` run inside a terminal card opens the file in an editor laid over that terminal, ready to type in, and waits until you close it, the way vim does. So `EDITOR=ift` or `GIT_EDITOR=ift` works for `git commit`.
- `ift -n some/file.rs` opens it as a card of its own and returns at once. `ift file.rs:20` opens with line 20 in the middle of the view.
- `ift ~/Code/project` opens a card with the file tree rooted there.
- "Editor: open a file" in the palette opens the macOS file panel: a file opens in an editor card, a folder opens with its tree.

Drag the line between the tree and the text to make the tree wider or narrower; the card keeps that width. "Card: sidebar wider" and "narrower" in the palette do the same from the keyboard.
- `Cmd N` opens an empty editor, and `Cmd` + click on a path in a terminal opens the file beside it.

A file that does not exist yet opens empty, and the first save creates it; closing without saving leaves nothing on disk.

## Typing into it

An editor you arrived at with the arrows is a card like any other and takes no keys, so nothing typed at the canvas can land in a file. Click into the text or press `Enter` to lock the keyboard to it: the ring turns the warning colour and the status bar says so. While locked, `Cmd T` / `Cmd W` open and close tabs, `Cmd 1` to `Cmd 9` jump between them, and `Cmd S` saves. Double `Escape` lets go.

Typing a quote, a backtick or an opening bracket over selected text wraps it: select `foo`, type `(`, get `(foo)`. The text stays selected, so a second press wraps it again.

## JSON with a schema

A `.json` or `.jsonc` file with a `"$schema"` key near the top completes from that schema: a path beside the file, or an `https` address, downloaded once and kept. Type a quote for its keys, or a quote after a colon for the allowed values, `true` / `false` and the default. The list opens while you type a word, not after a space, a comma or a new line, and not when you move the caret.

Your own `settings.json` needs no `$schema`: type `"ui.` and every `ui.*` setting is listed with its default, and the selected one with its description. Letters without a dot match anywhere in the name, so `fitpad` finds `ui.fitPadding`. `Tab` takes the highlighted row. `Enter` takes it only after you moved to it with Up or Down; otherwise `Enter` makes a new line as usual. `Escape` closes the list.

## Multiple cursors

- `Cmd` + click adds a cursor or removes one.
- `Ctrl Shift Up/Down` adds a cursor on the line above or below.
- `Cmd D` adds the next occurrence of the selection; `Ctrl Cmd G` selects every occurrence.
- `Cmd Shift L` splits a selection into one cursor per line.
- `Escape` goes back to one cursor.

Typing, deleting and moving work at every cursor. Copy, paste and the line commands still act on the main one, and an edit made at several cursors takes several `Cmd Z` to undo.

## Saving and closing

`Cmd S` saves. Saving an untitled buffer asks for a name, starting on its directory with `untitled.txt` and only the name selected; an existing file is asked about before it is replaced.

Closing unsaved work asks Save, Don't Save (`Cmd D`) or Cancel, as macOS does, and Save closes the card only once the file is written. Unsaved changes survive a quit of the app. A file changed on disk under a clean buffer is reloaded.
