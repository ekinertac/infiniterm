# The command key on Linux and Windows: a proposal

What this covers: how the Mac's Cmd chords become chords on Linux and Windows, which have no Cmd key and where Ctrl is also the terminal's key. Status (2026-10-22): a PROPOSAL for Ekin; nothing in the keymap is changed yet. It replaces the Windows branch's "side of Ctrl" design (`keymap::modifier_roles`, `keycode::roles`, `keyboard.commandModifier` = left Ctrl / Win).

Read it before touching `keymap.rs`'s chords or `modifier_roles`, `keycode::roles`, `shortcuts::ModifierLabels`, `editor_keys.rs`, `browser_keys.rs`, or a body's hard-coded `m.platform` chord.

Related: `linux-port.md` (the port), `docs/windows-handoff.md` (the Windows constraints, from ift-windows-port), `keymap.rs` (`DEFAULT_KEYMAP`, the source of the table below).

## Ekin's rule

"Ctrl+Shift for conflicted keys. Ctrl+Shift+C for copy, Ctrl+Shift+V for paste and so on." Where Cmd+K would become Ctrl+K and Ctrl+K is input to a terminal, the app's chord is Ctrl+Shift+K; where Ctrl+K means nothing to a terminal (the digits, `=`, `,`, Tab, Enter), plain Ctrl is the app's.

