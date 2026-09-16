#!/bin/sh
# The whole point of the tmux backend, end to end: a card's shell survives
# the app quitting and comes back with what it printed.
#
# Runs on the scratch data dir, which means the `infiniterm-dev` tmux
# session: a run here cannot see, resize or kill the windows the real app
# is holding.
. "$(dirname "$0")/lib.sh"
SESSION=infiniterm-dev
# A session left by an earlier run. Absent is the normal case, and
# lib.sh runs under `set -e`, so a failure here must not end the script.
tmux kill-session -t "$SESSION" 2>/dev/null || true
FRESH=1
drive_start
type_text "echo marker-from-the-first-launch"; key_code 36; wait_s 1
cmd t; type_text "sleep 600"; key_code 36; wait_s 1
cmd 2; wait_s 1;                      shot 01-two-cards 1
echo "--- tmux windows while the app is up ---"
tmux list-windows -t "$SESSION" -F '   #{window_id} #{pane_current_command}' 2>&1 || true
quit
wait_s 2
echo "--- after Cmd+Q: the session must still be there ---"
tmux list-windows -t "$SESSION" -F '   #{window_id} #{pane_current_command}' 2>&1 || true
echo "--- and the long job must still be running ---"
pgrep -fl "sleep 600" | head -2 || echo "   the sleep is GONE"
echo "--- relaunching: the cards should adopt those windows ---"
FRESH=
KEEP=1
drive_start
wait_s 3
cmd 2; wait_s 1;                      shot 02-after-relaunch 1.5
tmux list-windows -t "$SESSION" -F '   #{window_id} #{pane_current_command}' 2>&1 || true
drive_log
drive_stop
