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
HOOK="$APP/Contents/MacOS/infiniterm-hook"
rm -rf "$DATA" "$CONFIG" "$FAKE"; mkdir -p "$DATA" "$CONFIG" "$FAKE"
cat > "$FAKE/claude" <<'FAKE'
#!/bin/sh
id=${2:-11111111-2222-3333-4444-555555555555}
if [ "$1" = --resume ]; then echo "FAKE-CLAUDE RESUMED $id"; else echo "FAKE-CLAUDE STARTED $id"; fi
n=0
trap 'n=$((n+1)); if [ $n -ge 2 ]; then echo; echo "Resume this session with:"; echo "claude --resume $id"; exit 0; else echo "(Press Ctrl-C again to exit)"; fi' INT
while :; do sleep 0.2; done
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
fail() { echo "FAIL: $*"; "$IFT" read 1 --all | tail -12; exit 1; }

"$IFT" send 1 "$FAKE/claude" --enter
sleep 1.5
echo "{\"session_id\":\"$SID\"}" | INFINITERM_CARD_ID=$CARD "$HOOK" SessionStart claude
sleep 0.5
echo "--- ls --agents"
"$IFT" ls --agents
"$IFT" ls --agents | grep -q "$SID" || fail "the card has no session id"

echo "--- from inside the card: skipped"
OUT=$(INFINITERM_CARD_ID=$CARD IFT="$IFT" "$ROOT/tools/restart-claude.sh" --dry-run) || true
echo "$OUT"
case $OUT in *"this card"*) ;; *) fail "did not skip its own card" ;; esac

echo "--- dry run"
OUT=$(IFT="$IFT" "$ROOT/tools/restart-claude.sh" --dry-run) || true
echo "$OUT"
case $OUT in *"would restart session $SID"*) ;; *) fail "dry run said nothing useful" ;; esac

echo "--- restart"
OUT=$(IFT="$IFT" "$ROOT/tools/restart-claude.sh" --wait 15 --cmd "$FAKE/claude --resume {id}") || true
echo "$OUT"
case $OUT in *"#1: restarted $SID"*) ;; *) fail "not restarted" ;; esac
sleep 1.5
"$IFT" read 1 --lines 6
"$IFT" read 1 --lines 6 | grep -q "FAKE-CLAUDE RESUMED $SID" || fail "the card did not resume"
echo "PASS"
