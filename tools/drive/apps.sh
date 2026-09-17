#!/bin/sh
# The same programs under whichever backend CONFIG selects, so the two can
# be compared frame for frame. bat, less and vim: bat shows nothing at all
# under tmux, and vim is reported fine, which is the interesting split.
. "$(dirname "$0")/lib.sh"
TAG=${TAG:-run}
tmux kill-session -t infiniterm-dev 2>/dev/null || true
FRESH=1
drive_start
# Actual size: a whole card only fits the window at about 42%, where the
# text is too small to judge. At 100% the top-left of the card is where the
# output is anyway.
cmd 0; wait_s 1
type_text "echo cols=\$(tput cols) rows=\$(tput lines) TERM=\$TERM"; key_code 36; wait_s 1.5
shot "$TAG-01-size" 1
type_text "bat --version; bat /etc/hosts"; key_code 36; wait_s 3
shot "$TAG-02-bat" 1.5
type_text "q"; wait_s 1
key_code 53
type_text "clear; head -40 /etc/services"; key_code 36; wait_s 2
shot "$TAG-03-plain" 1.5
drive_log
drive_stop
