# Where this stands

The list Ekin asks about. Dates are when something landed. The rule for this file: update it when something ships or gets parked, never let it describe last week.

## Done

- v1, 2026-09-14: terminal, editor, diff, transcript cards on the canvas; groups, workspaces, selection, phantom slots, agent state from hooks, palette, themes, `ift`.
- The Rust port replaced the Tauri app, 2026-09-16. CEF browser cards were the reason. The Tauri repo is archived at `ekinertac/infiniterm-tauri`.
- Omnibox (Cmd+L), 2026-09-16: address or search, tab-to-search, our own frecency history. Back, forward, reload, copy address, find in page, reopen a closed card.
- File drops from the Finder, three agent states with three hues, selection rings, agent state log.
- Persistent sessions, 2026-09-17: our own daemon, `iftd`, one per card. tmux was tried first, was the default for one evening, and was pulled (docs/tmux-handover.md). Claude Code renders clean through a quit and a relaunch. `ift sessions` and `ift attach`. Design: docs/superpowers/specs/2026-09-17-session-daemon-design.md.
- Cmd+Ctrl+Alt+R restarts the app in place, same window, same sessions.
- 2026-09-18, the first day living in it: Shift+Enter in Claude Code and Pi (the kitty keyboard handshake, and the daemon keeps a session's opening bytes so it survives a relaunch), Turkish Option characters, Option+Delete, the swap chord into phantom slots, one card label instead of two, menus that keep their order, the Pi cursor, a replay that lands in one frame, a focus ring you can see, a maximized card that reflows, an editor crash on scroll, text fields with a real caret and the macOS chord set, the canvas dimming when another app is in front, and cards and workspaces as palette rows.

## v1.5: the other agents

Hook adapters so a card running something other than Claude Code gets the same colours. Pi is done (`ift install-pi-hooks`). OpenCode and Codex remain, in that order. The MCP server was dropped: hooks already let an agent drive its own card.

## v2: what survives what

The daemon keeps a shell alive across the app quitting. Nothing keeps a process alive across a reboot, and Ekin does not want that; he wants the scrollback.

- **Scrollback across a reboot.** `iftd` writes its ring to `<data>/s/<id>.ring` on SIGTERM (macOS sends it at shutdown) and every 30 s while dirty. After a reboot no socket answers, the card spawns a fresh daemon as it does today, and a daemon that finds a `.ring` for its id preloads it and sends it as the first replay, followed by `\e[?1049l\e[0m` so a shell that died inside vim does not leave the new prompt in the alternate screen. `Kill` deletes the file. What you see: the old scrollback, then a fresh prompt under it. Cost: 4 MiB a card on disk at worst, and terminal history on disk in plain bytes, which is a new place a secret can land. About half a day.
- **`claude --resume` on restore.** The card already knows its transcript path from the hooks. After a reboot, a card that had a Claude session gets the old scrollback from the item above and the session resumed below it. Parked on 2026-09-16 because the daemon was not built yet; the daemon makes it worth doing. Open question from then: run it, or leave it typed for Enter.
- **Inline images in a terminal card.** Nothing handles iTerm2's `OSC 1337 File=` (imgcat), the kitty graphics protocol, or sixel today. It means intercepting the sequence in the output path, decoding the image, keeping it beside the grid rows it occupies, painting it as a texture at those cells, and making it scroll and survive a replay from the ring. One protocol done properly is about two days; iTerm2's is the one Ekin would use, kitty's is what newer tools emit.
- **Ring compaction**, only if 4 MiB proves shallow for a Claude card. Ink repaints are large and repetitive. The upgrade is to parse what is about to fall out of the ring into a headless grid with our own parser and serialise it back as the new front. Named in the daemon spec, deliberately not built.

## Open from the daemon work

- `tools/drive/daemon.sh` ran once and passed; its screenshots 01 to 04 did not write (`winid --pid` found nothing for those instances) and only 05 did. Not chased.
- What N daemons cost in practice. 13 were alive at once on the real canvas; the spec guessed ~2 MB each plus the ring. Never measured.
- Colour reporting under tmux, Nerd Font glyph widths, a very long scrollback on adopt: the tmux handover's open items 2, 3 and 6, still true of tmux, mostly moot under the daemon.

## Parked

- The screencast: `tools/drive/cast.sh` exists and needs a shakedown run. Parked until the app has been lived in for a while.
- Bookmarks in the omnibox, deferred at design time.
