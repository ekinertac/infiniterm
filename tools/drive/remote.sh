#!/bin/sh
# The app in remote mode (#118), headless: no keys, no screenshots. A scratch
# instance told INFINITERM_REMOTE runs its cards through a stand-in `ssh` that
# executes the remote command here, against a separate "server" data dir. So
# the daemon sockets must appear under the server dir and never under the
# app's own, `ift send` and `ift read` must work on a remote card, and a
# relaunch must bring the same shell back with its history.
set -e
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
APP="$ROOT/target/bundle/infiniterm.app"
BIN="$APP/Contents/MacOS"
CLIENT=/tmp/infiniterm-rm-c
SERVER=/tmp/infiniterm-rm-s
CONFIG=/tmp/infiniterm-rm-cfg
rm -rf "$CLIENT" "$SERVER" "$CONFIG"; mkdir -p "$CLIENT" "$SERVER" "$CONFIG"
cat > "$CLIENT/ssh" <<STUB
#!/bin/sh
for last; do :; done
INFINITERM_DATA_DIR='$SERVER' PATH='$BIN':"\$PATH" exec sh -c "\$last"
STUB
chmod +x "$CLIENT/ssh"
export INFINITERM_DATA_DIR=$CLIENT
IFT="$BIN/ift"
cleanup() {
    pkill -f "$BIN/infiniterm\$" 2>/dev/null || true
    pkill -f "iftd --socket $SERVER" 2>/dev/null || true
}
trap cleanup EXIT
fail() { echo "FAIL: $*"; "$IFT" read 1 --all 2>&1 | tail -10; exit 1; }
launch() {
    pkill -f "$BIN/infiniterm\$" 2>/dev/null || true
    sleep 0.5
    : > "$ROOT/run.log"
    ( cd "$ROOT" && open --stderr "$PWD/run.log" --stdout "$PWD/run.log" \
        --env INFINITERM_DATA_DIR="$CLIENT" --env INFINITERM_CONFIG_DIR="$CONFIG" \
        --env INFINITERM_REMOTE=test@server --env INFINITERM_REMOTE_SSH="$CLIENT/ssh" \
        --env INFINITERM_REMOTE_IFT="$BIN/ift" "$APP" )
    for _ in 1 2 3 4 5 6 7 8 9 10; do [ -S "$CLIENT/infiniterm.sock" ] && break; sleep 1; done
    sleep 3
}

launch
"$IFT" ls | grep -q . || fail "the app did not start with a card"
"$IFT" send 1 'echo REMOTE_$((6*7))' --enter
sleep 2
"$IFT" read 1 --all | grep -q REMOTE_42 || fail "a command typed into the remote card did not run"
echo "--- sockets: server $(ls "$SERVER"/s/*.sock 2>/dev/null | wc -l | tr -d ' '), client $(ls "$CLIENT"/s/*.sock 2>/dev/null | wc -l | tr -d ' ')"
[ "$(ls "$SERVER"/s/*.sock 2>/dev/null | wc -l)" -ge 1 ] || fail "no daemon socket on the server side"
[ "$(ls "$CLIENT"/s/*.sock 2>/dev/null | wc -l)" -eq 0 ] || fail "a daemon socket appeared on the client side"

echo "--- relaunch: the same shell comes back"
sleep 3   # the canvas is saved on a debounce
pkill -f "$BIN/infiniterm\$" 2>/dev/null || true
sleep 2
launch
"$IFT" read 1 --all | grep -q REMOTE_42 || fail "the remote shell did not come back with its history"
"$IFT" send 1 'echo AGAIN_$((8*9))' --enter
sleep 2
"$IFT" read 1 --lines 6 | grep -q AGAIN_72 || fail "the reattached remote shell does not take input"
echo "PASS"
