#!/bin/sh
# Phase 6's done-check: an editor card opens on a file with syntax colours
# and a gutter, typing edits it, Cmd+S saves, an untitled editor asks for a
# path, Cmd+F finds, Cmd+K shows the file tree and Enter in it opens a file.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
cmd_shift p; type_text "open a file"; key_code 36
type_text "$HOME/Code/infini-rust/infiniterm-editor/src/search.rs"; key_code 36
                                shot 01-file-opened 1.2
cmd 1;                          shot 02-fit 0.8
key_code 125; key_code 125; key_code 125; type_text "// typed here"
                                shot 03-typed 0.5
cmd z;                          shot 04-undone 0.4
cmd f; type_text "matches";     shot 05-find 0.5
key_code 36;                    shot 06-find-next 0.4
key_code 53;                    shot 07-find-closed 0.4
cmd k;                          shot 08-tree 0.8
key_code 125; key_code 125; key_code 36; shot 09-tree-opened-file 1.0
cmd k;                          shot 10-tree-hidden 0.5
cmd_shift p; type_text "new untitled"; key_code 36
type_text "hello from a new buffer"; shot 11-untitled 0.6
cmd s; type_text "/tmp/infiniterm-drive/saved.txt"; key_code 36
                                shot 12-saved 0.8
echo "saved.txt: $(cat /tmp/infiniterm-drive/saved.txt 2>/dev/null)"
# The config pair: two editors, the defaults read-only.
cmd_shift p; type_text "open settings"; key_code 36
                                shot 13-config-pair 1.2
# A draft survives a quit: type into the saved file, quit, relaunch.

click 700 20
cmd 2
type_text " plus a draft"; wait_s 1.2
quit
FRESH= KEEP=1 KEEP_SHOTS=1 drive_start
cmd 2;                          shot 14-draft-restored 1.2
ls "$DATA/drafts/" 2>/dev/null | sed 's/^/draft: /'
drive_log
drive_stop
