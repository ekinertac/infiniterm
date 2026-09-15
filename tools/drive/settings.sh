#!/bin/sh
# Phase 5's done-check: a settings.json edit applies within a second (font
# size and theme here), and the window comes back where it was after a
# quit. Runs on a COPY of the real config directory; the Tauri app watches
# the real one.
. "$(dirname "$0")/lib.sh"
CONFIG=/tmp/infiniterm-drive/config
mkdir -p "$CONFIG"
cp "$HOME/.config/infiniterm/settings.json" "$HOME/.config/infiniterm/keybindings.json" "$CONFIG/" 2>/dev/null || true
FRESH=1
drive_start
                                shot 01-before 1.5
# The size and the theme in one edit; the watcher polls every second.
python3 - "$CONFIG/settings.json" <<'PY'
import re, sys
p = sys.argv[1]; s = open(p).read()
s = re.sub(r'"fontSize":\s*\d+', '"fontSize": 26', s)
s = re.sub(r'"theme":\s*"[^"]*"', '"theme": "Dracula"', s) if '"theme"' in s else s.replace('{', '{\n  "theme": "Dracula",', 1)
open(p, 'w').write(s)
PY
wait_s 2
                                shot 02-after-edit 0.3
osascript -e "$SE to set position of window 1 to {300, 200}"
wait_s 1.5
quit
echo "window.json: $(cat "$DATA/window.json" 2>/dev/null)"
FRESH= KEEP=1 KEEP_SHOTS=1 drive_start
                                shot 03-relaunched 1.0
echo "relaunched at $WX,$WY (expected 300,200)"
grep -c "" "$DATA/workspace.json" >/dev/null && echo "workspace.json saved"
drive_log
drive_stop
