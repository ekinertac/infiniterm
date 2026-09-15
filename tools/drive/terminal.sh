#!/bin/sh
# Phase 4's done-check, the first half: shells spawn in every terminal card,
# typing reaches the focused one, output paints, a TUI takes the keys.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
                                shot 01-first-shell 1.5
type_text "ls -la ~ | head -5"; key_code 36;  shot 02-ls 0.8
cmd t;                          shot 03-second-card 1.2
type_text "echo hello from card two"; key_code 36; shot 04-echo 0.6
cmd 2;                          shot 05-fit-all 0.6
type_text "htop"; key_code 36;  shot 06-htop 1.5
type_text "q";                  shot 07-htop-quit 0.8
cmd k;                          shot 08-cleared 0.4
drive_log
drive_stop
