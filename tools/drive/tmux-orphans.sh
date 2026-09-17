#!/bin/sh
# Two things a unit test cannot show: a window this app left behind is
# killed at the next launch, and one somebody opened by hand is not.
. "$(dirname "$0")/lib.sh"
SESSION=infiniterm-dev
tmux kill-session -t "$SESSION" 2>/dev/null || true
FRESH=1
drive_start
cmd t; wait_s 1
cmd 2; wait_s 1
echo "--- two cards, two windows ---"
tmux list-windows -t "$SESSION" -F '   #{window_id} #{?@infiniterm,ours,theirs}' 2>&1 || true
# A window somebody opened by hand in the same session.
tmux neww -d -t "$SESSION" 2>/dev/null || true
echo "--- after adding one by hand ---"
tmux list-windows -t "$SESSION" -F '   #{window_id} #{?@infiniterm,ours,theirs}' 2>&1 || true
# The app dies WITHOUT quitting: its windows become orphans.
echo "--- killing the app outright (a crash) ---"
pkill -f "$ROOT/target/bundle/infiniterm.app/Contents/MacOS/infiniterm$" 2>/dev/null || true
wait_s 2
tmux list-windows -t "$SESSION" -F '   #{window_id} #{?@infiniterm,ours,theirs}' 2>&1 || true
echo "--- relaunching on an EMPTY canvas: our orphans go, theirs stays ---"
FRESH=1
drive_start
wait_s 3
tmux list-windows -t "$SESSION" -F '   #{window_id} #{?@infiniterm,ours,theirs}' 2>&1 || true
drive_stop
