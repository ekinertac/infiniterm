#!/bin/sh
# DevTools in a card: a browser card on a page, the palette's "open
# DevTools", a second browser card beside it showing the DevTools page
# connected to the first. Checks the card count through `ift ls` and
# screenshots the pair. Needs the network (example.com).
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
cmd_shift p; type_text "browser open a url"; key_code 36
type_text "https://example.com"; key_code 36
wait_s 6
cards_before=$(ift ls 2>/dev/null | wc -l | tr -d ' ')
cmd_shift p; type_text "browser open devtools"; key_code 36
wait_s 8
cards_after=$(ift ls 2>/dev/null | wc -l | tr -d ' ')
cmd 2;                          shot 01-devtools 3.0
echo "cards before: $cards_before, after: $cards_after"
[ "$cards_after" -gt "$cards_before" ] || { echo "FAIL: DevTools opened no card" >&2; drive_stop; exit 1; }
grep -i "devtools\|remote-allow" "$ROOT/run.log" | head -5
[ "${KEEP_RUNNING:-}" = 1 ] || drive_stop
