#!/bin/sh
# The page's own keys inside a browser card: find with a match count, back
# and forward, reload. Then the undo for closing a card, which is not the
# page's but is what makes closing one cheap.
#
# The card is sent to two addresses through the omnibox rather than by
# clicking a link: a click that misses the link leaves no history, and back
# then proves nothing.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
# A browser card through the omnibox, which is now the shortest way in.
key l "command down"
type_text "example.com"; key_code 36;      shot 01-page 4
# The page owns Cmd+F: a bar top right, a live count as the query grows.
key f "command down";                      shot 02-find-bar 1
type_text "domain";                        shot 03-find-count 1.5
key_code 36;                               shot 04-find-next 1
type_text "zzz";                           shot 05-no-matches 1.5
key_code 53;                               shot 06-find-closed 1
# A second address in the same card, so back has somewhere to go.
key l "command down"
type_text "iana.org"; key_code 36;         shot 07-second-page 6
key "[" "command down";                    shot 08-back 5
key "]" "command down";                    shot 09-forward 5
key r "command down";                      shot 10-reloaded 3
# Close the card and put it back where it was.
key w "command down";                      shot 11-closed 1
key t "command down, control down";        shot 12-reopened 2
drive_log
drive_stop
