#!/bin/sh
# Far zoom: the terminal keeps its texture below the glyph cutoff and the
# centred name stays, however far out.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
type_text "ls -la ~/Code | head -30"; key_code 36
cmd t; type_text "fastfetch"; key_code 36; wait_s 1
cmd 2;                          shot 01-fit-all 1.0
cmd -; cmd -; cmd -; cmd -; cmd -; cmd -;  shot 02-far 0.8
drive_log
drive_stop
