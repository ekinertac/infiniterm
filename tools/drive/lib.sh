#!/bin/sh
# Drives the native app for a test run: launch on a scratch data dir (a
# copy of the real canvas, never the real file), send keys and clicks,
# screenshot each step, print the command log. Source this from a scenario
# in tools/drive/ and call `drive_start` then the step functions.
#
# Keys go through System Events, not cliclick: cliclick's typed characters
# arrive in gpui with the fn flag and no character, and its Return and
# Escape never arrive at all (macOS hands non-printing keys to the input
# context first). cliclick is still what moves the mouse. Coordinates are
# WINDOW-relative; the window is found by owner name each time.
#
# Never run this while Ekin is using the Mac without saying so first.
set -e
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DATA=${INFINITERM_DATA_DIR:-/tmp/infiniterm-drive}
SHOTS=${SHOTS:-/tmp/infiniterm-drive/shots}
SE='tell application "System Events" to tell process "infiniterm"'

drive_start() {
    mkdir -p "$DATA" "$SHOTS"
    [ "${KEEP_SHOTS:-}" = 1 ] || rm -f "$SHOTS"/*.png
    # FRESH=1 starts empty, KEEP=1 uses whatever the scenario put in $DATA,
    # the default is a copy of the real canvas.
    if [ "${FRESH:-}" = 1 ]; then rm -f "$DATA/workspace.json"; elif [ "${KEEP:-}" = 1 ]; then :; else
        cp "$HOME/Library/Application Support/dev.ekinertac.infiniterm/workspace.json" "$DATA/workspace.json" 2>/dev/null || true
    fi
    pkill -f "MacOS/infiniterm$" 2>/dev/null || true
    sleep 0.5
    : > "$ROOT/run.log"
    # CONFIG moves ~/.config/infiniterm too, for a scenario that edits settings.
    ( cd "$ROOT" && open --stderr "$PWD/run.log" --stdout "$PWD/run.log" --env INFINITERM_DATA_DIR="$DATA" --env INFINITERM_KEYLOG=1 ${CONFIG:+--env INFINITERM_CONFIG_DIR="$CONFIG"} target/bundle/infiniterm.app )
    sleep 3
    osascript -e 'tell application "infiniterm" to activate'
    sleep 0.5
    _win
    # Activation alone sometimes leaves the window not key; a click on the
    # title bar (never a card) makes it so.
    click 700 20
    # If something else is still in front, Ekin is using the Mac and every
    # key below would land in his window (a Cmd+Shift+P once opened Page
    # Setup in iTerm2 and the rest typed into it). Stop before the first key.
    FRONT=$(osascript -e 'tell application "System Events" to get name of first application process whose frontmost is true')
    if [ "$FRONT" != "infiniterm" ]; then
        echo "abort: $FRONT is frontmost, not infiniterm; someone is using the Mac" >&2
        drive_stop
        exit 2
    fi
}

_win() {
    B=$("$ROOT/tools/winid" infiniterm | head -1)
    WX=$(echo "$B" | grep -o '"X": [0-9-]*' | grep -o '[0-9-]*$')
    WY=$(echo "$B" | grep -o '"Y": [0-9-]*' | grep -o '[0-9-]*$')
    [ -n "$WX" ] || { echo "no infiniterm window" >&2; exit 1; }
}

# key <char> [modifiers]: modifiers as System Events words: "command down",
# "command down, shift down". key_code <n> for named keys (36 return, 53
# escape, 123-126 left right down up, 51 delete).
_using() { if [ -n "$1" ]; then printf ' using {%s}' "$1"; fi; }
key() { osascript -e "$SE to keystroke \"$1\"$(_using "$2")"; sleep 0.25; }
key_code() { osascript -e "$SE to key code $1$(_using "$2")"; sleep 0.25; }
type_text() { osascript -e "$SE to keystroke \"$1\""; sleep 0.25; }
cmd() { key "$1" "command down"; }
cmd_shift() { key "$1" "command down, shift down"; }
cmd_alt() { key_code "$1" "command down, option down"; }
cmd_alt_shift() { key_code "$1" "command down, option down, shift down"; }
click() { cliclick "c:$((WX+$1)),$((WY+$2))"; sleep 0.25; }
shift_click() { cliclick "kd:shift" "c:$((WX+$1)),$((WY+$2))" "ku:shift"; sleep 0.25; }
drag() { cliclick "dd:$((WX+$1)),$((WY+$2))" "m:$((WX+$3)),$((WY+$4))" "m:$((WX+$3+1)),$((WY+$4))" "du:$((WX+$3+1)),$((WY+$4))"; sleep 0.3; }
cmd_drag() { cliclick "kd:cmd" "dd:$((WX+$1)),$((WY+$2))" "m:$((WX+$3)),$((WY+$4))" "m:$((WX+$3+1)),$((WY+$4))" "du:$((WX+$3+1)),$((WY+$4))" "ku:cmd"; sleep 0.3; }
shot() { sleep "${2:-0.4}"; "$ROOT/tools/shot.sh" infiniterm "$SHOTS/$1.png" >/dev/null; echo "shot $1"; }
wait_s() { sleep "$1"; }

drive_log() {
    grep 'cmd \|glide\|warn\|close ' "$ROOT/run.log" | sed 's/\[infiniterm\] //' | cut -c1-110
}

# Quit through the menu, so the saves flush the way a real quit does.
quit() { key q "command down"; sleep 1; }

drive_stop() {
    pkill -f "MacOS/infiniterm$" 2>/dev/null || true
    echo "shots in $SHOTS"
}
