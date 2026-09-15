#!/bin/sh
# Phase 3's done-check, driven: fit-all, a new card, hints, rename, swap,
# split and close, a group, a workspace, the palette, the shortcuts panel,
# a drag by the top edge, a Cmd+drag pan and a Cmd+scroll zoom. Read the
# shots in order; drive_log prints what dispatched.
#   tools/drive/phase3.sh            # on a copy of the real canvas
#   FRESH=1 tools/drive/phase3.sh    # on an empty canvas
. "$(dirname "$0")/lib.sh"
drive_start
cmd 2;                      shot 01-fit-all
cmd t;                      shot 02-new-card
cmd f;                      shot 03-hints
type_text d;                shot 04-hint-taken
cmd_shift r;                shot 05-rename-prompt
type_text api; key_code 36; shot 06-renamed
cmd_alt_shift 124;          shot 07-swap-right 0.05
                            shot 08-swap-landed 0.4
cmd d;                      shot 09-split-right
cmd w;                      shot 10-split-closed
cmd_shift 'p'; type_text zoom; shot 11-palette
key_code 53
cmd g; type_text grp; key_code 36; shot 12-grouped
cmd 3;                      shot 13-fit-group
cmd_shift n;                shot 14-new-workspace
key_code 18 "control down"; shot 15-workspace-1
key "/" "command down";     shot 16-shortcuts
key "/" "command down"
cmd_alt 124;                shot 17-focus-right
drive_log
drive_stop
