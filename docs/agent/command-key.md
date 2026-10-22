# The command key on Linux and Windows

What this covers: how the Mac's Cmd chords become chords on Linux and Windows, which have no Cmd key and where Ctrl is also the terminal's key. Status: DECIDED by Ekin (2026-10-22) and built (`infiniterm-core/src/platform_keys.rs`). It replaced the Windows branch's "side of Ctrl" design (`keymap::modifier_roles`, left Ctrl for the app and right Ctrl for the terminal), which is gone.

Read it before touching `keymap.rs`'s chords, `platform_keys.rs`, `keycode::roles`, `input.rs::platform_keystroke`, `shortcuts::ModifierLabels`, `editor_keys.rs`, `browser_keys.rs`, or a body's hard-coded `m.platform` chord.

Related: `linux-port.md` (the port), `docs/windows-handoff.md` (the Windows constraints, from ift-windows-port), `keymap.rs` (`DEFAULT_KEYMAP`, the source of the table below).

## The rule

Ekin: "Ctrl+Shift for conflicted keys. Ctrl+Shift+C for copy, Ctrl+Shift+V for paste and so on." Where Cmd+K would become Ctrl+K and Ctrl+K is input to a terminal, the app's chord is Ctrl+Shift+K; where Ctrl+K means nothing to a terminal, plain Ctrl.

