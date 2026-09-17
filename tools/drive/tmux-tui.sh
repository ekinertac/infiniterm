#!/bin/sh
# Full-screen programs under the tmux backend: the sizes the shell believes,
# and what a TUI actually draws, in a whole card and in a split one.
. "$(dirname "$0")/lib.sh"
SESSION=infiniterm-dev
tmux kill-session -t "$SESSION" 2>/dev/null || true
FRESH=1
drive_start
cmd 1; wait_s 1
type_text "echo TERM=\$TERM cols=\$(tput cols) rows=\$(tput lines)"; key_code 36; wait_s 2
shot 01-sizes 1
echo "--- what tmux thinks the window is ---"
tmux list-windows -t "$SESSION" -F '   #{window_id} #{window_width}x#{window_height} panes=#{window_panes}' 2>&1 || true
echo "--- what the card believes (the shell answered above) ---"
tmux capture-pane -p -t "$SESSION" 2>/dev/null | grep "TERM=" | tail -1
# A full-screen program in a whole card.
type_text "less /etc/services"; key_code 36; wait_s 2
shot 02-less-full-card 1.5
type_text "q"; wait_s 1
# The same in a split.
cmd_shift d; wait_s 2
type_text "less /etc/services"; key_code 36; wait_s 2
shot 03-less-split 1.5
type_text "q"; wait_s 1
echo "--- sizes after the split ---"
tmux list-windows -t "$SESSION" -F '   #{window_id} #{window_width}x#{window_height}' 2>&1 || true
drive_log
drive_stop
