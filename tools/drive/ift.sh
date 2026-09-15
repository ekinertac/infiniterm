#!/bin/sh
# Phase 10's ift check, headless: no keys, no screenshots, so it runs on a
# locked Mac. Launches the bundle on a fresh scratch data dir, then drives
# it with `ift` and the hook binary over the socket and reads what the
# save file says.
set -e
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DATA=/tmp/infiniterm-ift
export INFINITERM_DATA_DIR=$DATA
IFT="$ROOT/target/debug/ift"
HOOK="$ROOT/target/debug/infiniterm-hook"
rm -rf "$DATA"; mkdir -p "$DATA"
pkill -f "$ROOT/target/bundle/infiniterm.app/Contents/MacOS/infiniterm$" 2>/dev/null || true
sleep 0.5
: > "$ROOT/run.log"
( cd "$ROOT" && open --stderr "$PWD/run.log" --stdout "$PWD/run.log" --env INFINITERM_DATA_DIR="$DATA" target/bundle/infiniterm.app )
for _ in 1 2 3 4 5 6 7 8 9 10; do [ -S "$DATA/infiniterm.sock" ] && break; sleep 1; done
echo "--- ift ls"
"$IFT" ls
CARD=$("$IFT" ls | head -1 | cut -f1)
echo "--- ift name (as card $CARD)"
INFINITERM_CARD_ID=$CARD "$IFT" name "named by ift"
echo "--- ift <file>"
INFINITERM_CARD_ID=$CARD "$IFT" "$ROOT/README.md:3"
echo "--- ift diff"
INFINITERM_CARD_ID=$CARD "$IFT" diff "$ROOT"
echo "--- hook: working, then idle"
echo '{"transcript_path":"/tmp/x.jsonl"}' | INFINITERM_CARD_ID=$CARD "$HOOK" UserPromptSubmit
sleep 0.3; "$IFT" ls | head -1
echo '{}' | INFINITERM_CARD_ID=$CARD "$HOOK" Stop
sleep 0.3; "$IFT" ls | head -1
sleep 1
echo "--- saved"
python3 -c "
import json; d=json.load(open('$DATA/workspace.json'))
for c in d['cards']: print(c['kind'], repr(c['title']), c['path'], c.get('root'))"
grep "cmd \|warn\|ift" "$ROOT/run.log" | sed 's/\[infiniterm\] //' | head -12
pkill -f "$ROOT/target/bundle/infiniterm.app/Contents/MacOS/infiniterm$" 2>/dev/null || true
