#!/bin/sh
# #213: typing "ui. in settings.json opens a popup of the ui.* settings, with
# defaults and a description; Down moves, Tab inserts the key. Then a value
# position, `"ui.fullscreen": "`, offers its choices. Writes a small
# settings.json into the scratch instance's own config folder and opens it.
. "$(dirname "$0")/lib.sh"
FRESH=1
mkdir -p "$DATA/config"
printf '{\n  \n  "ui.fullscreen": \n}\n' > "$DATA/config/settings.json"
drive_start
"$ROOT/target/bundle/infiniterm.app/Contents/MacOS/ift" "$DATA/config/settings.json" >/dev/null
                                shot 01-opened 1.0
# A click in the text locks the card; the keys then walk to the end of line 2
# (Cmd+Up: top of the file, Down, End), so the card's size does not matter.
click 500 400;                  shot 02-locked 0.4
key_code 126 "command down"; key_code 125; key_code 119
type_text '\"'; type_text "ui.";  shot 03-popup 0.6
key_code 125; key_code 125;     shot 04-down-twice 0.4
type_text "fitP";               shot 05-narrowed 0.5
key_code 48;                    shot 06-accepted 0.5
key_code 53;                    shot 07-escape-with-no-popup 0.3
# Values: line 3 is `  "ui.fullscreen": `; a quote there opens its choices.
key_code 125; key_code 119;
type_text '\"';                 shot 08-values 0.5
key_code 125;                   shot 09-values-down 0.4
key_code 48;                    shot 10-value-accepted 0.5
drive_stop
