#!/bin/sh
# Dragging a card by its top edge: alignment guides while it moves, a red
# outline while it overlaps, and a drop on another card put back. Positions
# assume fit-all of Ekin's canvas in the 1600x1000 window: the grid's
# top-left card spans 499..617 x 513..652 (shadowless shot pixels are window
# pixels; the title bar is the first 44).
. "$(dirname "$0")/lib.sh"
drive_start
cmd 2;                                              shot 01-fit-all
cliclick "dd:$((WX+555)),$((WY+516))" "m:$((WX+600)),$((WY+400))" "m:$((WX+700)),$((WY+200))"
sleep 0.2;                                          shot 02-mid-drag 0.05
cliclick "du:$((WX+700)),$((WY+200))"; sleep 0.3;   shot 03-dropped
cliclick "dd:$((WX+700)),$((WY+202))" "m:$((WX+690)),$((WY+400))" "m:$((WX+680)),$((WY+560))"
sleep 0.2;                                          shot 04-over-another 0.05
cliclick "du:$((WX+680)),$((WY+560))"; sleep 0.3;   shot 05-put-back
drive_log
drive_stop
