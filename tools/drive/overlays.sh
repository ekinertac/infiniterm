#!/bin/sh
# What the four overlays look like on a canvas: the omnibox, the palette, a
# dialog and the shortcuts panel, each over the same cards, for judging
# whether they read as floating.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
type_text "fastfetch"; key_code 36; wait_s 2
cmd t; type_text "ls -la ~/Code | head -20"; key_code 36; wait_s 1
cmd 2; wait_s 1;                  shot 00-canvas 0.8
key l "command down";             shot 01-omnibox 0.8
key_code 53
cmd_shift p;                      shot 02-palette 0.8
key_code 53
cmd_shift w;                      shot 03-dialog 0.8
key_code 53
key "/" "command down";           shot 04-shortcuts 0.8
key_code 53
drive_log
drive_stop
