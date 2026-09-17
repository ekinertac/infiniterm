# tmux backend: where it stands

Written 2026-09-17, mid-work, so the next session does not have to rediscover any of this.

## The way back

    before-tmux    d77bb8e    the last commit before any tmux work

Eleven commits sit on top of it, all on master, none pushed (28 unpushed in total, the rest are the omnibox and browser work from earlier the same day).

Three ways out, cheapest first:

1. `terminal.backend: "pty"` in `~/.config/infiniterm/settings.json`. Takes effect on the next launch, no rebuild, and every card is a plain local shell again. The tmux code stays where it is.
2. `git revert` the eleven commits, keeping the history.
3. `git reset --hard before-tmux`, discarding them.

The tmux session itself survives all three: `tmux kill-session -t infiniterm` ends it.

## What is built

tmux is the DEFAULT backend. One `tmux -C` control client on tmux's usual socket, attached to one session named `infiniterm`, and a card is a tmux WINDOW holding one pane.

- Shells outlive the app. Quitting detaches; relaunching adopts the same windows and replays their scrollback.
- The card's window id is in the save file (`tmuxWindow`), written only for cards that have one, so a canvas that never used tmux still round-trips byte for byte.
- Windows this app made and no card claims are killed at the next launch. They are tagged with a tmux user option (`@infiniterm`), so a window opened by hand with `tmux neww -t infiniterm` is never touched.
- `tmux attach -t infiniterm` from any terminal reaches the same sessions. That is the whole argument for tmux over a daemon of our own.
- A machine with no tmux falls back to local PTYs and says so once.
- A scratch instance (`INFINITERM_DATA_DIR` set, so `make run` and every driver scenario) uses a different session name, `infiniterm-dev`, and cannot touch the real one.

Files: `infiniterm-core/src/backend/tmux.rs` (the client), `tmux_protocol.rs` (the reading, pure and tested without a tmux), `backend/mod.rs` (`Panes`, the enum that picks). Scenarios: `tools/drive/tmux.sh`, `tmux-orphans.sh`, `tmux-flood.sh`, `tmux-tui.sh`. Measurements: `spikes/tmux/NOTES.md`.

## Bugs found and fixed, with what gave them away

Every one of these was found by running it, not by reading it. The unit tests passed throughout.

1. **A card claimed a pane it did not own.** The first `%output` after attaching comes from the session's own initial window, and a card waiting for a pane took it. Everything typed into that card went to a window no card owned, and the card came back from a relaunch with an empty history. Fixed by asking for both ids at once: `new-window -P -F '#{window_id} #{pane_id}'`. Nothing about a pane is guessed now.
2. **tmux replies to the `new-session` on its own command line.** A reply block nothing queued, so every answer after it was off by one and window ids landed in the reply block of an earlier command. It has a seat in the queue now. This is why the first attempt at fix 1 appeared to do nothing.
3. **A `;`-separated command list is several commands to tmux and produces several reply blocks.** Measured: three commands, three blocks. Everything goes one command per line.
4. **The tag went to the wrong window.** `set-option -w -t <session>` targets the session's CURRENT window, and `new-window -d` does not change that. Tagged by id now, at the moment tmux reports the id.
5. **The card's size never reached tmux.** The ui asks the instant it spawns, which is before tmux has named the window, so the request was dropped and the pane kept the 80x24 it was created with while the card drew a much bigger grid. Measured: window 80x24 and `tput` answering `cols=80 rows=23` in a card 1725 world pixels wide. The size is remembered and applied when tmux answers. This produced the interleaved-character corruption in full-screen programs.
6. **`pane-border-status top` steals a row.** It is in Ekin's `~/.tmux.conf`, and a pane border is a row of the card: every shell was one row shorter than the grid drawn for it. Turned off for our session.
7. **Replaying tmux's padding.** `capture-pane` flattens the grid and pads every line to the pane width using TMUX's character widths. A line holding an emoji that our emulator measures one cell differently overflows, wraps where tmux did not, and every line after it lands a row out. Trailing blanks are padding, never content, so they are stripped. Live output was never affected, which is why a card healed itself as soon as the program printed something new.
8. **Backpressure was losing output.** `refresh-client -A '%N:pause'` does not hold output back: the pane keeps running and tmux DISCARDS what it produces for that client. Measured, output made while paused arrives neither during the pause nor after `continue`, while tmux's own grid has all of it. Any card past the 256 KiB unacknowledged mark lost bytes for good, and its grid stopped matching the program's, which shows up as a redraw landing in the wrong place. `off` is the verb: tmux stops READING the pane, the program blocks, and everything arrives on `on`. Measured both ways.

