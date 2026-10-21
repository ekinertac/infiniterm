#!/bin/sh
# First launch on an empty data dir and empty settings: the "Start here"
# card alone, then Cmd+T for the first terminal and Cmd+Alt+Left back to
# the card, the two steps its opening teaches (#178).
#   INFINITERM_DATA_DIR=/tmp/x CONFIG=/tmp/x-config tools/drive/welcome.sh
. "$(dirname "$0")/lib.sh"
rm -rf "$DATA" "${CONFIG:?set CONFIG to an empty settings dir}"
mkdir -p "$CONFIG"
FRESH=1 drive_start
wait_s 2
shot 1-first-launch
cmd t
shot 2-cmd-t 4
# The new terminal's first output can leave the window not key; make it
# key again so the chord reaches the app.
_activate
sleep 0.5
cmd_alt 123
shot 3-cmd-alt-left 4
drive_log
drive_stop
