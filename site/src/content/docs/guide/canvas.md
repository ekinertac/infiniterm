---
title: The canvas
description: Cards, where new ones go, moving and resizing, zoom, groups and workspaces.
---

Every card sits on one canvas you pan and zoom. Cards never move on their own: a card stays where you put it until you move it.

## Kinds of card

- **Terminal**: a real shell, your login shell, started in the card's directory. See [Terminal cards](../terminal/).
- **Editor**: a file with syntax highlighting (17 tree-sitter grammars), multiple cursors, find and replace, and a file tree beside it (`Cmd K`). No LSP, on purpose. See [Editor cards](../editor/).
- **Diff**: `ift diff` opens the changes against git HEAD, the changed files on the left and one file's diff on the right. `Cmd B` adds a blame gutter.
- **Transcript**: `Cmd I` on a card running Claude Code or Pi opens its session beside it as turns, with tool calls folded under each.
- **Page**: a Markdown file rendered (headings, lists, code, links), read-only. It never takes the keyboard: the wheel scrolls it, and while it is focused the arrows, Page Up/Down, Space, Home and End do too; every other key stays the canvas's. A scrollbar at the right edge shows when there is more below. Web links open in the browser, links to other `.md` files open as Pages. The "Start here" card a first launch shows is one.
- **Browser**: Chromium inside the card, with the Claude in Chrome extension loaded, so Claude Code can drive a browser on your canvas. `Cmd L` opens the address bar.

## Where a new card goes

`Cmd T` puts a new card in the first free slot of a square block grown from the top-left corner of your cards. Four cards make a 2x2, nine a 3x3. It does not depend on which card you were on, so you can predict where it lands. Holes in your grid of cards are filled first, top row first, before the grid grows.

A new card is 16:9 and sized to fill the window at 100%. `cards.shape`, `cards.width` and `cards.height` change that. `cards.gap` is the space between cards, 25 pixels by default (0 to 200). New cards, splits, a grown card, Canvas: tidy and group frames use it; cards already on the canvas keep their places until you move them or tidy. Dragging still snaps to the 25-pixel grid.

Other ways to place one:

- `Cmd Ctrl T` letters every empty slot around the cards; press a letter.
- `Cmd D` / `Cmd Shift D` split the focused card to the right or below. The halves remember each other, so closing one hands its space back.
- Arrow (`Cmd Alt` + arrow) into an empty slot to get a hollow card; `Enter` asks what goes in it.

## Moving and resizing

Drag a card by its top edge or its label. The card stays put while an outline follows the pointer, and the grid's slots for a card of that size show around it: halves for a half, quarters for a quarter. Near a slot the outline snaps to it; anywhere else it goes where you put it. It wears the focus colour where the card fits and the warning colour where it does not. Drop on free space to move, drop on another card to swap the two, `Escape` to cancel. Any other edge resizes. From the keyboard, `Cmd Alt Shift` + arrow swaps the card with its neighbour.

`Cmd Ctrl Enter` grows a card into the gap it sits in, up to the default size, from whichever corner fills that gap. It grows an empty slot the same way: arrow onto a free slot (it has the size of the card you came from), press `Cmd Ctrl Enter`, and the card you make there has the new size.

`Cmd Z` and `Cmd Shift Z` undo and redo on the canvas: a move, swap, resize or a closed card. Undo never closes a card you opened. Inside an editor they are the buffer's.

Canvas: tidy, in the palette, packs the cards back onto the grid in reading order, a gutter apart. A group or a split pair moves as one block and keeps its inside arrangement. `Cmd Z` scatters them again.

## Zoom

- `Cmd` + scroll, or a pinch on the trackpad, zooms around the pointer. The palette has zoom in and out too. (`Cmd =` and `Cmd -` change the terminal font size; see [Terminal cards](../terminal/).)
- `Cmd 0` actual size, `Cmd 1` fit the focused card, `Cmd 2` fit everything.
- `Cmd 3` fits the selected cards when several are selected. With one card it fits the card's group, or else the block of cards around it (every card within a gutter of the next).
- Double-click a card's frame or label to fit it; double-click empty canvas to fit everything.

Zoomed far out, text is drawn as bars, one per word, so a full card still reads as full. The line is set by what the screen can draw, not a fixed zoom level.

### Maximising a card

