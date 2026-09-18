#!/bin/bash
# The decoy over a card: Cmd+Shift+H masks the focused card with
# terminal.decoyCommand, Enter lifts it. One shot each way, on a fresh
# canvas whose one card is a shell.
set -e
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
. "$ROOT/tools/drive/lib.sh"
FRESH=1 drive_start
type_text "echo the secret is here"
key_code 36
shot 01-plain 1
key_code 4 "command down, shift down"   # h
shot 02-masked 2
key_code 36                             # enter lifts the mask
shot 03-lifted 1
drive_log
drive_stop
