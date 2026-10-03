---
title: Configuration
description: Settings and keybindings files, themes, snippets, and what the app touches.
---

Configuration lives in `~/.config/infiniterm/`, four files in two pairs:

```
settings.json              yours, only what you changed
settings.default.json      every setting with its default, commented
keybindings.json           yours, only what you rebound
keybindings.default.json   every binding, commented
```

The `.default` files are rewritten at every launch, so editing them does nothing. Read them to see what exists, copy a line into your file and change it there. An upgrade never touches your files; new settings appear in the defaults.

`Cmd ,` opens settings and `Cmd Shift ,` keybindings, each as a pair of editor cards. Changes apply on save, no restart.

## Settings

Keys are flat and dotted, so an override is one line:

```jsonc
{
  "terminal.fontSize": 16,
  "ui.showFps": true
}
```

Comments and trailing commas are allowed. Every setting is on the [settings reference](../../reference/settings/).

## Keybindings

```jsonc
{
  "cmd+shift+k": "card.clear",
  "cmd+k": null
}
```

A binding set to `null` is removed, which gives the key back to the terminal. Every chord must hold `cmd`, with two exceptions: `ctrl` plus a digit, and `ctrl+tab`. A terminal needs Ctrl, Alt and bare keys for itself, but Ctrl plus a digit mostly means nothing to a shell. (On an xterm-compatible terminal `Ctrl 3` sends Escape; if you live in vim, rebind it.)

Chords follow the physical key, not the character your layout prints on it, so `cmd+=` is the key right of `-` on every keyboard. `Cmd H`, `Cmd M` and `Cmd Q` belong to the menu and cannot be rebound. Every command id is on the [commands reference](../../reference/commands/). In the app, the shortcuts panel (`Cmd /`) shows each command's id beside its label and finds commands by it: Up and Down highlight one, and `Cmd C` copies its id, ready to paste into `keybindings.json`.

## Themes

All 500-odd schemes from iTerm2-Color-Schemes ship with the app; Violite is the default. Your own `.itermcolors` files go in:

```
~/Library/Application Support/dev.ekinertac.infiniterm/themes/
```

"Switch theme" in the palette lists the theme you have first, marked active, then every other one A to Z. Each one is applied as you move through the list, and the list keeps its order while you do. `Escape` puts back the one you started on; `Enter` keeps the new one. The theme colours the app's chrome, the editor's syntax and the card labels too.

## The window and the canvas

`ui.windowOpacity` (0.1 to 1) lets the desktop show through the canvas, title bar and status bar, and `ui.windowBlur` blurs it. `ui.backgroundImage` puts a picture behind the canvas: `"dusk"`, `"aurora"` or `"ember"` come with the app, or give a path to your own; `ui.backgroundImageFit` is `"cover"` (crop to fill) or `"contain"`. A picture is opaque, so it hides the desktop. `ui.cardOpacity` lets the canvas and the picture show through card backgrounds while the text stays solid; browser cards stay opaque. `"ui.showGrid": false` leaves the canvas plain.

## Snippets

`Cmd Ctrl S` lists your snippets and pastes the one you pick into the focused card, the same way `Cmd V` would, so Claude takes a multi-line prompt as one block.

Snippets are plain files in `~/.config/infiniterm/snippets/`, one per snippet. The file name without its extension is the snippet's name; the contents are pasted as written, less one trailing line break. The picker's last row opens the folder.

## What the app touches

- Your shells, one per card, started as your login shell with `INFINITERM_CARD_ID` in the environment. Nothing is typed into them that you did not type.
- `~/.config/infiniterm/` for settings, keybindings and snippets, and `~/Library/Application Support/dev.ekinertac.infiniterm/` for the canvas, drafts, themes, the window frame, `agent.log`, the browser profile and address-bar history. Deleting `workspace.json` there resets the canvas and leaves your settings alone.
- `ps`, `lsof` and `git` as subprocesses, to label cards and for the diff and blame cards.
- The network only from browser cards, the update check, address-bar suggestions from Google, which are off unless you turn on `browser.suggestions`, and one request to api.lemonsqueezy.com when you run `ift licence <email> <key>`.

No account, no telemetry.
