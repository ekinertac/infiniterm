#!/bin/sh
# tools/restart-claude.sh against a fake `claude`, headless (no keys, no
# screenshots, so it runs on a locked Mac). A scratch instance with its own
# data and config dirs. Its card runs a fake `claude`, by absolute path
# because the login shell resets PATH and the real Claude would win, that
# leaves on the second Ctrl+C, printing a resume hint, and says RESUMED when
# started with --resume. The hook binary gives the card its session id the way
# a real Claude would; the script is told to resume with the fake (`--cmd`).
set -e
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DATA=/tmp/infiniterm-rc
CONFIG=/tmp/infiniterm-rc-config
FAKE=/tmp/infiniterm-rc-bin
SID=11111111-2222-3333-4444-555555555555
export INFINITERM_DATA_DIR=$DATA
export INFINITERM_CONFIG_DIR=$CONFIG
APP="$ROOT/target/bundle/infiniterm.app"
IFT="$APP/Contents/MacOS/ift"
# The script under test; RESTART=<path> runs another one (an older one, to see
# this scenario fail).
RESTART=${RESTART:-$ROOT/tools/restart-claude.sh}
HOOK="$APP/Contents/MacOS/infiniterm-hook"
rm -rf "$DATA" "$CONFIG" "$FAKE"; mkdir -p "$DATA" "$CONFIG" "$FAKE"
cat > "$FAKE/claude" <<'FAKE'
#!/bin/sh
# --named: a session with a name, whose hint is the name in quotes. It leaves
# on a typed /exit, or on a second Ctrl+C (a first only prints a reminder).
id=${2:-11111111-2222-3333-4444-555555555555}
if [ "$1" = --named ]; then id='"fake-name"'; fi
if [ "$1" = --resume ]; then echo "FAKE-CLAUDE RESUMED $id"; else echo "FAKE-CLAUDE STARTED $id"; fi
n=0
# Leaving takes two seconds after the hint, and anything typed meanwhile is
# swallowed (read one line from the terminal), as a real Claude's shutdown does.
leave() { echo; echo "Resume this session with:"; echo "claude --resume $id"; perl -e "alarm 2; <STDIN>"; exit 0; }
trap 'n=$((n+1)); if [ $n -ge 2 ]; then leave; else echo "(Press Ctrl-C again to exit)"; fi' INT
while :; do
    if read -r line; then [ "$line" = /exit ] && leave; fi
done
FAKE
chmod +x "$FAKE/claude"
printf '{\n}\n' > "$CONFIG/settings.json"
pkill -f "$APP/Contents/MacOS/infiniterm$" 2>/dev/null || true
sleep 0.5
: > "$ROOT/run.log"
( cd "$ROOT" && open --stderr "$PWD/run.log" --stdout "$PWD/run.log" --env INFINITERM_DATA_DIR="$DATA" --env INFINITERM_CONFIG_DIR="$CONFIG" "$APP" )
for _ in 1 2 3 4 5 6 7 8 9 10; do [ -S "$DATA/infiniterm.sock" ] && break; sleep 1; done
sleep 2
CARD=$("$IFT" ls | head -1 | cut -f1)
cleanup() {
    pkill -f "$APP/Contents/MacOS/infiniterm$" 2>/dev/null || true
    pkill -f "iftd --socket $DATA" 2>/dev/null || true
}
trap cleanup EXIT
fail() {
    echo "FAIL: $*"
    echo "--- the card's screen:"; "$IFT" read 1 --all | tail -20
    echo "--- ift ls --agents:"; "$IFT" ls --agents
    echo "--- ift sessions:"; "$IFT" sessions
    spid=$("$IFT" sessions | awk -F'\t' '$6 == "#1" { print $2 }' | head -1)
    echo "--- children of the card's shell ($spid):"
    ps -o pid=,ppid=,command= -p "$(pgrep -P "$spid" | tr '\n' ',' | sed 's/,$//')" 2>&1 | head -5
    exit 1
}

"$IFT" send 1 "$FAKE/claude" --enter
sleep 1.5
echo "{\"session_id\":\"$SID\"}" | INFINITERM_CARD_ID=$CARD "$HOOK" SessionStart claude
sleep 0.5
echo "--- ls --agents"
"$IFT" ls --agents
"$IFT" ls --agents | grep -q "$SID" || fail "the card has no session id"

# The window is not drawing: hidden, as with another app in front or another
# Space showing. Terminal output is parsed inside a frame, so a read would be
# stale or empty unless `ift read` parses it itself. (Needs System Events, so
# unlike the rest of this scenario it is not for a locked Mac.)
PID=$(pgrep -f "$APP/Contents/MacOS/infiniterm$" | head -1)
osascript -e "tell application \"System Events\" to set visible of (first process whose unix id is $PID) to false" || true
sleep 1

echo "--- from inside the card: skipped"
OUT=$(INFINITERM_CARD_ID=$CARD IFT="$IFT" "$RESTART" --dry-run) || true
echo "$OUT"
case $OUT in *"this card"*) ;; *) fail "did not skip its own card" ;; esac

echo "--- dry run"
OUT=$(IFT="$IFT" "$RESTART" --dry-run) || true
echo "$OUT"
case $OUT in *"would restart session $SID"*) ;; *) fail "dry run said nothing useful" ;; esac

echo "--- restart"
OUT=$(IFT="$IFT" "$RESTART" --wait 15 --cmd "$FAKE/claude --resume {id}") || true
echo "$OUT"
case $OUT in *"#1: restarted $SID"*) ;; *) fail "not restarted" ;; esac
sleep 1.5
"$IFT" read 1 --lines 6
"$IFT" read 1 --lines 6 | grep -q "FAKE-CLAUDE RESUMED $SID" || fail "the card did not resume"

echo "--- a named session"
"$IFT" send 1 "/exit" --enter
sleep 2
"$IFT" send 1 "$FAKE/claude --named" --enter
sleep 1.5
OUT=$(IFT="$IFT" "$RESTART" --wait 15 --cmd "$FAKE/claude --resume {id}") || true
echo "$OUT"
case $OUT in *'#1: restarted "fake-name"'*) ;; *) fail "the named session was not restarted" ;; esac
sleep 1.5
"$IFT" read 1 --lines 5
"$IFT" read 1 --lines 5 | grep -q 'claude --resume "fake-name"' || fail "the card did not get the quoted name"
"$IFT" read 1 --lines 3 | grep -q "FAKE-CLAUDE RESUMED fake-name" || fail "the named session did not resume"

echo "--- nothing running in the card: left alone, no Ctrl+C sent to a prompt"
"$IFT" send 1 "/exit" --enter
sleep 3
OUT=$(IFT="$IFT" "$RESTART" --wait 5) || true
echo "$OUT"
case $OUT in *"#1: nothing is running in it, left alone"*) ;; *) fail "did not notice an idle shell" ;; esac

echo "--- something else running (not Claude): left alone"
"$IFT" send 1 "sleep 300" --enter
sleep 1.5
OUT=$(IFT="$IFT" "$RESTART" --wait 5) || true
echo "$OUT"
case $OUT in *"#1: runs sleep, not Claude, left alone"*) ;; *) fail "touched a card that is not running Claude" ;; esac
"$IFT" read 1 --lines 4 | grep -q '\^C' && fail "a Ctrl+C reached the other program"
"$IFT" send 1 --key ctrl-c
echo "PASS"
