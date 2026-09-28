#!/bin/sh
# Batch 2's done-check: Cmd+D adds a cursor per occurrence instead of
# moving one selection, typing lands at every cursor at once, Escape
# drops to the primary before dropping its selection, and Cmd+Shift+L
# splits a selection into one per line.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
cmd_shift p; type_text "new untitled"; key_code 36
key_code 36                      # bare Enter: locks the card
type_text "foo bar"; key_code 36
type_text "foo baz"; key_code 36
type_text "foo qux"
                                  shot 01-typed 0.6

key_code 126 "command down"     # Cmd+Up: doc start, onto the first "foo"
cmd d;                           shot 02-first-foo-selected 0.4
cmd d;                           shot 03-second-foo-added 0.4
cmd d;                           shot 04-third-foo-added 0.4

key_code 53                      # Escape: down to the primary cursor
type_text "X";                   shot 05-only-the-primary-line-changed 0.5

cmd a
cmd_shift l;                     shot 06-split-into-lines 0.5
type_text "Y";                   shot 07-every-line-replaced 0.5

drive_log
drive_stop
