#!/bin/sh
# The shortcuts panel: opens centred over a backdrop, filters as you type,
# Escape closes it.
. "$(dirname "$0")/lib.sh"
drive_start
cmd 2
key "/" "command down";     shot 01-panel
type_text swap;             shot 02-filtered
key_code 53;                shot 03-closed
drive_log
drive_stop
