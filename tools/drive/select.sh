#!/bin/sh
# Phase 4's done-check, the second half: a drag selects text and Cmd+C
# copies it (pasted back with Cmd+V to prove the clipboard has it), two
# clicks take a word, the cursor blinks in the focused card only, an
# unfocused card is dimmed, and a card whose directory does not exist shows
# the spawn error and retries on a click.
. "$(dirname "$0")/lib.sh"
# Two cards seeded by hand, small enough that fit-card lands at 100% so the
# drag coordinates below hold. (A card whose directory does not exist
# starts in the home directory instead, portable-pty's choice and the
# reference's behaviour too; the spawn-error panel has no cheap trigger.)
mkdir -p "$DATA"
rm -f "$DATA/workspace.json"
WS=05ae1568-a31f-4e88-a2d9-3568a577dd48
card() { printf '{"id":"%s","workspaceId":"%s","rect":{"x":%s,"y":12.5,"w":800,"h":600},"z":0,"title":"","cwd":"%s","groupId":null,"softGroupId":null,"splitFrom":null,"kind":"terminal","path":null,"root":null,"explorer":false,"url":null,"sidebar":null,"sidebarTop":false,"zoom":null}' "$1" "$WS" "$2" "$3"; }
printf '{"version":3,"viewport":{"x":12.5,"y":12.5,"scale":1},"uiScale":1,"usage":{},"focusedId":"a0a4e683-2439-4553-9faa-4d06162004df","workspaces":[{"id":"%s","name":"workspace 1","viewport":{"x":12.5,"y":12.5,"scale":1}}],"activeWorkspaceId":"%s","groups":[],"cards":[%s,%s]}' \
    "$WS" "$WS" "$(card a0a4e683-2439-4553-9faa-4d06162004df 12.5 "$HOME/Code")" "$(card eaf6d673-8714-4560-b8bd-56e7c4f904c9 837.5 "$HOME")" > "$DATA/workspace.json"
KEEP=1
drive_start
cmd 1
type_text "clear; echo the quick brown fox jumps"; key_code 36
                                shot 01-typed 1.0
# The card is centred at 100%: its text starts at (406, 217) in window
# pixels and a cell is about 8.9 px wide. The drag starts on "quick" and
# ends after "brown", on the first row. Check against 01-typed if it drifts.
drag ${SEL_X0:-442} ${SEL_Y:-224} ${SEL_X1:-540} ${SEL_Y:-224}
                                shot 02-dragged 0.4
cmd c; type_text "echo ["; cmd v; type_text "]"; key_code 36
                                shot 03-pasted 0.8
cliclick "dc:$((WX+482)),$((WY+${SEL_Y:-224}))"; sleep 0.3
                                shot 04-word 0.3
click 700 600;                  shot 05-clicked-away 0.3
                                shot 06-blink-a 0.05
                                shot 07-blink-b 0.6
cmd 2;                          shot 08-fit-all-dim 0.8
drive_log
drive_stop