Ctrl+K is terminal input for every letter (each is a C0 control code: Ctrl+C is ETX, Ctrl+H is backspace, Ctrl+P is history), `[` (ESC), `]`, `\` (SIGQUIT), `/` (readline's undo in xterm) and Space (NUL, emacs's mark). Free keys: the digits, `=`, `-` (free by Ekin's decision, as in gnome-terminal, Alacritty and WezTerm: Ctrl+- is font smaller, although it is also `^_`), `,`, Enter, Escape, and the arrows with another modifier.

Decisions on the cases the plain rule does not settle:

1. **A shifted twin.** Cmd+K and Cmd+Shift+K would both become Ctrl+Shift+K for a clashing key. The shifted one adds Alt: Ctrl+Alt+Shift+K (new workspace Ctrl+Alt+Shift+N, reopen Ctrl+Alt+Shift+T, redo Ctrl+Alt+Shift+Z, visual mode Ctrl+Alt+Shift+C against copy's Ctrl+Shift+C). A Cmd+Shift chord with no unshifted twin stays Ctrl+Shift (the palette Ctrl+Shift+P, as in VS Code). Split down was checked against Windows Terminal's Alt+Shift+D and kept Ctrl+Alt+Shift+D: in a terminal Alt+Shift+D is readline's Meta-d (kill-word), since readline runs an unbound uppercase Meta key as its lowercase one, in bash and in zsh's emacs keymap.
2. **The digits.** The workspaces move from Ctrl+1..9 to Alt+1..9 (gnome-terminal, Konsole, Windows Terminal and Chrome on Linux switch tabs with Alt+digit); Ctrl+1, Ctrl+2, Ctrl+3 and Ctrl+0 are the fit keys, as Cmd+digit on the Mac. Ctrl+4 to Ctrl+9 are left to the terminal, like every other Linux terminal: our encoder sends xterm's legacy bytes for them (Ctrl+4 is `^\`, SIGQUIT in a default tty; Ctrl+7 `^_`; Ctrl+8 DEL), so a Mac habit of "workspace 4" quits the foreground program. Ekin may still change that.
3. **The Cmd+Ctrl layer** (no second Cmd-like key exists off the Mac): full screen F11, close and leave Ctrl+Alt+W, size reset Ctrl+Alt+Enter, place a card Ctrl+Alt+P, paste a snippet Ctrl+Alt+V; the emoji panel no key (GNOME and Windows have a system picker); restart, card slide and canvas pan palette only (Ekin accepted the reduced version; pan still has Ctrl+drag, the middle button and the wheel).

## How it is built

The keymap stays written in Mac chords. `PlatformKeys::build` takes every Mac chord in play (the active keymap, the `when` bindings, and `BODY_CHORDS`: the terminal's copy, paste and cut, and the own-window chord, which a body handles itself) and makes ONE two-way table: forward for what the panel and the menus show, reverse for input. One table, because the twin rule depends on which chords are bound. A user binding that lands on another's chord, or a Cmd+Ctrl chord the layer does not name, is reported as a notice when the keymap loads.

Input: `input.rs::platform_keystroke` turns each key-down into the Mac chord it stands for, once, where the event enters, so the thirty-odd reads of `m.platform` downstream and every Mac chord in the code stay as they are. It builds the PHYSICAL chord the way the keymap builds every chord (the physical key from `keycode.rs`, the real Shift), looks it up, and leaves an event the table does not name untouched (a plain Ctrl+letter on its way to a terminal). When the Mac chord's Shift or key differs from what was pressed (Ctrl+Shift+T is Cmd+T, F11 is Cmd+Ctrl+F), `keycode::forget_last` drops the physical record for that event so the rest builds the chord from the rewritten keystroke. `own_window.rs` does the same with the canvas rule (its card is a terminal). `INFINITERM_KEYLOG=1` prints `[platform] ctrl+shift+t -> cmd+t`.

Inside a card that owns the keyboard (a locked editor or browser) no terminal is listening, so plain Ctrl is Cmd there wholesale (Ctrl+C copy, Ctrl+Shift+T reopen a browser tab), as in every Linux editor and browser; Ctrl+Tab stays the switcher's and Alt+digit the workspaces'.

The mouse: off the Mac with `ctrl`, Ctrl plays Cmd's part (`keycode::roles`: Ctrl+click opens a link, Ctrl+scroll zooms, Ctrl+drag pans). The card switcher reads the PHYSICAL Ctrl (`on_modifiers_changed`), since it commits when Ctrl comes up.

The setting: `keyboard.commandModifier` is `ctrl` (the default off the Mac, this table) or `super` (Linux only: the Mac's chords with Super for Cmd, Ctrl left whole for the terminal, opt-in because desktops take many Super chords; no rewriting at all). On Windows `super` reads as `ctrl`, since Windows takes every Win chord. The old Windows values `leftControl` and `win` read as `ctrl`. The Mac ignores the setting.

keybindings.json: in THIS change it still reads Mac chords on every platform (a Linux user writes `cmd+t` and gets Ctrl+Shift+T). Ekin decided (2026-10-22, "do what vscode does") the next step, a separate PR: off the Mac the file uses the chords the panel shows (`ctrl+shift+t`), with no portable `cmd` alias; `cmd` (and `super`, `win`, `meta`) there names the OS key itself, Super on Linux and Win on Windows. Then `keybindings.default.json` is generated with the platform's chords and a comment block saying so; the "every chord has cmd" rule becomes the platform's rule (a chord must not take input a focused terminal needs: plain Ctrl+letter, or a bare key without a `when` that is false in a terminal, each refused with a notice saying what to write instead); and a file in Mac vocabulary copied to Linux gets a notice per line, never a silent drop. The reverse table here already knows every platform chord, which is what that change needs.

Not in this change: the shortcuts panel, the menus, the palette's hints, the keycast and `ift commands` still show Mac chords (the next PR shows the platform chord through `PlatformKeys::platform`); and the Windows side (AltGr not counted as Ctrl+Alt, scan codes, the welcome card's "Cmd" strings), which is ift-windows-port's.

## Traps, by platform

- **AltGr (Windows only).** Windows delivers AltGr as Right Alt plus a synthetic Left Ctrl, so a chord reader sees Ctrl+Alt. Turkish Q types `{ [ ] } \ @ € ~` with AltGr. Every Ctrl+Alt row below is only safe on Windows after ift-windows-port's fix: AltGr is not counted as Ctrl+Alt, while a real Ctrl+Alt chord still is (the mechanism is theirs: Right Alt alone is not enough, since on a US layout Right Alt is a plain Alt and a real Left Ctrl held with it must still count). On Linux AltGr is its own modifier (ISO_Level3_Shift) and never reads as Ctrl+Alt.
- **Chords the system takes first.** Windows: Ctrl+Esc (Start: so `browser.leave`'s Ctrl+Escape is dead there; a double Escape still unlocks), Ctrl+Shift+Esc, Ctrl+Alt+Del, Alt+F4, Alt+Tab, every Win chord. GNOME and Ubuntu: Ctrl+Alt+T (open a terminal: `card.new.ungrouped`), Ctrl+Alt+Left/Right and Ctrl+Alt+Shift+Left/Right (switch workspace, move window: focus move and card swap left/right; believed default since GNOME 40, not checked on a GNOME install). KDE, Xfce, Cinnamon: Ctrl+Alt+L locks the screen (`focus.move.right`'s vim-style alias). Older IBus versions open emoji typing on Ctrl+Shift+E (`card.findSelection`), not checked. Each can be rebound in keybindings.json or in the desktop.
- **Ctrl+Shift as a layout switch** (Windows, when that hotkey is set): only a bare Ctrl+Shift tap switches; Ctrl+Shift+key does not.
- **Physical keys.** Chords are named from the physical key (`keycode.rs`), so Ctrl+Shift+= and Ctrl+Shift+0 stay apart on Turkish Q on both platforms. F11 is the only new key, and gpui names it itself.

## What a focused terminal still receives

Every plain Ctrl+letter, so readline, vim, emacs and Claude Code keep all their control keys. Legacy terminals cannot tell Ctrl+Shift+letter from Ctrl+letter (both are the same control code), so the app taking Ctrl+Shift+letters costs the terminal nothing it could have distinguished; only a program on the kitty keyboard protocol could, and none we know binds them. Ctrl+4 to Ctrl+9 reach it (item 2). The terminal loses: Ctrl+0..3, Ctrl+`=`, Ctrl+`-` (its `^_`, readline's undo, which Ctrl+7 and Ctrl+Shift+- still send in other terminals), Ctrl+`,`, Ctrl+Shift+arrows (selection in a few TUIs), Ctrl+Alt+arrows, the Ctrl+Alt+letters in the table (J K L T S R W P V and the Shift ones; emacs's C-M- keys on those letters), Alt+1 to Alt+9 (readline's digit argument) and F11.

## The table

macOS chord, command, and the Linux and Windows chord with `keyboard.commandModifier` = `ctrl`. Generated by `platform_keys::default_table_markdown` from `DEFAULT_KEYMAP` and `BODY_CHORDS`; `the_doc_table_is_the_one_the_code_builds` fails when this copy differs, so regenerate it rather than editing it.

<!-- table:begin -->
| macOS | command | Linux / Windows |
|---|---|---|
| `cmd+shift+[` | `workspace.prev` | `ctrl+alt+shift+[` |
| `cmd+shift+]` | `workspace.next` | `ctrl+alt+shift+]` |
| `cmd+shift+n` | `workspace.new` | `ctrl+alt+shift+n` |
| `cmd+shift+w` | `workspace.close` | `ctrl+alt+shift+w` |
| `ctrl+1` | `workspace.show.1` | `alt+1` |
| `ctrl+2` | `workspace.show.2` | `alt+2` |
| `ctrl+3` | `workspace.show.3` | `alt+3` |
| `ctrl+4` | `workspace.show.4` | `alt+4` |
| `ctrl+5` | `workspace.show.5` | `alt+5` |
| `ctrl+6` | `workspace.show.6` | `alt+6` |
| `ctrl+7` | `workspace.show.7` | `alt+7` |
| `ctrl+8` | `workspace.show.8` | `alt+8` |
| `ctrl+9` | `workspace.show.9` | `alt+9` |
| `cmd+shift+arrowleft` | `focus.extend.left` | `ctrl+shift+arrowleft` |
| `cmd+shift+arrowright` | `focus.extend.right` | `ctrl+shift+arrowright` |
| `cmd+shift+arrowup` | `focus.extend.up` | `ctrl+shift+arrowup` |
| `cmd+shift+arrowdown` | `focus.extend.down` | `ctrl+shift+arrowdown` |
| `cmd+alt+arrowleft` | `focus.move.left` | `ctrl+alt+arrowleft` |
| `cmd+alt+arrowright` | `focus.move.right` | `ctrl+alt+arrowright` |
| `cmd+alt+arrowup` | `focus.move.up` | `ctrl+alt+arrowup` |
| `cmd+alt+arrowdown` | `focus.move.down` | `ctrl+alt+arrowdown` |
| `cmd+alt+shift+arrowleft` | `card.swap.left` | `ctrl+alt+shift+arrowleft` |
| `cmd+alt+shift+arrowright` | `card.swap.right` | `ctrl+alt+shift+arrowright` |
| `cmd+alt+shift+arrowup` | `card.swap.up` | `ctrl+alt+shift+arrowup` |
| `cmd+alt+shift+arrowdown` | `card.swap.down` | `ctrl+alt+shift+arrowdown` |
| `cmd+ctrl+alt+arrowleft` | `card.slide.left` | palette only |
| `cmd+ctrl+alt+arrowright` | `card.slide.right` | palette only |
| `cmd+ctrl+alt+arrowup` | `card.slide.up` | palette only |
| `cmd+ctrl+alt+arrowdown` | `card.slide.down` | palette only |
| `cmd+alt+j` | `focus.move.left` | `ctrl+alt+j` |
| `cmd+alt+k` | `focus.move.down` | `ctrl+alt+k` |
| `cmd+alt+l` | `focus.move.right` | `ctrl+alt+l` |
| `cmd+0` | `canvas.zoom.actual` | `ctrl+0` |
| `cmd+ctrl+shift+arrowleft` | `canvas.pan.left` | palette only |
| `cmd+ctrl+shift+arrowright` | `canvas.pan.right` | palette only |
| `cmd+ctrl+shift+arrowup` | `canvas.pan.up` | palette only |
| `cmd+ctrl+shift+arrowdown` | `canvas.pan.down` | palette only |
| `cmd+a` | `focus.selectAll` | `ctrl+shift+a` |
| `cmd+1` | `canvas.zoom.fitCard` | `ctrl+1` |
| `cmd+2` | `canvas.zoom.fitAll` | `ctrl+2` |
| `cmd+=` | `terminal.font.bigger` | `ctrl+=` |
| `cmd+-` | `terminal.font.smaller` | `ctrl+-` |
| `cmd+shift+=` | `ui.scale.up` | `ctrl+shift+=` |
| `cmd+shift+-` | `ui.scale.down` | `ctrl+shift+-` |
| `cmd+shift+0` | `ui.scale.reset` | `ctrl+shift+0` |
| `cmd+shift+enter` | `card.maximize.toggle` | `ctrl+shift+enter` |
| `cmd+shift+h` | `card.mask` | `ctrl+shift+h` |
| `cmd+l` | `card.omnibox` | `ctrl+shift+l` |
| `cmd+t` | `card.new.terminal` | `ctrl+shift+t` |
| `cmd+d` | `card.split.right` | `ctrl+shift+d` |
| `cmd+shift+d` | `card.split.down` | `ctrl+alt+shift+d` |
| `cmd+k` | `card.clear` | `ctrl+shift+k` |
| `cmd+alt+t` | `card.new.ungrouped` | `ctrl+alt+t` |
| `cmd+w` | `card.close` | `ctrl+shift+w` |
| `cmd+ctrl+w` | `card.close.leave` | `ctrl+alt+w` |
| `cmd+ctrl+enter` | `card.size.reset` | `ctrl+alt+enter` |
| `cmd+z` | `layout.undo` | `ctrl+shift+z` |
| `cmd+shift+z` | `layout.redo` | `ctrl+alt+shift+z` |
| `cmd+shift+l` | `card.protect` | `ctrl+alt+shift+l` |
| `cmd+alt+s` | `card.size` | `ctrl+alt+s` |
| `cmd+shift+t` | `card.reopen` | `ctrl+alt+shift+t` |
| `cmd+s` | `card.save` | `ctrl+shift+s` |
| `cmd+n` | `card.new.editor` | `ctrl+shift+n` |
| `cmd+b` | `editor.blame` | `ctrl+shift+b` |
| `cmd+i` | `card.transcript` | `ctrl+shift+i` |
| `cmd+escape` | `browser.leave` | `ctrl+escape` |
| `cmd+\` | `card.sidebar.flip` | `ctrl+shift+\` |
| `cmd+r` | `terminal.redraw` | `ctrl+shift+r` |
| `cmd+shift+r` | `card.rename` | `ctrl+alt+shift+r` |
| `cmd+,` | `app.settings` | `ctrl+,` |
| `cmd+shift+,` | `app.keybindings` | `ctrl+shift+,` |
| `cmd+shift+p` | `app.palette` | `ctrl+shift+p` |
| `cmd+ctrl+space` | `app.emoji` | palette only |
| `cmd+ctrl+f` | `app.fullscreen` | `f11` |
| `cmd+/` | `app.shortcuts` | `ctrl+shift+/` |
| `cmd+ctrl+shift+r` | `app.restart` | palette only |
| `cmd+ctrl+t` | `card.place` | `ctrl+alt+p` |
| `cmd+ctrl+s` | `snippet.paste` | `ctrl+alt+v` |
| `ctrl+tab` | `card.switcher.next` | `ctrl+tab` |
| `ctrl+shift+tab` | `card.switcher.prev` | `ctrl+shift+tab` |
| `cmd+f` | `card.find` | `ctrl+shift+f` |
| `cmd+e` | `card.findSelection` | `ctrl+shift+e` |
| `cmd+shift+c` | `terminal.visual` | `ctrl+alt+shift+c` |
| `cmd+j` | `focus.hint` | `ctrl+shift+j` |
| `cmd+g` | `group.new` | `ctrl+shift+g` |
| `cmd+shift+g` | `group.dissolve` | `ctrl+alt+shift+g` |
| `cmd+alt+r` | `group.rename` | `ctrl+alt+r` |
| `cmd+[` | `focus.prev` | `ctrl+shift+[` |
| `cmd+]` | `focus.next` | `ctrl+shift+]` |
| `cmd+alt+[` | `group.card.prev` | `ctrl+alt+[` |
| `cmd+alt+]` | `group.card.next` | `ctrl+alt+]` |
| `cmd+3` | `canvas.zoom.fitGroup` | `ctrl+3` |
| `cmd+c` | (body chord) | `ctrl+shift+c` |
| `cmd+v` | (body chord) | `ctrl+shift+v` |
| `cmd+x` | (body chord) | `ctrl+shift+x` |
| `cmd+shift+o` | (body chord) | `ctrl+shift+o` |
<!-- table:end -->