`Cmd Shift Enter` makes the focused card fill the window, and pressing it again puts the card back. `"ui.hideLabelWhenMaximised": true` leaves the card's label off while it fills the window.

### How a fit frames the cards

Every fit (`Cmd 1`, `Cmd 2`, `Cmd 3`, the double-clicks) leaves `"ui.fitPadding"` pixels around what it frames, 48 by default; `0` lets the cards touch the window's edges. A fit never zooms past 100%, so a small card stays its own size in the middle of the window; `"ui.fitMagnify": true` lifts that, and a fit then zooms in until the card fills the window. `Cmd 1` on a card that was split frames the whole slot, both halves together; `"ui.fitSplitSlot": false` makes it fit that card alone, so it fills the window.

Interface size is separate from zoom: `Cmd Shift =` and `Cmd Shift -` make the app's own chrome (labels, title bar, status bar, palette, dialogs) bigger or smaller, never terminal text.

## Moving between cards

- `Cmd Alt` + arrow, or `Cmd Alt J K L`, moves focus to the neighbour.
- `Ctrl Tab` is Cmd+Tab for cards: hold Ctrl, press Tab to step through the cards you worked in, most recent first, release to go there. Cards you only crossed with the arrows are left out unless you stayed.
- `Cmd J` puts a letter on every card; press one to go there and frame it. (`Cmd F` finds text inside the card you are on.)
- The palette lists every card by name.

Several cards can be selected at once, and close, split, fit and group then act on all of them:

- Drag on empty canvas to draw a selection rectangle. Every card it touches is selected, and the card nearest where the drag started gets the focus. `Shift` + drag adds to the selection, and `Escape` during the drag puts the previous selection back. A click without a drag deselects.
- `Cmd` + click a card to add it, or a selected card to take it out. On the one card you are in, `Cmd` + click still opens links.
- `Shift` + click on a card's body, label or frame also adds it or takes it out.
- `Cmd Shift` + arrow extends the selection to the neighbour.
- Dragging any selected card by its frame or label moves the whole selection.
- `Cmd Alt Shift` + arrow moves the whole selection one block that way, as one piece: into free space, or trading places with the cards there. It refuses, with a notice, when that would split a card or land in another group. `Cmd Z` undoes it in one step.

While a dialog, the palette, the address bar or the shortcuts panel is open, it keeps every Cmd and Ctrl shortcut to itself, so text shortcuts work in its field and nothing moves the canvas behind it.

## Groups and workspaces

A group is a named frame around a set of cards: `Cmd G` groups the focused card, `Cmd Shift G` dissolves its group, `Cmd [` and `Cmd ]` step between groups.

A workspace is a separate canvas with a tab in the title bar. `Cmd Shift N` makes one, `Ctrl 1` to `Ctrl 9` jump to one, `Cmd Shift [` and `Cmd Shift ]` step. Each tab wears one dot per card, in the cards' reading order, lit in the card's state colour, so you can see from another workspace that something there wants you.

With `"ui.workspaceIsolation": true`, `Ctrl Tab` walks only the current workspace's cards and the status bar counts only them; the palette still lists every card, so it stays the way across.

"Card: move to workspace…" in the palette sends the focused card, or the selection, to another workspace or a new one. The card takes a free slot there and you stay where you are.

## Card labels and numbers

Every card has a label in its corner: the name you gave it (`Cmd Shift R`), else the running process, else the directory. It also wears a number, `#7`, the lowest one free. `ift attach 7` reaches that card's shell from any terminal.

The label sits top right by default. `"ui.cardLabelPosition"` moves it to `"top left"`, `"bottom left"` or `"bottom right"` (the words in either order); a card in an ssh session shows its ssh badge at the other end of the same edge.

## Closing and protecting

A card closes when its shell exits, so `exit` or Ctrl+D does what `Cmd W` does. `Cmd Shift T` reopens the last card you closed, where it was, as in a browser.

Closing a terminal card does not kill its program on the spot: a build or an agent mid-turn runs to its end, and `Cmd Shift T` or `Cmd Z` brings the card back with it still live. See [Terminal cards](../terminal/#closing-a-card).

`Cmd Shift L` protects a card: `Cmd W` refuses it, its workspace will not close around it, and if its shell exits a fresh one takes over.

`Cmd Shift H` masks a card while somebody reads your screen: a decoy (by default `log stream`) runs over it at the same size, and nothing you type reaches the card underneath. `Enter` or `Escape` lifts it.
