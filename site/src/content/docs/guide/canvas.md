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
- **Browser**: Chromium inside the card, with the Claude in Chrome extension loaded, so Claude Code can drive a browser on your canvas. "Browser: open DevTools" in the palette opens Chromium's DevTools for the page in a card beside it. It uses a debugging port on a random loopback port that only that DevTools page may use; a program running on your Mac could still attach to it, so `"browser.devtools": false` turns the feature off (restart the app after changing it). `Cmd L` opens the address bar, and so does `Cmd T` on the new blank tab. Middle-click or `Cmd` + click on a link opens it in a new tab, middle-click on a tab closes it, and the mouse's back and forward buttons walk the history (Back on a tab a link opened closes it).

## Where a new card goes

`Cmd T` puts a new card in the first free slot of a square block grown from the top-left corner of your cards. Four cards make a 2x2, nine a 3x3. It does not depend on which card you were on, so you can predict where it lands. Holes in your grid of cards are filled first, top row first, before the grid grows.

A new card is 16:9 and sized to fill the window at 100%. `cards.shape`, `cards.width` and `cards.height` change that. `cards.gap` is the space between cards, 25 pixels by default (0 to 200). New cards, splits, a grown card, Canvas: tidy and group frames use it; cards already on the canvas keep their places until you move them or tidy. Dragging still snaps to the 25-pixel grid.

Other ways to place one:

- `Cmd Ctrl T` letters every empty slot around the cards; press a letter.
- `Cmd D` / `Cmd Shift D` split the focused card to the right or below. The halves remember each other, so closing one hands its space back.
- Arrow (`Cmd Alt` + arrow) into an empty slot to get a hollow card; `Enter` asks what goes in it.

## Moving and resizing

Drag a card by its top edge or its label. The card stays put while an outline follows the pointer, and the grid's slots for a card of that size show around it: halves for a half, quarters for a quarter. Near a slot the outline snaps to it; anywhere else it goes where you put it. It wears the focus colour where the card fits and the warning colour where it does not. Drop on free space to move, drop on another card to swap the two, `Escape` to cancel. Any other edge resizes. From the keyboard, `Cmd Alt Shift` + arrow swaps the card with its neighbour.

`Cmd Ctrl Alt` + arrow slides the card that way into the next gap, which closes the space a resize (`Cmd Alt S`) leaves. The card stops one gutter short of the next card in line, or on the next grid slot when nothing is there, and never overlaps. `Cmd Z` undoes it.

`Cmd Ctrl Enter` grows a card into the gap it sits in, up to the default size, from whichever corner fills that gap. It grows an empty slot the same way: arrow onto a free slot (it has the size of the card you came from), press `Cmd Ctrl Enter`, and the card you make there has the new size.

`Cmd Z` and `Cmd Shift Z` undo and redo on the canvas: a move, swap, resize or a closed card. Undo never closes a card you opened. Inside an editor they are the buffer's.

Canvas: tidy, in the palette, packs the cards back onto the grid in reading order, a gutter apart. A group or a split pair moves as one block and keeps its inside arrangement. `Cmd Z` scatters them again.

## Zoom

- `Cmd` + scroll, or a pinch on the trackpad, zooms around the pointer. The palette has zoom in and out too. (`Cmd =` and `Cmd -` change the terminal font size; see [Terminal cards](../terminal/).)
- `Cmd 0` actual size, `Cmd 1` fit the focused card, `Cmd 2` fit everything.
- `Cmd 3` fits the selected cards when several are selected. With one card it fits the card's group, or else the block of cards around it (every card within a gutter of the next).
- Double-click a card's frame or label to fit it; double-click empty canvas to fit everything. A terminal card drawn as bars (only when `ui.textAsBars` is on, see below) fits on a double-click anywhere on it.

Cards draw their text at every zoom. On a slow Mac you can turn on `"ui.textAsBars": true`: far out, or with many cards on screen, text is then drawn as bars, one per word, so a full card still reads as full. `ui.minTextPx` (7 by default) is how big a glyph must be, in device pixels, before text replaces bars, and `ui.glyphBudget` is how many terminal cells may be on screen before every card turns to bars. Neither does anything while `ui.textAsBars` is off.

### Zooming in on a card's bottom

`Cmd 1` on a terminal card that is already fitted zooms in to 150% with the card's bottom edge at the bottom of the window, which is where a long agent answer ends. The status bar says "zoomed". `ui.cardZoom` sets the zoom (1.1 to 4, and `0` turns the behaviour off, so a second `Cmd 1` does nothing new); the older name `ui.readZoom` still works.

While it is on, `Cmd Up` and `Cmd Down` pan three rows inside the card. Typing and `Enter` still reach the shell and bring the view back to the bottom. `Cmd 1` again or `Cmd 2` leaves it, and so does panning or zooming by hand. The palette has it as "Canvas: zoom in on the focused card's bottom".

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

While a dialog, the palette, the address bar or the shortcuts panel is open, it keeps every Cmd and Ctrl shortcut to itself, so text shortcuts work in its field and nothing moves the canvas behind it. Their rows and buttons also take a click, and a click outside closes any of them, as `Escape` does. Anything you can click in them shows a hand pointer and lights its row.

