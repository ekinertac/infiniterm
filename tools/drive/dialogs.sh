#!/bin/sh
# The three dialogs: the rename prompt, the workspace-close confirm, and
# the interface multiplier reaching them. A second workspace first, since
# the last one cannot be closed.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
cmd_shift r;                     shot 01-prompt 0.5
key_code 53
cmd_shift p; type_text "workspace: new"; key_code 36; wait_s 0.5
cmd_shift p; type_text "workspace: close"; key_code 36
                                 shot 02-confirm 0.6
key_code 53
# Physical Equal with Cmd+Shift twice: interface at 1.2x then 1.44x.
key_code 24 "command down, shift down"; key_code 24 "command down, shift down"
cmd_shift p; type_text "workspace: close"; key_code 36
                                 shot 03-confirm-scaled 0.6
key_code 53
key_code 29 "command down, shift down"
drive_log
drive_stop
