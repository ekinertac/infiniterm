#!/bin/sh
# The phantom and slot picking: an arrow into empty space shows a hollow
# card, Enter asks what goes in it, the placement menu letters every slot.
# Then a Cmd+drag pan. Starts on an EMPTY canvas so the positions are known:
# the first card at the origin, fit to the view by Cmd+2.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
cmd 2;                       shot 01-first-card
cmd_alt 124;                 shot 02-phantom-right
key_code 36;                 shot 03-slot-menu
key_code 36;                 shot 04-filled
cmd 2
cmd_alt 125;                 shot 05-phantom-below
key_code 124 "command down, shift down"; shot 06-phantom-extended
key_code 53;                 shot 07-escaped
key t "command down, control down"; shot 08-placement-menu
key_code 36;                 shot 09-slot-picking
type_text s;                 shot 10-slot-s-filled
cmd 2;                       shot 11-fit
cmd_drag 300 500 500 300;    shot 12-panned 0.6
drive_log
drive_stop
