#!/bin/sh
# An idle canvas must not paint: launch, wait, and read what the [paint]
# line says kept the frames coming.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
wait_s 3
cmd 2
wait_s 3
shot 01-idle 0.1
# Two or three frames a second is the blink; anything more is a leak.
grep '\[paint\]' "$ROOT/run.log" | tail -4
drive_stop
