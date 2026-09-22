#!/bin/sh
# THE FALSIFICATION TEST for the whole session-daemon design (see
# docs/superpowers/specs/2026-09-17-session-daemon-design.md, "Risks" #1,
# and docs/superpowers/plans/2026-09-17-session-daemon.md, Task 7 Step 2).
#
# Every other part of that plan has unit and integration tests. This one
# cannot: whether Claude Code's inline redraw survives a detach and a
# reattach through our own ring replay is a question about pixels, and the
# only way to answer it is to run Claude Code in a card, quit, relaunch,
# and look.
#
# If Claude comes back here with two lines merged into one row, the ring
# replay and the post-replay resize-nudge (infiniterm-core/src/backend/
# daemon.rs, right after ReplayEnd) are NOT the cure, and the fault is in
# our own cell widths or VT parser, not in tmux and not in the daemon.
# Do not flip terminal.backend's default (config.rs) until this has been
# run and passed on screen. See docs/tmux-handover.md for the bug this
# design exists to route around.
#
# Runs on a scratch DATA dir and a scratch CONFIG dir, so it can turn the
# daemon backend on without touching Ekin's real settings.json, which
# stays "pty" until this passes.
. "$(dirname "$0")/lib.sh"
CONFIG=/tmp/infiniterm-drive/config
mkdir -p "$CONFIG"
cat > "$CONFIG/settings.json" <<'JSON'
{ "terminal": { "backend": "daemon" } }
JSON
IFT="$ROOT/target/debug/ift"
SESSIONS="$DATA/s"

FRESH=1
drive_start
# A fresh canvas seeds its card and spawns the shell AFTER the window is up,
# and keys typed before the shell exists go nowhere: on 2026-09-23 the
# redraw command below was lost that way while the later echo arrived.
# Wait for the card's session socket, then a beat for zsh's own startup.
for _ in $(seq 1 30); do
    ls "$SESSIONS"/*.sock >/dev/null 2>&1 && break
    sleep 0.5
done
wait_s 2

echo "--- part 1: a synthetic inline redraw, no Claude needed ---"
# Mimics Ink's own update style (and Claude Code's): print two lines, then
# repeatedly move the cursor up and overwrite them in place rather than
# clearing the screen. Any point where the emulator loses track of the
# cursor shows as both lines sharing one row, which is the exact failure
# mode this whole design is about.
# Written to a file and RUN, not typed: `type_text` interpolates into an
# AppleScript string literal, so a double quote in the payload closes it
# and osascript dies with "Expected \" but found unknown token".
cat > "$DATA/redraw.sh" <<'SH'
printf 'top\nbottom\n'
i=1
while [ $i -le 50 ]; do
  sleep 0.3
  printf '\033[2A\033[Ktop %s\n\033[Kbottom %s\n' $i $i
  i=$((i + 1))
done
SH
type_text "sh $DATA/redraw.sh"
key_code 36
wait_s 2
                                shot 01-redrawing 1
echo "--- sessions on disk while the app is up ---"
ls -la "$SESSIONS" 2>&1 || echo "   no sessions dir yet"
"$IFT" sessions 2>&1 || true
quit
wait_s 1
echo "--- after Cmd+Q: the daemon and its child must still be running ---"
ls -la "$SESSIONS" 2>&1 || echo "   sessions dir is gone"
# Scoped to this run's data dir: a bare "iftd --socket" also lists every
# daemon of the real canvas, which proves nothing about this one.
pgrep -fl "iftd --socket $SESSIONS/" || echo "   iftd is GONE"
"$IFT" sessions 2>&1 || echo "   ift sessions failed with the app dead: that is the bug, it must not need the app"

echo "--- relaunching: the card should adopt that session, redraw and all ---"
FRESH=
KEEP=1
drive_start
wait_s 2
                                shot 02-adopted 1.5
echo "look at 01 vs 02 by eye: the redraw must still be one clean pair of lines, not garbled."
key c "control down"; wait_s 0.5
type_text "echo still-alive-after-relaunch"
key_code 36
wait_s 1
                                shot 03-live-prompt 1
echo "check 03: the echo above must show a real prompt still taking input, which is the point of adopting rather than starting a fresh shell."
drive_log

echo
echo "--- part 2: the real falsification test, Claude Code itself ---"
cmd t; wait_s 1
type_text "claude"
key_code 36
wait_s 5
                                shot 04-claude-drawing 2
echo "let it think/draw for a bit before quitting; a blank or half-drawn screen here is not a fair test."
quit
wait_s 1
FRESH=
KEEP=1
drive_start
wait_s 3
                                shot 05-claude-after-relaunch 2
echo "THE CHECK: compare 04 and 05. Two lines sharing one row in 05 means the falsification fired: stop, report, do not flip the default."
drive_log
drive_stop
