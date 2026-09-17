# tmux control mode: what it can actually do

Measured 2026-09-17 against tmux 3.7c on the Mac Mini, all of it on a private
server socket (`tmux -L infiniterm-spike`) so nothing could touch a real
session. The probes are in git history; this is what they said.

The question was whether tmux can back persistent cards without giving up the
things that make a card a card: our scrollback, our selection, our mouse, and
our backpressure. The answer is yes, and two of my three stated worries were
wrong.

## It is a protocol, not a screen

`tmux -C new-session` speaks control mode over plain pipes. No pty is needed,
which is worth knowing: the app can spawn it like any other child.

    %begin <time> <cmd> <flags> ... %end    a command's reply, framed
    %output %0 \033[m...\015\012            a pane's bytes, octal-escaped
    %window-add @1                          a window appeared
    %window-renamed @0 zsh                  automatic-rename fired
    %session-changed $0 spike

tmux draws nothing. The bytes arrive and our own emulator renders them, so
scrollback, selection and the mouse stay exactly what they are today. This is
what iTerm2 does, and it is why tmux feels normal there and strange when you
run it as a full-screen program inside a terminal.

Octal escapes have to be unescaped: `\033` and `\015\012` are three- and
six-character sequences in the text, not bytes.

## Each card can be its own size

    refresh-client -C '@0:100x30'
    refresh-client -C '@1:60x20'

Two windows, two sizes, one client. The shells agree: `tput cols; tput lines`
answered 100x29 and 60x19. The missing row is tmux's status line, so
`set -g status off` is not cosmetic, it is a row of the card.

This settles the mapping: **a card is a tmux WINDOW with one pane**, never a
pane inside a shared window. Panes tile inside a window and would have to
share its size.

## Backpressure exists, and I was wrong about it

I said control mode had no per-pane flow control. It does, since tmux 3.2:

    refresh-client -A '%0:pause'      and 'continue', 'on', 'off'

Measured with `yes` flooding a pane: 18,725 output lines per second, then
exactly **0 lines in 1.5 seconds** while paused, then 15,555/s after continue.
tmux emits `%pause %0` when it happens. `off` goes further and stops tmux
reading the pane at all once no client wants it.

`pause` is the WRONG verb for it, which cost an evening to learn. A paused
pane keeps running and tmux DISCARDS what it produces for that client:
measured, output made while paused arrives neither during the pause nor
after `continue`, while tmux's own grid has it all. Backpressure built on
`pause` silently loses bytes, and a card's grid then stops matching the
program's; the symptom is a redraw landing in the wrong place, which looks
like a rendering bug and is not.

    refresh-client -A '%0:off'    and 'on'

`off` is the one. tmux stops READING the pane once no client wants it, so
the program blocks instead of producing output nobody receives, and
everything arrives when it is turned back on. Measured both ways. That is
the same mechanism the local backend uses: a reader that stops reading
stalls the child at the kernel's pty buffer.

## Sessions survive, and the history comes back

The control client was killed outright (the app quitting, or crashing). The
session stayed, and a `sleep 300 &` started inside it was still running.

    capture-pane -p -S - -t %0

returns the pane's whole history as text, which is what seeds a restored
card's grid. `PaneEvent::Replay` exists unused in `backend/mod.rs` for exactly
this and needs no new code path.

## The trap that cost the afternoon

`capture-pane -t spike:0` returned nothing, repeatedly, and looked like "the
history is gone". The window was `probe:1.1`: this machine has `base-index 1`,
so window 0 does not exist.

**Address tmux by id, never by index or name.** `%0` for a pane, `@0` for a
window, `$0` for a session. Indexes and names are the user's configuration and
are not ours to assume, the same discipline that makes the driver address the
app by pid.
