#!/bin/bash
# A picture in an editor card: `ift <png>` on a fresh canvas, one shot
# with the image fitted to the card, one after Cmd+1 fits the card to the
# window. Checks that the file is untouched afterwards (the buffer is
# empty and a save must not write it).
set -e
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
. "$ROOT/tools/drive/lib.sh"
IFT="$ROOT/target/debug/ift"
PIC=$(mktemp -d)/icon.png
cp "$ROOT/assets/icon.png" "$PIC"
BEFORE=$(md5 -q "$PIC")
FRESH=1 drive_start
"$IFT" "$PIC"
shot 01-image 1.5
key 1 "command down"
shot 02-fitted 1
key s "command down"
shot 03-after-save 0.5
[ "$(md5 -q "$PIC")" = "$BEFORE" ] || echo "FAIL: the picture changed on disk"
drive_log
drive_stop
