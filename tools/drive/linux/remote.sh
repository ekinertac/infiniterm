#!/usr/bin/env bash
# The Linux GUI driver's box half: runs ON the Linux machine (a droplet made
# by box.sh, or any host given as LINUX_HOST), against a virtual display.
# box.sh copies the tree there and calls this as `remote.sh <verb> ...`.
#
# Verbs:
#   start [fresh]     Xvfb on :99 with openbox, then a scratch instance
#                     (data dir $DATA, default /tmp/ift-drive). `fresh`
#                     empties the data dir first. Waits for the socket.
#   stop              ends the instance, by pid from its socket (never by
#                     name: see CLAUDE.md's pkill trap).
#   shot <file>       screenshot of the whole display.
#   mean <file>       mean brightness of a screenshot, 0 for all black.
#   chord <keys...>   a HELD chord: `chord super minus` presses super then
#                     minus, waits HOLD seconds, releases in reverse.
#   drag <x1> <y1> <x2> <y2>
#                     a left-button drag in steps.
#   dblclick <x> <y>  a left double-click.
#   win               the app window's frame on screen: x y w h.
#   type <text>       types text into the focused window.
#   layout <xkb>      the keyboard layout, `layout tr` or `layout us intl`.
#   ift <args...>     `ift` against the scratch instance.
#   setting <json> [fresh]
#                     writes <data>/config/settings.json (before `start`);
#                     `fresh` empties the data dir first.
#   settings          prints <data>/config/settings.json.
#
# Why these rules (docs/agent/linux-port.md): without a window manager the
# first frame never shows, so openbox always runs; a chord sent with
# `xdotool key` is released before the app asks the X server which key is
# down and falls back to the layout name, so chords are held.
#
# Related: box.sh (the Mac side), smoke.sh (the self-checking scenario).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
export DISPLAY=${DISPLAY:-:99}
DATA=${DATA:-/tmp/ift-drive}
export INFINITERM_DATA_DIR=$DATA
export PATH=$ROOT/target/debug:$PATH
HOLD=${HOLD:-0.1}

app_pid() {
    # The instance holds its socket; that is how it is told apart from any
    # other infiniterm on the box.
    # No socket is "not running", not a failure (set -e and pipefail).
    { lsof -t "$DATA/infiniterm.sock" 2>/dev/null || true; } | head -1
}

start_display() {
    if ! xdpyinfo >/dev/null 2>&1; then
        # Every process started here is detached from the ssh session
        # (setsid, no stdin), or ssh waits for it and `start` never returns.
        setsid Xvfb "$DISPLAY" -screen 0 1920x1080x24 </dev/null >/tmp/ift-xvfb.log 2>&1 &
        for _ in $(seq 50); do xdpyinfo >/dev/null 2>&1 && break; sleep 0.1; done
    fi
    pgrep -x openbox >/dev/null || { setsid openbox </dev/null >/tmp/ift-openbox.log 2>&1 & }
    sleep 1
}

case "${1:-}" in
start)
    [ "${2:-}" = fresh ] && rm -rf "$DATA"
    mkdir -p "$DATA/themes"
    # The bundle ships the themes; a source tree run has to put them there.
    cp -n "$ROOT"/assets/themes/*.itermcolors "$DATA/themes/" 2>/dev/null || true
    start_display
    [ -n "$(app_pid)" ] && { echo "already running: pid $(app_pid)"; exit 0; }
    # The redirects are on the whole subshell: on the command alone, the
    # forked `cd && ...` list kept ssh's stdout and `start` never returned.
    (cd /tmp && INFINITERM_KEYLOG=1 exec setsid infiniterm) </dev/null >"$DATA/run.log" 2>&1 &
    for _ in $(seq 100); do [ -n "$(app_pid)" ] && break; sleep 0.1; done
    [ -n "$(app_pid)" ] || { echo "the app did not start; $DATA/run.log:" >&2; tail -20 "$DATA/run.log" >&2; exit 1; }
    # The window draws a moment after the socket is up.
    sleep 3
    echo "started: pid $(app_pid)"
    ;;
stop)
    pid=$(app_pid)
    if [ -n "$pid" ]; then
        kill "$pid"
        for _ in $(seq 50); do [ -z "$(app_pid)" ] && break; sleep 0.1; done
        echo "stopped: pid $pid"
    else
        echo "not running"
    fi
    ;;
shot)
    import -window root "${2:?shot <file>}"
    echo "$2"
    ;;
mean)
    # A whole number 0 to 255.
    convert "${2:?mean <file>}" -colorspace Gray -format '%[fx:int(mean*255)]' info:
    echo
    ;;
chord)
    shift
    [ $# -gt 0 ] || { echo "chord <keys...>" >&2; exit 2; }
    xdotool keydown "$@"
    sleep "$HOLD"
    # Reverse order, so the modifier is released last.
    keys=("$@")
    for ((i = ${#keys[@]} - 1; i >= 0; i--)); do xdotool keyup "${keys[i]}"; done
    sleep 0.3
    ;;
drag)
    # `drag <x1> <y1> <x2> <y2>`: the left button pressed at the first point,
    # moved in steps (a window manager starts a move on motion, not on a
    # jump), released at the second.
    x1=${2:?drag <x1> <y1> <x2> <y2>} y1=${3:?} x2=${4:?} y2=${5:?}
    xdotool mousemove "$x1" "$y1" mousedown 1
    for s in 1 2 3 4 5 6 7 8 9 10; do
        xdotool mousemove $((x1 + (x2 - x1) * s / 10)) $((y1 + (y2 - y1) * s / 10))
        sleep 0.03
    done
    xdotool mouseup 1
    sleep 0.5
    ;;
dblclick)
    xdotool mousemove "${2:?dblclick <x> <y>}" "${3:?}" click --repeat 2 --delay 80 1
    sleep 0.8
    ;;
win)
    # The app window's frame on screen: `x y w h`. By name, which is safe on
    # the box: the instance under test is the only infiniterm there.
    id=$(xdotool search --name '^infiniterm$' | head -1)
    eval "$(xdotool getwindowgeometry --shell "$id")"
    echo "$X $Y $WIDTH $HEIGHT"
    ;;
type)
    xdotool type --delay 30 "${2:?type <text>}"
    ;;
layout)
    # The layout lives on the display, so it needs one before the app.
    start_display
    setxkbmap "${2:?layout <xkb>}" ${3:+-variant "$3"}
    ;;
ift)
    shift
    ift "$@"
    ;;
setting)
    [ "${3:-}" = fresh ] && rm -rf "$DATA"
    mkdir -p "$DATA/config"
    printf '%s\n' "${2:?setting <json>}" >"$DATA/config/settings.json"
    ;;
settings)
    cat "$DATA/config/settings.json"
    ;;
*)
    # The header comment is the help.
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"
    exit 2
    ;;
esac
