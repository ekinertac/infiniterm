---
title: Editor cards
description: Editing files on the canvas, ift as your EDITOR, multiple cursors, tabs, saving, and the diff card.
---

An editor card is a file on the canvas: syntax highlighting for 17 languages through tree-sitter, find and replace, go to line (`Ctrl G`), comment toggle, and a file tree beside it (`Cmd K`). There is no LSP, on purpose; the completions are words from your files and, for JSON files with a schema, keys and values (below). A scrollbar at the right edge shows when the file is longer than the card.

## Opening files

- `ift some/file.rs` run inside a terminal card opens the file in an editor laid over that terminal, ready to type in, and waits until you close it, the way vim does. So `EDITOR=ift` or `GIT_EDITOR=ift` works for `git commit`.
- `ift -n some/file.rs` opens it as a card of its own and returns at once. `ift file.rs:20` opens with line 20 in the middle of the view.
- `ift ~/Code/project` opens a card with the file tree rooted there.
- "Editor: open this card's folder" in the command palette does the same from the card you are on: an editor beside it with its file tree on the folder the card is in. It is the twin of "Diff: this card against git HEAD".
- "Editor: open a file…" in the palette opens the macOS file panel: a file opens in an editor card, a folder opens with its tree.

Drag the line between the tree and the text to make the tree wider or narrower; the card keeps that width. "Card: sidebar wider" and "narrower" in the palette do the same from the keyboard.
- `Cmd N` opens an empty editor, and `Cmd` + click on a path in a terminal opens the file beside it.

A file that does not exist yet opens empty, and the first save creates it; closing without saving leaves nothing on disk.

## Typing into it

An editor you arrived at with the arrows is a card like any other and takes no keys, so nothing typed at the canvas can land in a file. Click into the text or press `Enter` to lock the keyboard to it: the ring turns the warning colour and the status bar says so. While locked, `Cmd T` / `Cmd W` open and close tabs, `Cmd 1` to `Cmd 9` jump between them, and `Cmd S` saves. Middle-click on a tab closes it. `Escape` lets go once it has nothing else to close: the first press closes a completion list, the find bar, extra cursors or a selection, and the next one unlocks. `Cmd Escape` unlocks at once, and `Cmd Shift P` opens the command palette from a locked editor, as from any card. To use another chord for it, set `"cmd+escape": null` and, for example, `"cmd+shift+e": "browser.leave"` in `keybindings.json`. A chord can also apply only while an editor is locked, see [bindings that apply only sometimes](../configuration/#bindings-that-apply-only-sometimes).

Typing a quote, a backtick or an opening bracket over selected text wraps it: select `foo`, type `(`, get `(foo)`. The text stays selected, so a second press wraps it again.

## Words from your files

Type two letters and a list of words from the file opens under the caret, nearest to the caret first, then words from the card's other tabs, marked with the tab's name. Where the file has a grammar, words inside comments and strings are left out. From three letters, letters in order match too: `gtl` finds `getTotalLength`. `Tab` takes the top row. `Enter` takes a row only after you chose it with Up or Down, so typing a word and pressing `Enter` is still a new line. `Escape` closes the list. To turn it off, set `"editor.completeWords": false`.

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

## The diff card

`ift diff` opens your changes against git HEAD (`ift diff src/` only under a path). The card is an editor in a read-only diff mode, so it keeps the editor's features.

- **Left:** the changed files, each with its `+added` and `-removed` counts. A long name is cut from the front, so the end of the name stays.
- **Right:** the file's diff. It has the syntax colours of the file's language, two line-number columns (old and new) with `+` and `-` signs, green added lines and red removed lines. Unchanged stretches are folded under "N unchanged lines". There is a real caret and selection, `Cmd C` copies, `Cmd F` finds, and `Cmd B` adds a blame gutter.
- **Keys.** The card locks like an editor card. Click into it or press `Enter` to go in. On `ift diff <folder>`, a second `Enter` opens the file under the list cursor. In the list, `Right` or `Enter` opens the file and puts the focus on its text; `Left` at column 0 goes back to the list. `Cmd Alt Left` and `Cmd Alt Right` also move between the list and the text. `Cmd K` works on the list in three steps: with the list hidden it shows the list and puts the keys in it; with the keys in the text it moves them to the list, which stays visible; with the keys in the list it hides the list. A diff card opens with the list visible, so the first `Cmd K` from the text moves the keys to the list. `Escape` leaves the card, and `Cmd W` on a locked diff closes it. Bare arrow keys no longer work until you have gone in.
- **Not there yet:** a right-click menu with a Copy row, and the picture of a changed image.

## Saving and closing

`Cmd S` saves. Saving an untitled buffer asks for a name, starting on its directory with `untitled.txt` and only the name selected; an existing file is asked about before it is replaced.

Closing unsaved work asks Save, Don't Save (`Cmd D`) or Cancel, as macOS does, and Save closes the card only once the file is written. Unsaved changes survive a quit of the app. A file changed on disk under a clean buffer is reloaded.
