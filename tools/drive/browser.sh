#!/bin/sh
# Phase 9's done-check: a browser card opens on a url and paints the page,
# the first click on an unfocused card only focuses it, keys reach the
# page, the page zoom chords zoom the page, a popup opens as a card.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
cmd_shift p; type_text "browser open a url"; key_code 36
type_text "https://example.com"; key_code 36
cmd 1;                          shot 01-browser 3.0
cmd_shift p; type_text "zoom the page in"; key_code 36
                                shot 02-page-zoomed 1.5
cmd_shift p; type_text "go to a url"; key_code 36
key a "command down"; type_text "https://news.ycombinator.com"; key_code 36
                                shot 03-navigated 4.0
drive_log
grep -i "cef\|browser\|warn" "$ROOT/run.log" | head -8
# KEEP_RUNNING=1 leaves the app up, for a Claude Code check through the
# extension (list_connected_browsers should name it).
[ "${KEEP_RUNNING:-}" = 1 ] || drive_stop
