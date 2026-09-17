#!/bin/sh
# One card floods; the others must stay responsive. Under the local backend
# a flooding pane stalls at its own kernel buffer. Under tmux everything
# shares one socket, so the ledger has to ask tmux to pause the pane or one
# screaming card delays every other.
. "$(dirname "$0")/lib.sh"
SESSION=infiniterm-dev
tmux kill-session -t "$SESSION" 2>/dev/null || true
FRESH=1
drive_start
type_text "yes flooding-this-card"; key_code 36; wait_s 2
cmd t; wait_s 1
cmd 2; wait_s 1;                       shot 01-one-flooding 1
echo "--- typing into the OTHER card while the first floods ---"
type_text "echo the-quiet-card-still-answers"; key_code 36; wait_s 2
shot 02-quiet-card 1
tmux capture-pane -p -t "$SESSION":.+ 2>/dev/null | grep -c "the-quiet-card-still-answers" || true
drive_log
drive_stop
