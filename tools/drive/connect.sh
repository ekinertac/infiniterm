#!/bin/sh
# `ift connect` end to end (#118), headless: the launcher opens a second app
# for a host (a stand-in ssh runs the remote commands here, against a separate
# "server" data dir), the instance gets its own folder under remotes/, a card
# in it runs a command on the "server", and a second `ift connect` for the same
# host says it is already open instead of opening another. No keys, no
# screenshots.
set -e
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
APP="$ROOT/target/bundle/infiniterm.app"
BIN="$APP/Contents/MacOS"
BASE=/tmp/infiniterm-cn
SERVER=/tmp/infiniterm-cn-server
rm -rf "$BASE" "$SERVER"; mkdir -p "$BASE/localcfg" "$SERVER"
cat > "$BASE/ssh" <<STUB
#!/bin/sh
for last; do :; done
INFINITERM_DATA_DIR='$SERVER' PATH='$BIN':"\$PATH" exec sh -c "\$last"
STUB
chmod +x "$BASE/ssh"
printf '{\n  "ui.cardLabelSize": 17\n}\n' > "$BASE/localcfg/settings.json"
export INFINITERM_DATA_DIR=$BASE
export INFINITERM_CONFIG_DIR=$BASE/localcfg
export INFINITERM_REMOTE_SSH=$BASE/ssh
HOST=ops@box
DIR="$BASE/remotes/ops@box"
cleanup() {
    pkill -f "$BIN/infiniterm\$" 2>/dev/null || true
    pkill -f "iftd --socket $SERVER" 2>/dev/null || true
}
trap cleanup EXIT
fail() { echo "FAIL: $*"; exit 1; }
pkill -f "$BIN/infiniterm\$" 2>/dev/null || true

echo "--- ift connect --check"
"$BIN/ift" connect "$HOST" --ift "$BIN/ift" --check
echo "--- ift connect"
"$BIN/ift" connect "$HOST" --ift "$BIN/ift"
for _ in 1 2 3 4 5 6 7 8 9 10 11 12; do [ -S "$DIR/infiniterm.sock" ] && break; sleep 1; done
[ -S "$DIR/infiniterm.sock" ] || fail "the instance never opened its socket under remotes/"
[ -f "$DIR/config/settings.json" ] || fail "its settings were not seeded"
grep -q cardLabelSize "$DIR/config/settings.json" || fail "the seed is not a copy of the local settings"
sleep 3
export INFINITERM_DATA_DIR=$DIR
"$BIN/ift" send 1 'echo CONNECTED_$((6*7))' --enter
sleep 2
"$BIN/ift" read 1 --all | grep -q CONNECTED_42 || fail "the card did not run the command"
[ "$(ls "$SERVER"/s/*.sock 2>/dev/null | wc -l)" -ge 1 ] || fail "no daemon socket on the server side"
echo "--- a second connect to the same host"
export INFINITERM_DATA_DIR=$BASE
OUT=$("$BIN/ift" connect "$HOST" --ift "$BIN/ift")
echo "$OUT"
case $OUT in *"already connected"*) ;; *) fail "opened a second window for the same host" ;; esac
# SHOTS=<dir> also looks at it: the remote window, and the Dock, which here
# autohides on the right and is shown by putting the pointer at that edge for a
# moment (so this part moves the mouse; leave it off while somebody is working).
if [ -n "${SHOTS:-}" ]; then
    mkdir -p "$SHOTS"
    PID=$(pgrep -f "$BIN/infiniterm\$" | head -1)
    screencapture -x -o -l "$("$ROOT/tools/winid" --pid "$PID" | head -1 | cut -f1)" "$SHOTS/remote-window.png"
    ORIG=$(cliclick p | grep -oE '[0-9]+,[0-9]+' | head -1)
    BOUNDS=$(osascript -e 'tell application "Finder" to get bounds of window of desktop')
    W=$(echo "$BOUNDS" | awk -F', *' '{print $3}'); H=$(echo "$BOUNDS" | awk -F', *' '{print $4}')
    cliclick "m:$((W - 1)),$((H / 2))"
    sleep 2
    screencapture -x "$SHOTS/dock.png"
    cliclick "m:$ORIG"
    echo "shots in $SHOTS"
fi
echo "PASS"
