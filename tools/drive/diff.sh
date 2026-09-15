#!/bin/sh
# Phase 7's done-check: a diff card beside an editor shows the working
# tree against HEAD with the collapse rule, Cmd+B adds the blame gutter,
# Cmd+K lists the changed files with their counts and Enter shows another.
# Runs against this repo's own uncommitted changes, so commit nothing
# between editing and running it.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
cmd_shift p; type_text "open a file"; key_code 36
type_text "$HOME/Code/infini-rust/infiniterm-core/src/model/cards_cmd.rs"; key_code 36
wait_s 1
cmd_shift p; type_text "against git HEAD"; key_code 36
cmd 1;                          shot 01-diff 1.5
cmd b;                          shot 02-blame 1.5
cmd k;                          shot 03-changed-files 1.0
key_code 125; key_code 36;      shot 04-second-file 1.2
key_code 53; cmd k; cmd k;      shot 05-tree-hidden 0.6
drive_log
grep "diff " "$ROOT/run.log" | sed 's/\[infiniterm\] //'
drive_stop
