#!/bin/sh
# The three agent states on one canvas: working, blocked on you, finished.
# Driven through the hook binary, which is the same program Claude Code runs
# from ~/.claude/settings.json, so this is the real path with the waiting
# removed.
#
# Three cards, one state each, seen together: the point is whether the three
# colours can be told apart, which is a question about them side by side and
# at the zoom you actually work at.
. "$(dirname "$0")/lib.sh"
IFT="$ROOT/target/debug/ift"
HOOK="$ROOT/target/debug/infiniterm-hook"
FRESH=1
drive_start
type_text "echo working"; key_code 36; wait_s 1
cmd t; type_text "echo blocked on you"; key_code 36; wait_s 1
cmd t; type_text "echo finished"; key_code 36; wait_s 1
cmd 2; wait_s 1
# id, group, directory, state, remote: one line per card, in order.
A=$("$IFT" ls | sed -n 1p | cut -f1)
B=$("$IFT" ls | sed -n 2p | cut -f1)
C=$("$IFT" ls | sed -n 3p | cut -f1)
echo '{"transcript_path":"/tmp/a.jsonl"}' | INFINITERM_CARD_ID=$A "$HOOK" UserPromptSubmit
echo '{"transcript_path":"/tmp/b.jsonl"}' | INFINITERM_CARD_ID=$B "$HOOK" Notification
echo '{"transcript_path":"/tmp/c.jsonl"}' | INFINITERM_CARD_ID=$C "$HOOK" Stop
wait_s 1;                            shot 01-three-states 1
"$IFT" ls | cut -f1,4 | sed 's/^/    /'
# The zoom the complaint was about: two oranges were one colour here.
key_code 27 "command down"; key_code 27 "command down"
wait_s 1;                            shot 02-zoomed-out 1
# A group of all three reports whichever most wants you: the amber wins.
key_code 123 "command down, shift down"; key_code 123 "command down, shift down"
cmd g; wait_s 1; type_text "three"; key_code 36
wait_s 1;                            shot 03-group-reports-waiting 1.5
drive_log
drive_stop
