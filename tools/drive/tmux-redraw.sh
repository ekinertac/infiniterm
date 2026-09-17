#!/bin/sh
# A program that redraws in place, across a restart. This is the shape of
# the Claude Code corruption without spending anybody's tokens: `top`
# repaints its whole screen on a timer, and the repaint begins with the
# clear that was being dropped while we did not yet know whose pane it was.
. "$(dirname "$0")/lib.sh"
SESSION=infiniterm-dev
tmux kill-session -t "$SESSION" 2>/dev/null || true
FRESH=1
drive_start
cmd 0; wait_s 1
type_text "top -o cpu"; key_code 36; wait_s 4
shot 01-before-restart 1.5
echo "--- quitting; the program keeps running ---"
quit
wait_s 3
echo "--- relaunching: this card is ADOPTED ---"
FRESH=
KEEP=1
drive_start
wait_s 5
cmd 0; wait_s 2
shot 02-adopted 2
drive_log
drive_stop
