#!/bin/sh
# Screenshot one app's window by owner name, for verifying a build without
# being at the Mac. Ekin is usually remote; every visual check in the
# spikes went through this plus `cliclick` for input.
#   tools/shot.sh canvas out.png      # first window owned by "canvas"
# Needs tools/winid built once: swiftc -O tools/winid.swift -o tools/winid
# (the binary is gitignored). Screen Recording permission for the terminal
# that runs it, or the capture comes out black.
set -e
cd "$(dirname "$0")/.."
[ -x tools/winid ] || swiftc -O tools/winid.swift -o tools/winid
id=$(tools/winid "$1" | head -1 | cut -f1)
[ -n "$id" ] || { echo "no window owned by $1" >&2; exit 1; }
screencapture -x -l "$id" "$2"
echo "$2"
