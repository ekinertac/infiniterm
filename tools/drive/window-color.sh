#!/bin/sh
# window.color in the LOCAL instance (#160): the palette's two-step colour
# picker on a scratch instance with its own config, no server involved. Picks
# teal by typing its name, checks the key it saved to ui-themes/default.json
# (a look key, #511), then picks the "None" row and checks it is empty again. Keys go to the
# scratch instance through the driver (it refuses a foreign frontmost app).
. "$(dirname "$0")/lib.sh"
CONFIG=/tmp/infiniterm-drive/config
rm -rf "$CONFIG"; mkdir -p "$CONFIG"
printf '{\n}\n' > "$CONFIG/settings.json"
FRESH=1
drive_start
shot 01-plain 0.5
cmd_shift p; type_text "window: change the title bar"; key_code 36
wait_s 0.8
shot 02-picker 0.4
type_text "teal"
wait_s 0.5
shot 03-preview 0.4
key_code 36
wait_s 2
shot 04-teal 0.4
THEME="$CONFIG/ui-themes/default.json"
grep -q '^ *"ui.windowColor": "teal"' "$THEME" || { echo "FAIL: teal was not saved"; cat "$THEME"; drive_stop; exit 1; }
echo "saved: $(grep '^ *"ui.windowColor"' "$THEME")"
cmd_shift p; type_text "window: change the title bar"; key_code 36
wait_s 0.8
type_text "none"
key_code 36
wait_s 2
shot 05-none 0.4
grep -q '^ *"ui.windowColor": ""' "$THEME" || { echo "FAIL: none was not saved"; cat "$THEME"; drive_stop; exit 1; }
echo "saved: $(grep '^ *"ui.windowColor"' "$THEME")"
drive_log
drive_stop
echo PASS