## Groups and workspaces

A group is a named frame around a set of cards: `Cmd G` groups the focused card, `Cmd Shift G` dissolves its group, `Cmd [` and `Cmd ]` step between groups.

A workspace is a separate canvas with a tab in the title bar. `Cmd Shift N` makes one, `Ctrl 1` to `Ctrl 9` jump to one, `Cmd Shift [` and `Cmd Shift ]` step. Each tab wears one dot per card, in the cards' reading order, lit in the card's state colour, so you can see from another workspace that something there wants you.

Drag a tab onto another tab to put it in that place. From the keyboard, "Workspace: move this tab left" and "move this tab right" in the palette do the same. The order is saved, and `Ctrl 1` to `Ctrl 9` follow it.

With `"ui.workspaceIsolation": true`, `Ctrl Tab` walks only the current workspace's cards and the status bar counts only them; the palette still lists every card, so it stays the way across.

"Card: move to workspace…" in the palette sends the focused card, or the selection, to another workspace or a new one. The card takes a free slot there and you stay where you are. "Group: move to workspace…" does the same for the focused card's whole group, with its frame, its name and its cards in the same arrangement and sizes. It lands in the first free block of the other workspace.

## Card labels and numbers

Every card has a label in its corner: the name you gave it (`Cmd Shift R`), else the running process, else the directory. It also wears a number, `#7`, the lowest one free. `ift attach 7` reaches that card's shell from any terminal.

The label sits top right by default. `"ui.cardLabelPosition"` moves it to `"top left"`, `"bottom left"` or `"bottom right"` (the words in either order); a card in an ssh session shows its ssh badge at the other end of the same edge.

### Locking a browser card

A page needs the keyboard for itself, so a browser card works like a locked editor. Click into the page, or press `Enter` on a card you arrived at with the arrows, and the keyboard is the page's: the ring turns the warning colour and the status bar says so. Chrome's own chords work then (`Cmd T`, `Cmd W`, `Cmd 1` to `Cmd 9` for tabs). Press `Escape` twice within 400 ms to let go; a single `Escape` goes to the page, which uses it. `Cmd Escape` lets go at once. `Cmd Shift P` opens the command palette from a locked page too.

## The right-click menu

Right-click opens the native macOS menu, with each command's shortcut at the right edge and the rows that cannot run now greyed. What it holds depends on where you click:

- **A terminal**: Copy, Paste, Find…, Search the Web for Selection, Split Right and Down, New (Terminal, Editor, Browser…, Claude Code), Clear Buffer, Show Transcript, Rename…, Protect, Mask, Size, Move to Workspace… and Close.
- **A card's frame or label**: Rename…, Group…, Protect, Clear State Colour, Size, Move to Workspace…, the splits, New and Close.
- **Bare canvas**: New, Fit All Cards, Actual Size, Tidy into a Block, Command Palette…, Settings and Keyboard Shortcuts.
- **A workspace tab** (the click does not switch to it): Rename…, New Workspace, Move Left, Move Right and Close Workspace.
- **An editor**: Save, Find…, Go to Line…, Transform (every text transform), Show File Tree, Blame, Rename…, Size and Close.
- **A file or folder in an editor's tree**: the row lights up while the menu is open. Open, Reveal in Finder, Copy Path and Copy Relative Path.
- **A tab of an editor or browser card**: switches to that tab, then New Tab, Close Tab and Reopen Closed Tab. A browser tab also has Reload, Copy Address and Open in System Browser.
- **A browser page**: Back and Forward when there is somewhere to go, Reload, Cut, Copy and Paste where they apply, and Open Page in System Browser. On a link: Copy Link Address, Open Link in New Tab, Open Link in New Card and Open Link in System Browser.

With several cards selected, a right-click on one of them keeps the selection and opens a menu for all of them: Group, Protect, Clear State Colour, Fit Selection, Move to Workspace… and Close. Rename, Split and Size are left out, since they are for one card. A right-click on a card outside the selection selects that card alone.

A terminal program that uses the mouse, such as vim, tmux or htop, still gets the plain right-click. Hold `Shift` to open the menu over it.

A label that ends in an ellipsis, in the menu and in the command palette, asks you for something more before it acts: a name, a file, an address. One without it acts at once.

## Closing and protecting

A card closes when its shell exits, so `exit` or Ctrl+D does what `Cmd W` does. `Cmd Shift T` reopens the last card you closed, where it was, as in a browser.

Closing a terminal card does not kill its program on the spot: a build or an agent mid-turn runs to its end, and `Cmd Shift T` or `Cmd Z` brings the card back with it still live. See [Terminal cards](../terminal/#closing-a-card).

`Cmd Shift L` protects a card: `Cmd W` refuses it, its workspace will not close around it, and if its shell exits a fresh one takes over.

`Cmd Shift H` masks a card while somebody reads your screen: a decoy (by default `log stream`) runs over it at the same size, and nothing you type reaches the card underneath. `Enter` or `Escape` lifts it.
