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

The `.default` files are rewritten at every launch, so editing them does nothing. Read them to see what exists, copy a line into your file and change it there. An upgrade never touches your files; new settings appear in the defaults. Your files can be symlinks into a dotfiles repo: a save writes the file the link points at and keeps the link.

`Cmd ,` opens settings and `Cmd Shift ,` keybindings, each as a pair of editor cards. Changes apply on save, no restart, also when you save from another editor. A file that does not parse, or has the wrong kind of value (text where a number goes), is not applied: your previous settings stay, and the status bar says `settings.json not applied` with the reason until you save a valid file. `keybindings.json` works the same way. Unknown names, numbers out of range and trailing commas are accepted. In `settings.json` the setting names complete as you type (see [JSON with a schema](../editor/#json-with-a-schema)).

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

A binding set to `null` is removed, which gives the key back to the terminal. A chord must hold `cmd`, with two exceptions: `ctrl` plus a digit, and `ctrl+tab`. A terminal needs Ctrl, Alt and bare keys for itself, but Ctrl plus a digit mostly means nothing to a shell. (On an xterm-compatible terminal `Ctrl 3` sends Escape; if you live in vim, rebind it.) A third exception is a chord with a `when` that is false in a terminal, described next.

### Bindings that apply only sometimes

Give a chord an object with a `when` and the binding applies only while that holds:

```jsonc
{
  "cmd+shift+e": { "command": "browser.leave", "when": "editorTextFocus" },
  "cmd+t": { "command": null, "when": "editorTextFocus" }
}
```

The first line binds `Cmd Shift E` to `browser.leave` only in a locked editor. The second unbinds `Cmd T` there and nowhere else (`"command": null` removes the binding in that situation). To give one chord different commands in different places, use a list of objects. The last one whose `when` holds wins. The plain shape above still works.

A `when` joins conditions with `&&` and `||`, negates with `!`, compares with `==` and `!=`, and groups with parentheses. Text goes in single or double quotes, and `true` and `false` are allowed. For example `cardKind == 'editor' && !editorHasSelection`.

| Key | True when |
| --- | --- |
| `cardKind` | the focused card's kind: `terminal`, `editor`, `browser`, `diff`, `transcript`, `page` or `none` |
| `terminalFocus` | a terminal card is focused |
| `editorFocus` | an editor card is focused, arrowed to or locked |
| `editorTextFocus` | an editor card is focused and locked |
| `browserFocus` | a browser card is focused |
| `cardLocked` | the focused editor or browser card holds the keyboard |
| `overlay` | what is open over the canvas: `none`, `palette`, `prompt`, `omnibox`, `shortcuts`, `find` or `switcher` |
| `phantomFocus` | an empty slot is focused |
| `multiSelection` | more than one card is selected |
| `suggestWidgetVisible` | the editor's completion list is open |
| `findWidgetVisible` | the find bar is open |
| `editorHasSelection` | the editor has text selected |

A key without `cmd`, such as `Escape`, `F2` or a letter, can be bound this way, but only with a `when` that is false in a focused terminal: `editorTextFocus`, `browserFocus` or `phantomFocus` do, `cardLocked` does not. That way a binding can never take a terminal's input. Otherwise the file reports an error and that binding is ignored. A mistake inside a `when`, such as an unknown key or a bracket left open, is reported the same way. The shortcuts panel and `ift commands` list only plain bindings so far, not the ones with a `when`.

Chords follow the physical key, not the character your layout prints on it, so `cmd+=` is the key right of `-` on every keyboard. `Cmd H`, `Cmd M` and `Cmd Q` belong to the menu and cannot be rebound. Every command id is on the [commands reference](../../reference/commands/). In the app, the shortcuts panel (`Cmd /`) shows each command's id beside its label and finds commands by it: Up and Down highlight one, and `Cmd C` copies its id, ready to paste into `keybindings.json`.

## Themes

All 500-odd schemes from iTerm2-Color-Schemes ship with the app; Violite is the default. Your own `.itermcolors` files go in:

```
~/Library/Application Support/dev.ekinertac.infiniterm/themes/
```

"Switch theme" in the palette lists the theme you have first, marked active, then every other one A to Z. Each one is applied as you move through the list, with the canvas left undimmed so the cards show its real colours, and the list keeps its order while you do. `Escape`, or a click outside the list, puts back the one you started on; `Enter` keeps the new one. The theme colours the app's chrome, the editor's syntax and the card labels too.

## The window and the canvas

`ui.windowOpacity` (0.1 to 1) lets the desktop show through the canvas, title bar and status bar, and `ui.windowBlur` blurs it. `ui.backgroundImage` puts a picture behind the canvas: `"dusk"`, `"aurora"` or `"ember"` come with the app, or give a path to your own; `ui.backgroundImageFit` is `"cover"` (crop to fill) or `"contain"`. A picture is opaque, so it hides the desktop. Give a list instead, `["dusk", "~/Pictures/lake.jpg"]`, and the pictures take turns in that order: each stays `ui.backgroundImageInterval` seconds (300 by default), with a crossfade of `ui.backgroundImageFade` seconds (2 by default, `0` for a cut). An entry that does not exist is skipped. Only two pictures are in memory at a time, so a long list costs no more than a short one. `ui.cardOpacity` lets the canvas and the picture show through card backgrounds while the text stays solid; browser cards stay opaque. `ui.cardRadius` rounds the corners of cards and group frames, in pixels at 100% zoom (0 to 40, default 0 for square corners); the border, the focus ring, the label and the tabs follow it. The window's own corners stay as macOS draws them. `"ui.showGrid": false` leaves the canvas plain.

"Window: change the title bar and Dock colour" in the palette tints the title bar and the workspace tabs, and badges the Dock icon. Pick a named colour (each one shows as you move through the list, `Escape` puts it back) or "Custom hex colour…" for a code like `3b82f6`. The choice is saved as `ui.windowColor`, a name or a hex code, empty for none. In an `ift connect` window the same command sets that server's colour instead, and "Default: from the host name" goes back to the automatic one.

## Snippets

`Cmd Ctrl S` lists your snippets and pastes the one you pick into the focused card, the same way `Cmd V` would, so Claude takes a multi-line prompt as one block.

Snippets are plain files in `~/.config/infiniterm/snippets/`, one per snippet. The file name without its extension is the snippet's name; the contents are pasted as written, less one trailing line break. The picker's last row opens the folder.

## What the app touches

- Your shells, one per card, started as your login shell with `INFINITERM_CARD_ID` in the environment. Nothing is typed into them that you did not type.
- `~/.config/infiniterm/` for settings, keybindings and snippets, and `~/Library/Application Support/dev.ekinertac.infiniterm/` for the canvas, drafts, themes, the window frame, `agent.log`, the browser profile and address-bar history. Deleting `workspace.json` there resets the canvas and leaves your settings alone. Any of these files can be a symlink: every file infiniterm writes is written where the link points, and the link stays.
- macOS privacy prompts, only when a program in a card asks for something macOS guards (Photos, the camera, your Documents folder and others). Nothing is asked at install or launch. See [privacy prompts](../terminal/#privacy-prompts).
- `ps`, `lsof` and `git` as subprocesses, to label cards and for the diff and blame cards.
- The network only from browser cards, the update check, address-bar suggestions from Google, which are off unless you turn on `browser.suggestions`, one request to api.lemonsqueezy.com when you run `ift licence <email> <key>`, ssh to the hosts you name in `ift connect`, and a download from GitHub for `ift connect --install`.

No account, no telemetry.
