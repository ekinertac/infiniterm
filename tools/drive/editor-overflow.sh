#!/bin/sh
# #197: a long line scrolled right must be cut at the text area's edge, not
# drawn over the file tree. Opens a file with one 600-character line, shows
# the tree (palette), puts the caret at the end of the line (End) and looks.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
mkdir -p /tmp/infiniterm-drive
awk 'BEGIN { printf "// "; for (i = 0; i < 60; i++) printf "word%02d-----", i; printf "\n" }' > /tmp/infiniterm-drive/long.rs
"$ROOT/target/bundle/infiniterm.app/Contents/MacOS/ift" /tmp/infiniterm-drive/long.rs >/dev/null
                                shot 01-opened 1.0
# The tree first: a locked editor swallows the palette's keys. Then a click
# into the text locks the card and puts the caret in it.
cmd_shift p; type_text "show or hide the file tree"; key_code 36
                                shot 02-tree 0.8
click 900 134;                  shot 03-clicked-into-text 0.5
key_code 119;                   shot 04-end-of-line 0.6
drive_log
drive_stop
