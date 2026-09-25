---
title: The canvas
description: Cards, where new ones go, moving and resizing, zoom, groups and workspaces.
---

Every card sits on one canvas you pan and zoom. Cards never move on their own: a card stays where you put it until you move it.

## Kinds of card

- **Terminal**: a real shell, your login shell, started in the card's directory.
- **Editor**: a file with syntax highlighting (17 tree-sitter grammars), find and replace, go to line, comment toggle, undo, and a file tree beside it (`Cmd K`). No LSP and no completion, on purpose. Unsaved changes survive a quit; closing the card discards them.
- **Diff**: `ift diff` opens the changes against git HEAD, the changed files on the left and one file's diff on the right. `Cmd B` adds a blame gutter.
- **Transcript**: `Cmd I` on a card running Claude Code or Pi opens its session beside it as turns, with tool calls folded under each.
- **Browser**: Chromium inside the card, with the Claude in Chrome extension loaded, so Claude Code can drive a browser on your canvas. `Cmd L` opens the address bar.

## Where a new card goes

`Cmd T` puts a new card in the first free slot of a square block grown from the top-left corner of the workspace. Four cards make a 2x2, nine a 3x3. It does not depend on which card you were on, so you can predict where it lands. A hole left by a closed card is filled first.

A new card is 16:9 and sized to fill the window at 100%. `cards.shape`, `cards.width` and `cards.height` change that.

Other ways to place one:

- `Cmd Shift T` letters every empty slot around the cards; press a letter.
- `Cmd D` / `Cmd Shift D` split the focused card to the right or below. The halves remember each other, so closing one hands its space back.
- Arrow (`Cmd Alt` + arrow) into an empty slot to get a hollow card; `Enter` asks what goes in it.

## Moving and resizing

Drag a card by its top edge or its label. The card stays put while an outline follows the pointer and snaps to the slots the cards around it offer: it wears the focus colour where the card fits and the warning colour where it does not. Drop on free space to move, drop on another card to swap the two, `Escape` to cancel. Any other edge resizes.

`Cmd Z` and `Cmd Shift Z` undo and redo on the canvas: a move, swap, resize, a closed card (it comes back in its slot with a fresh shell) or a new one. Inside an editor they are the buffer's.

Canvas: tidy, in the palette, puts hand-scattered cards back into the block in reading order. `Cmd Z` scatters them again.

## Zoom

- `Cmd =` / `Cmd -` zoom around the focused card, `Cmd` + scroll around the pointer.
- `Cmd 0` actual size, `Cmd 1` fit the focused card, `Cmd 2` fit everything, `Cmd 3` fit the group.
- Double-click a card's frame or label to fit it; double-click empty canvas to fit everything.

Zoomed far out, text is drawn as bars, one per word, so a full card still reads as full. The line is set by what the screen can draw, not a fixed zoom level.

Interface size is separate from zoom: `Cmd Shift =` and `Cmd Shift -` make the app's own chrome (labels, title bar, status bar, palette, dialogs) bigger or smaller, never terminal text.

## Moving between cards

- `Cmd Alt` + arrow, or `Cmd Alt J K L`, moves focus to the neighbour.
- `Ctrl Tab` is Cmd+Tab for cards: hold Ctrl, press Tab to step through the cards you worked in, most recent first, release to go there. Cards you only crossed with the arrows are left out unless you stayed.
- `Cmd F` puts a letter on every card; press one to go there.
- The palette lists every card by name.

`Cmd Shift` + arrow or Shift + click extends a selection; close, split, fit and group then act on all of it.

## Groups and workspaces

A group is a named frame around a set of cards: `Cmd G` groups the focused card, `Cmd Shift G` dissolves its group, `Cmd [` and `Cmd ]` step between groups.

A workspace is a separate canvas with a tab in the title bar. `Cmd Shift N` makes one, `Ctrl 1` to `Ctrl 9` jump to one, `Cmd Shift [` and `Cmd Shift ]` step. Each tab wears one dot per card, in the cards' reading order, lit in the card's state colour, so you can see from another workspace that something there wants you.

## Card labels and numbers

Every card has a label in its corner: the name you gave it (`Cmd Shift R`), else the running process, else the directory. It also wears a number, `#7`, the lowest one free. `ift attach 7` reaches that card's shell from any terminal.

## Closing and protecting

A card closes when its shell exits, so `exit` or Ctrl+D does what `Cmd W` does. `Cmd Ctrl T` reopens the last card you closed, where it was.

`Cmd Shift L` protects a card: `Cmd W` refuses it, its workspace will not close around it, and if its shell exits a fresh one takes over.

`Cmd Shift H` masks a card while somebody reads your screen: a decoy (by default `log stream`) runs over it at the same size, and nothing you type reaches the card underneath. `Enter` or `Escape` lifts it.
