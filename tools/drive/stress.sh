#!/bin/sh
# The reference's stress numbers: 25 cards flooding (the fps in the status
# bar of 03 and 04), calm, then ten alternating fits with the fps logged
# per step (`stress zoom` lines in the log), then the 10000-line burst.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
cmd_shift p; type_text "stress open"; key_code 36;   shot 01-25-cards 2.5
cmd 2;                                               shot 02-fit-all 1.0
cmd_shift p; type_text "run yes"; key_code 36;  shot 03-flooding 3.0
                                                     shot 04-still-flooding 2.0
cmd_shift p; type_text "stress stop"; key_code 36;   shot 05-calm 1.0
cmd_shift p; type_text "stress alternate"; key_code 36; shot 06-zooming 3.0
wait_s 4
cmd_shift p; type_text "stress 10000"; key_code 36;  shot 07-lines 2.0
drive_log
grep 'stress zoom\|burst' "$ROOT/run.log" | sed 's/\[infiniterm\] //'
# Where the flood's frames went: feed, frame build, links, shaping, glyphs.
grep '\[paint\]' "$ROOT/run.log" | sed -n '4,6p' 
drive_stop
