#!/bin/sh
# A multiple selection is a set: every card in it wears the same blue ring
# at the same strength, so it is readable which cards the next command is
# about to act on. One card on its own is a focus, and keeps the white ring.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
cmd t; cmd t; cmd t
cmd 2; wait_s 1;                                  shot 01-one-focused 1
key_code 123 "command down, shift down";          shot 02-two 1
key_code 123 "command down, shift down";          shot 03-three 1
key_code 123 "command down, shift down";          shot 04-four 1
key_code 27 "command down"; key_code 27 "command down"
wait_s 1;                                         shot 05-zoomed-out 1
drive_log
drive_stop
