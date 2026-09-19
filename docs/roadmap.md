# Where this stands

The list Ekin asks about. Dates are when something landed. The rule for this file: update it when something ships or gets parked, never let it describe last week.

## Done

- v1, 2026-09-14: terminal, editor, diff, transcript cards on the canvas; groups, workspaces, selection, phantom slots, agent state from hooks, palette, themes, `ift`.
- The Rust port replaced the Tauri app, 2026-09-16. CEF browser cards were the reason. The Tauri repo is archived at `ekinertac/infiniterm-tauri`.
- Omnibox (Cmd+L), 2026-09-16: address or search, tab-to-search, our own frecency history. Back, forward, reload, copy address, find in page, reopen a closed card.
- File drops from the Finder, three agent states with three hues, selection rings, agent state log.
- Persistent sessions, 2026-09-17: our own daemon, `iftd`, one per card. tmux was tried first, was the default for one evening, and was pulled (docs/tmux-handover.md). Claude Code renders clean through a quit and a relaunch. `ift sessions` and `ift attach`. Design: docs/superpowers/specs/2026-09-17-session-daemon-design.md.
- Cmd+Ctrl+Alt+R restarts the app in place, same window, same sessions.
- 2026-09-18, also: the emoji panel and dead keys (ime.rs, and `app.emoji` as our own chord), a workspace that remembers its focused card, a click that reveals a clipped card, double-click on a frame or label to fit, and the label as a drag handle.
- 2026-09-18, the first day living in it: Shift+Enter in Claude Code and Pi (the kitty keyboard handshake, and the daemon keeps a session's opening bytes so it survives a relaunch), Turkish Option characters, Option+Delete, the swap chord into phantom slots, one card label instead of two, menus that keep their order, the Pi cursor, a replay that lands in one frame, a focus ring you can see, a maximized card that reflows, an editor crash on scroll, text fields with a real caret and the macOS chord set, the canvas dimming when another app is in front, and cards and workspaces as palette rows.
- 2026-09-19: zoom on a dense canvas. Each terminal row is shaped once and its glyphs painted at their cells (`paint_row`), instead of 24-cell ASCII chunks plus one `shape_line` per icon or emoji; Ekin: "FPS is much better" on the 22-card canvas (not re-measured with `[paint]`; the day-before numbers were 18 to 30 ms a frame while zooming, shaping half of it). The editor, diff and transcript bodies still chunk and could take the same treatment if they ever show up in the log. Also: Cmd+Ctrl+W closes a card leaving its space, a phantom is the size of the card beside it, directional focus prefers the same row, every card wears a number, double-click on bare canvas is Cmd+2.
- 2026-09-18, evening, after a power cut brought 23 cards back as bare prompts: `iftd` writes its ring to `<data>/s/<id>.ring` every two seconds while output arrives (tmp, fsync, rename), a card whose session is dead replays it above the new shell with a `[session lost <time>]` line and a mode reset, and a card that ran Claude gets `claude --resume <id>` printed and appended to its own history file, so Up and Enter bring the agent back. One zsh history per card (`INFINITERM_HISTFILE`, taken by a line in `.zshrc`). Also that day: pictures in the editor card and the tree previewing them on arrow, close returning to the previously focused card, `card.mask` (Cmd+Shift+H, a decoy over the card), Developer ID signing and `make notarize`, flat dotted settings keys. Verified by SIGKILLing the app and every daemon together, three times.
- Browser tabs and focus lock, 2026-09-19: a card can hold several tabs, one CEF surface each, with a tab strip painted over the page; a popup, a `target=_blank` link, a Cmd+click or "Open link in new tab" opens a tab instead of a new card. The keyboard locks to the page on the first interaction past a focusing click, the same as alt-tabbing into a real Chrome window: Cmd+T/W/Shift+T, Cmd+1..9, Ctrl+Tab all become Chrome's bindings instead of this app's, double-Escape unlocks, and Cmd+L / Cmd+Esc / Ctrl+1..9 stay the app's regardless. Design: docs/superpowers/specs/2026-09-19-browser-tabs-design.md.

## v1.5: the other agents

Hook adapters so a card running something other than Claude Code gets the same colours. Pi is done (`ift install-pi-hooks`). OpenCode and Codex remain, in that order. The MCP server was dropped: hooks already let an agent drive its own card.

## v2: what survives what

The daemon keeps a shell alive across the app quitting. Nothing keeps a process alive across a reboot, and Ekin does not want that; he wanted the scrollback, and since 2026-09-18 he has it (see Done).

- **Composing text is not drawn.** Dead keys and input methods work (ime.rs), but the `\u{b4}` before the `e` is held and not shown, and an input method's candidate window sits at the caret only for terminals. Drawing the marked text at the caret, and caret bounds for the fields, is the rest of it.
- **Ring compaction**, only if 4 MiB proves shallow for a Claude card. Ink repaints are large and repetitive. The upgrade is to parse what is about to fall out of the ring into a headless grid with our own parser and serialise it back as the new front. Named in the daemon spec, deliberately not built.

## Open from the daemon work

- `tools/drive/daemon.sh` ran once and passed; its screenshots 01 to 04 did not write (`winid --pid` found nothing for those instances) and only 05 did. Not chased.
- What N daemons cost in practice. 13 were alive at once on the real canvas; the spec guessed ~2 MB each plus the ring. Never measured.
- Colour reporting under tmux, Nerd Font glyph widths, a very long scrollback on adopt: the tmux handover's open items 2, 3 and 6, still true of tmux, mostly moot under the daemon.

## Dropped

- Inline images in a terminal card (imgcat, kitty graphics, sixel). Dropped 2026-09-18: Cmd+click on an image path opens it in an editor card, and the tree walks a folder of screenshots with the arrows, which is what the images were wanted for. Rendering inside the grid would have been two days per protocol for the same picture in a smaller box.

## Parked

- The screencast: `tools/drive/cast.sh` exists and needs a shakedown run. Parked until the app has been lived in for a while.
- Bookmarks in the omnibox, deferred at design time.