9. **We answered terminal queries that were not ours to answer.** A program asks the terminal for its background colour, its device attributes, the cursor position. Under tmux, tmux IS the terminal for that pane and answers itself; our second answer arrived afterwards and was delivered to the program as keystrokes. Measured: the reply visible in the pane as `10;rgb:5050/9e9e/3131` at a prompt, echoed, after the program that asked had exited, and a probe seeing the answer both echoed and read. Claude Code re-queries as it redraws, so it took a steady drip of that into its input, which is what "claude is a mess" was. Emulator answers are now kept apart from typed input (`TerminalBody::replies`) and sent only when we are the terminal.

10. **Output was dropped for a pane we had not matched to a card yet.** Adoption asks tmux which pane is in the window, and a program does not wait for that answer: tmux makes a pane redraw the moment a client attaches, so the redraw arrives before we know whose it is, and it was being thrown away. The clear-screen at the front of that redraw went with it, and the program then painted over a screen we had never cleared, which is how two lines end up in one row character by character. Held now (`early`, capped at 1 MiB a pane so an unclaimed window cannot cost more) and delivered in order once the pane is known, after any replayed history. Verified with `top` across a restart: it comes back clean.

## Open, in the order I would take them

1. **Colour reporting under tmux.** Programs now get tmux's answer rather than ours, so a program that adapts to the terminal's theme sees tmux's idea of it, not the card's. Nothing looked wrong in testing, but a light theme would be the case to check.
2. **Nerd Font glyph widths.** Suspected, not proven. Those glyphs live in Unicode's private use area, where the width tables say one cell and the font draws two. If stray characters still appear beside `📁` or a powerline glyph in FRESH output, this is the cause, and it would affect the local backend equally. Nothing to do with tmux.
3. **A window that dies while the app is closed.** `live_windows` is asked once at startup and a card adopts from that list. A window that dies between the ask and the adopt never reports output; the card sits there with no shell rather than spawning one. Not seen, but it is a hole.
4. **Ekin's `~/.tmux.conf` generally.** We override `status`, `allow-rename` and `pane-border-status` for our session. Anything else in there applies and is a candidate whenever something looks wrong: `default-terminal` is `tmux-256color` (the terminfo is present on this machine), `mouse on`, and 5.5 KB besides.
5. **A very long scrollback on adopt.** `capture-pane -S -` replays the whole history at once. Tested with a few hundred lines, not with a hundred thousand.

## Traps, for whoever is next

- **Address tmux by id, never by index or name.** `%0` a pane, `@0` a window, `$0` a session. An index is somebody's `base-index`: this machine starts windows at 1, and `capture-pane -t session:0` silently matches nothing. That cost an afternoon.
- **One command per line.** See bug 3.
- **Every command must push exactly one expectation.** The queue in `tmux.rs` is the only thing tying a reply to the command that asked for it, because tmux answers in order and carries no correlation id we can predict. `send()` pushes while holding the stdin lock, so two threads cannot interleave a push and a write.
- **`off`, never `pause`.** See bug 8.
- **A driver scenario must never touch the real session.** `lib.sh` exports `INFINITERM_DATA_DIR`, which is what makes `session_name()` answer `infiniterm-dev`.

## The honest summary

It works: shells survive a quit, a card comes back to the one it had with its scrollback, a flooding card no longer starves the others, and crashed-app litter is swept. It has also produced eight bugs in one day, seven of them mine and one inherent to replaying another emulator's grid. Ekin's hesitation about tmux was well founded in the sense that matters: it inserts a second terminal's worth of state and a config file we do not control between the card and the shell, and every one of those is a new place to be wrong.
