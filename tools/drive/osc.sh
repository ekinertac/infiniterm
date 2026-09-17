#!/bin/sh
# How long the terminal takes to answer a colour query, which is what a
# program like bat or Claude Code waits on before drawing. Under the tmux
# backend the answer was arriving after the program had exited, and landing
# in the shell as typed text.
. "$(dirname "$0")/lib.sh"
TAG=${TAG:-run}
tmux kill-session -t infiniterm-dev 2>/dev/null || true
FRESH=1
drive_start
cmd 0; wait_s 1
type_text "bash /tmp/osctest.sh"
key_code 36; wait_s 3
shot "$TAG-osc" 1.5
drive_log
drive_stop
