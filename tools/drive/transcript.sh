#!/bin/sh
# Phase 8's done-check: a transcript card over a real session file lists
# the turns, arrows move the cursor, Enter unfolds the tool calls.
# The card is seeded into the canvas: it normally opens beside an agent
# card from its hook events, which a driver run has none of.
. "$(dirname "$0")/lib.sh"
SESSION=${SESSION:-$HOME/.claude/projects/-Users-ekinertac-Code-infiniterm/097edf25-6753-40b6-baea-90f7a10de59d.jsonl}
mkdir -p "$DATA"
WS=05ae1568-a31f-4e88-a2d9-3568a577dd48
printf '{"version":3,"viewport":{"x":12.5,"y":12.5,"scale":1},"uiScale":1,"usage":{},"focusedId":"t1","workspaces":[{"id":"%s","name":"workspace 1","viewport":{"x":12.5,"y":12.5,"scale":1}}],"activeWorkspaceId":"%s","groups":[],"cards":[{"id":"t1","workspaceId":"%s","rect":{"x":12.5,"y":12.5,"w":1400,"h":800},"z":0,"title":"","cwd":"%s","groupId":null,"softGroupId":null,"splitFrom":null,"kind":"transcript","path":"%s","root":null,"explorer":false,"url":null,"sidebar":520,"sidebarTop":false,"zoom":null}]}' \
    "$WS" "$WS" "$WS" "$HOME" "$SESSION" > "$DATA/workspace.json"
KEEP=1
drive_start
cmd 1;                          shot 01-transcript 1.5
key_code 126; key_code 126;     shot 02-up-two 0.5
key_code 36;                    shot 03-expanded 0.5
key_code 115 2>/dev/null;       shot 04-home 0.5
drive_log
drive_stop
