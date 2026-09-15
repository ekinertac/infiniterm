#!/bin/sh
# The reference's stress numbers: 25 cards flooding, the fps in the status
# bar, then calm. Read the fps off 03 and 04.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
cmd_shift p; type_text "stress open"; key_code 36;   shot 01-25-cards 2.5
cmd 2;                                               shot 02-fit-all 1.0
cmd_shift p; type_text "run yes"; key_code 36;  shot 03-flooding 3.0
                                                     shot 04-still-flooding 2.0
cmd_shift p; type_text "stress stop"; key_code 36;   shot 05-calm 1.0
drive_log
drive_stop