Ctrl+K is terminal input for every letter (each is a C0 control code: Ctrl+C is ETX, Ctrl+H is backspace, Ctrl+P is history), `[` (ESC), `]`, `\` (SIGQUIT), `/` and `-` (both `^_`, readline's undo), Space (NUL, emacs's mark) and the arrows (word moves in every shell). Ekin's list names 20 of the letters; this proposal treats all 26 the same, which changes nothing in the default keymap (the six letters he left out, H M O P U Y, appear only with Shift there).

## The collisions, and a proposed answer for each

Applied naively, the rule maps two Mac chords onto one Linux chord in 13 places, and leaves 12 chords that already use Cmd+Ctrl with nowhere to go.

1. **A shifted twin.** Cmd+K and Cmd+Shift+K both become Ctrl+Shift+K for a clashing key. Pairs: N (new editor / new workspace), W (close card / close workspace), T (new terminal / reopen), D (split right / split down), Z (undo / redo), L (address bar / protect), R (redraw / rename), G (group / ungroup), `[` and `]` (previous/next card / previous/next workspace), `-` (font smaller / interface smaller), and C (the terminal's copy, a body chord, / visual mode). Proposed: the shifted twin adds Alt, **Ctrl+Alt+Shift+K** (split down is Ctrl+Alt+Shift+D, redo Ctrl+Alt+Shift+Z). A Cmd+Shift chord with no unshifted twin stays Ctrl+Shift (the palette is Ctrl+Shift+P, as in VS Code; mask Ctrl+Shift+H).
2. **The digits.** Cmd+1/2/3 (fit card, fit all, fit group) become Ctrl+1/2/3, which the Mac already uses for workspaces 1 to 9 (Ctrl+N). Proposed: workspaces move to **Alt+1 to Alt+9** (gnome-terminal, Konsole and Chrome on Linux switch tabs with Alt+digit); the fit keys keep Ctrl. Not Ctrl+Shift+digit: Ctrl+Shift+6 is Ctrl+^ on a US board, vim's alternate-file key.
3. **The Cmd+Ctrl layer.** There is no second Cmd-like key off the Mac (Super is the desktop's on Linux and Windows takes every Win chord). Proposed per chord: fullscreen **F11** (every Linux and Windows app); close and leave **Ctrl+Alt+W**; size reset **Ctrl+Alt+Enter**; place a card **Ctrl+Alt+P** and paste a snippet **Ctrl+Alt+V** (new letters: Cmd+Ctrl+T and Cmd+Ctrl+S would land on Ctrl+Alt+T, Ubuntu's open-a-terminal key, and on card size's Ctrl+Alt+S); the emoji panel **no key** (GNOME and Windows have a system picker, Win+. and IBus's Ctrl+.); restart **palette only**; card slide (Cmd+Ctrl+Alt+arrows) and canvas pan (Cmd+Ctrl+Shift+arrows) **palette only**, because every Ctrl+Alt(+Shift)+arrow is already focus move or card swap (pan still has Ctrl+drag, the middle button and the wheel). The palette-only rows are a reduced version and Ekin's call.

With these the table below has no collision (checked by script against all 92 default bindings).

## Traps, by platform

- **AltGr (Windows only).** Windows delivers AltGr as Right Alt plus a synthetic Left Ctrl, so a chord reader sees Ctrl+Alt. Turkish Q types `{ [ ] } \ @ € ~` with AltGr. Every Ctrl+Alt row below is only safe on Windows after ift-windows-port's planned fix: AltGr is not counted as Ctrl+Alt, while a real Ctrl+Alt chord still is (the mechanism is theirs: Right Alt alone is not enough, since on a US layout Right Alt is a plain Alt and a real Left Ctrl held with it must still count). With it, Ctrl+Alt rows are fine. On Linux AltGr is its own modifier (ISO_Level3_Shift) and never reads as Ctrl+Alt: to be checked on the driver with `setxkbmap tr` in step 1b.
- **Chords the system takes first.** Windows: Ctrl+Esc (Start: so `browser.leave`'s Ctrl+Escape is dead there; a double Escape still unlocks), Ctrl+Shift+Esc, Ctrl+Alt+Del, Alt+F4, Alt+Tab, every Win chord. GNOME and Ubuntu: Ctrl+Alt+T (open a terminal: `card.new.ungrouped`'s Ctrl+Alt+T), Ctrl+Alt+Left/Right and Ctrl+Alt+Shift+Left/Right (switch workspace, move window: focus move and card swap left/right; to be checked on a GNOME install, believed default since GNOME 40). KDE, Xfce, Cinnamon: Ctrl+Alt+L locks the screen (`focus.move.right`'s vim-style alias). Older IBus versions open emoji typing on Ctrl+Shift+E (`card.findSelection`): to be checked. Each can be rebound in keybindings.json or in the desktop.
- **Ctrl+Shift as a layout switch** (Windows, when that hotkey is set): only a bare Ctrl+Shift tap switches; Ctrl+Shift+key does not.
- **Physical keys.** Chords are named from the physical key (`keycode.rs`), so Ctrl+Shift+= and Ctrl+Shift+0 stay apart on Turkish Q on both platforms. A binding on a key the set 1 table lacks fails `every_key_the_keymap_binds_has_a_scan_code`; nothing proposed here uses a new key except F11, which gpui names itself (named keys never go through the table).

## What a focused terminal still receives

Every plain Ctrl+letter, so readline, vim, emacs and Claude Code keep all their control keys. Legacy terminals cannot tell Ctrl+Shift+letter from Ctrl+letter (both are the same control code), so the app taking every Ctrl+Shift+letter costs the terminal nothing it could have distinguished; only a program on the kitty keyboard protocol could, and none we know binds them. The terminal does lose: Ctrl+digit, Ctrl+`=`, Ctrl+`,`, Ctrl+Shift+arrows (selection in a few TUIs), Ctrl+Alt+arrows, the Ctrl+Alt+letters in the table (J K L T S R W P V and the Shift ones; emacs's C-M- keys on those letters), Alt+1 to Alt+9 (readline's digit argument) and F11. On the Mac the terminal already loses Ctrl+1 to Ctrl+9.

## Inside a locked editor or browser

No terminal is there, so the card's own tables (`editor_keys.rs`, `browser_keys.rs`, and the editor's hard-coded `m.platform` chords) follow the platform's editors and browsers: plain Ctrl. Copy Ctrl+C, paste Ctrl+V, undo Ctrl+Z, find Ctrl+F, save Ctrl+S in the editor, with Sublime Text's Linux keymap as the reference for the line commands (the editor follows Sublime's defaults); Ctrl+T, Ctrl+W, Ctrl+Shift+T, Ctrl+1 to 9 and Ctrl+R in the browser, as Chrome. The terminal body's copy, paste and cut are Ctrl+Shift+C, V and X.

## The setting

`keyboard.commandModifier`: `ctrl` (the default off the Mac, this table) or `super` (Linux only: the Mac's table with Super for Cmd; the desktop takes many Super chords, so it is opt-in). Not offered on Windows, where Win chords are the system's. The Mac ignores it. The old Windows values (left Ctrl, Win) read as `ctrl`.

## Questions for Ekin

1. The shifted twin: Ctrl+Alt+Shift+K (proposed), or move one of each pair to another key?
2. Workspaces on Alt+1 to Alt+9?
3. The Cmd+Ctrl layer: the keys in item 3, and palette only for card slide, canvas pan and restart?
4. `-` is in your list (readline undo), so font smaller is Ctrl+Shift+- while font bigger is Ctrl+=. Keep that, or treat `-` as free like gnome-terminal does (Ctrl+- smaller, Ctrl+Shift+- interface smaller)?

## The table

macOS chord, command, Linux and Windows chord with `keyboard.commandModifier` = `ctrl`, and the rule that gave it. Generated from `DEFAULT_KEYMAP` (92 bindings); step 1b makes this a pure function in `keymap.rs` with a test that no two commands share a chord.

| macOS | command | Linux / Windows | rule |
|---|---|---|---|
| `cmd+shift+[` | `workspace.prev` | `ctrl+alt+shift+[` | shifted twin: +Alt |
| `cmd+shift+]` | `workspace.next` | `ctrl+alt+shift+]` | shifted twin: +Alt |
| `cmd+shift+n` | `workspace.new` | `ctrl+alt+shift+n` | shifted twin: +Alt |
| `cmd+shift+w` | `workspace.close` | `ctrl+alt+shift+w` | shifted twin: +Alt |
| `ctrl+1` | `workspace.show.1` | `alt+1` | digits are the fit keys |
| `ctrl+2` | `workspace.show.2` | `alt+2` | digits are the fit keys |
| `ctrl+3` | `workspace.show.3` | `alt+3` | digits are the fit keys |
| `ctrl+4` | `workspace.show.4` | `alt+4` | digits are the fit keys |
| `ctrl+5` | `workspace.show.5` | `alt+5` | digits are the fit keys |
| `ctrl+6` | `workspace.show.6` | `alt+6` | digits are the fit keys |
| `ctrl+7` | `workspace.show.7` | `alt+7` | digits are the fit keys |
| `ctrl+8` | `workspace.show.8` | `alt+8` | digits are the fit keys |
| `ctrl+9` | `workspace.show.9` | `alt+9` | digits are the fit keys |
| `cmd+shift+arrowleft` | `focus.extend.left` | `ctrl+shift+arrowleft` | free |
| `cmd+shift+arrowright` | `focus.extend.right` | `ctrl+shift+arrowright` | free |
| `cmd+shift+arrowup` | `focus.extend.up` | `ctrl+shift+arrowup` | free |
| `cmd+shift+arrowdown` | `focus.extend.down` | `ctrl+shift+arrowdown` | free |
| `cmd+alt+arrowleft` | `focus.move.left` | `ctrl+alt+arrowleft` | free |
| `cmd+alt+arrowright` | `focus.move.right` | `ctrl+alt+arrowright` | free |
| `cmd+alt+arrowup` | `focus.move.up` | `ctrl+alt+arrowup` | free |
| `cmd+alt+arrowdown` | `focus.move.down` | `ctrl+alt+arrowdown` | free |
| `cmd+alt+shift+arrowleft` | `card.swap.left` | `ctrl+alt+shift+arrowleft` | free |
| `cmd+alt+shift+arrowright` | `card.swap.right` | `ctrl+alt+shift+arrowright` | free |
| `cmd+alt+shift+arrowup` | `card.swap.up` | `ctrl+alt+shift+arrowup` | free |
| `cmd+alt+shift+arrowdown` | `card.swap.down` | `ctrl+alt+shift+arrowdown` | free |
| `cmd+ctrl+alt+arrowleft` | `card.slide.left` | palette only | Cmd+Ctrl layer, no key |
| `cmd+ctrl+alt+arrowright` | `card.slide.right` | palette only | Cmd+Ctrl layer, no key |
| `cmd+ctrl+alt+arrowup` | `card.slide.up` | palette only | Cmd+Ctrl layer, no key |
| `cmd+ctrl+alt+arrowdown` | `card.slide.down` | palette only | Cmd+Ctrl layer, no key |
| `cmd+alt+j` | `focus.move.left` | `ctrl+alt+j` | Ctrl+Alt |
| `cmd+alt+k` | `focus.move.down` | `ctrl+alt+k` | Ctrl+Alt |
| `cmd+alt+l` | `focus.move.right` | `ctrl+alt+l` | Ctrl+Alt |
| `cmd+0` | `canvas.zoom.actual` | `ctrl+0` | free |
| `cmd+ctrl+shift+arrowleft` | `canvas.pan.left` | palette only | Cmd+Ctrl layer, no key |
| `cmd+ctrl+shift+arrowright` | `canvas.pan.right` | palette only | Cmd+Ctrl layer, no key |
| `cmd+ctrl+shift+arrowup` | `canvas.pan.up` | palette only | Cmd+Ctrl layer, no key |
| `cmd+ctrl+shift+arrowdown` | `canvas.pan.down` | palette only | Cmd+Ctrl layer, no key |
| `cmd+a` | `focus.selectAll` | `ctrl+shift+a` | clash: +Shift |
| `cmd+1` | `canvas.zoom.fitCard` | `ctrl+1` | free |
| `cmd+2` | `canvas.zoom.fitAll` | `ctrl+2` | free |
| `cmd+=` | `terminal.font.bigger` | `ctrl+=` | free |
| `cmd+-` | `terminal.font.smaller` | `ctrl+shift+-` | clash: +Shift |
| `cmd+shift+=` | `ui.scale.up` | `ctrl+shift+=` | free |
| `cmd+shift+-` | `ui.scale.down` | `ctrl+alt+shift+-` | shifted twin: +Alt |
| `cmd+shift+0` | `ui.scale.reset` | `ctrl+shift+0` | free |
| `cmd+shift+enter` | `card.maximize.toggle` | `ctrl+shift+enter` | free |
| `cmd+shift+h` | `card.mask` | `ctrl+shift+h` | Ctrl+Shift, no twin |
| `cmd+l` | `card.omnibox` | `ctrl+shift+l` | clash: +Shift |
| `cmd+t` | `card.new.terminal` | `ctrl+shift+t` | clash: +Shift |
| `cmd+d` | `card.split.right` | `ctrl+shift+d` | clash: +Shift |
| `cmd+shift+d` | `card.split.down` | `ctrl+alt+shift+d` | shifted twin: +Alt |
| `cmd+k` | `card.clear` | `ctrl+shift+k` | clash: +Shift |
| `cmd+alt+t` | `card.new.ungrouped` | `ctrl+alt+t` | Ctrl+Alt |
| `cmd+w` | `card.close` | `ctrl+shift+w` | clash: +Shift |
| `cmd+ctrl+w` | `card.close.leave` | `ctrl+alt+w` | explicit |
| `cmd+ctrl+enter` | `card.size.reset` | `ctrl+alt+enter` | explicit |
| `cmd+z` | `layout.undo` | `ctrl+shift+z` | clash: +Shift |
| `cmd+shift+z` | `layout.redo` | `ctrl+alt+shift+z` | shifted twin: +Alt |
| `cmd+shift+l` | `card.protect` | `ctrl+alt+shift+l` | shifted twin: +Alt |
| `cmd+alt+s` | `card.size` | `ctrl+alt+s` | Ctrl+Alt |
| `cmd+shift+t` | `card.reopen` | `ctrl+alt+shift+t` | shifted twin: +Alt |
| `cmd+s` | `card.save` | `ctrl+shift+s` | clash: +Shift |
| `cmd+n` | `card.new.editor` | `ctrl+shift+n` | clash: +Shift |
| `cmd+b` | `editor.blame` | `ctrl+shift+b` | clash: +Shift |
| `cmd+i` | `card.transcript` | `ctrl+shift+i` | clash: +Shift |
| `cmd+escape` | `browser.leave` | `ctrl+escape` | free |
| `cmd+\` | `card.sidebar.flip` | `ctrl+shift+\` | clash: +Shift |
| `cmd+r` | `terminal.redraw` | `ctrl+shift+r` | clash: +Shift |
| `cmd+shift+r` | `card.rename` | `ctrl+alt+shift+r` | shifted twin: +Alt |
| `cmd+,` | `app.settings` | `ctrl+,` | free |
| `cmd+shift+,` | `app.keybindings` | `ctrl+shift+,` | free |
| `cmd+shift+p` | `app.palette` | `ctrl+shift+p` | Ctrl+Shift, no twin |
| `cmd+ctrl+space` | `app.emoji` | none (system picker) | explicit |
| `cmd+ctrl+f` | `app.fullscreen` | `f11` | explicit |
| `cmd+/` | `app.shortcuts` | `ctrl+shift+/` | clash: +Shift |
| `cmd+ctrl+shift+r` | `app.restart` | palette only | explicit |
| `cmd+ctrl+t` | `card.place` | `ctrl+alt+p` | explicit |
| `cmd+ctrl+s` | `snippet.paste` | `ctrl+alt+v` | explicit |
| `ctrl+tab` | `card.switcher.next` | `ctrl+tab` | unchanged |
| `ctrl+shift+tab` | `card.switcher.prev` | `ctrl+shift+tab` | unchanged |
| `cmd+f` | `card.find` | `ctrl+shift+f` | clash: +Shift |
| `cmd+e` | `card.findSelection` | `ctrl+shift+e` | clash: +Shift |
| `cmd+shift+c` | `terminal.visual` | `ctrl+alt+shift+c` | shifted twin: +Alt |
| `cmd+j` | `focus.hint` | `ctrl+shift+j` | clash: +Shift |
| `cmd+g` | `group.new` | `ctrl+shift+g` | clash: +Shift |
| `cmd+shift+g` | `group.dissolve` | `ctrl+alt+shift+g` | shifted twin: +Alt |
| `cmd+alt+r` | `group.rename` | `ctrl+alt+r` | Ctrl+Alt |
| `cmd+[` | `focus.prev` | `ctrl+shift+[` | clash: +Shift |
| `cmd+]` | `focus.next` | `ctrl+shift+]` | clash: +Shift |
| `cmd+alt+[` | `group.card.prev` | `ctrl+alt+[` | Ctrl+Alt |
| `cmd+alt+]` | `group.card.next` | `ctrl+alt+]` | Ctrl+Alt |
| `cmd+3` | `canvas.zoom.fitGroup` | `ctrl+3` | free |

Body chords outside `DEFAULT_KEYMAP`, for completeness: the terminal's copy, paste and cut (Cmd+C, V, X: `terminal_body.rs`) become Ctrl+Shift+C, V, X; the own-window chord Cmd+Shift+O (`input.rs`) becomes Ctrl+Shift+O; the find bar's step keys follow `group.new` / `group.dissolve` (Ctrl+Shift+G, Ctrl+Alt+Shift+G).
