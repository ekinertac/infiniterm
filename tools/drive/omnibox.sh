#!/bin/sh
# The omnibox: Cmd+L on a terminal makes a browser card, Cmd+L on that card
# prefills its address, Tab scopes a site search, and a card that is already
# open comes back as a result rather than a second copy of itself.
#
# Starts empty so the first card is the terminal Cmd+L is pressed on, and
# runs on the scratch data dir like every other scenario: the history file
# it writes is /tmp/infiniterm-drive/history.json, never Ekin's.
. "$(dirname "$0")/lib.sh"
FRESH=1
drive_start
key l "command down";                 shot 01-empty 0.8
type_text "example.com";              shot 02-typed 0.6
key_code 36;                          shot 03-made-a-card 3
key l "command down";                 shot 04-prefilled 0.8
key_code 53
# Tab to search: the offer appears on a prefix, Tab takes it, the chip shows.
key l "command down"
type_text "git";                      shot 05-tab-offer 0.8
key_code 48;                          shot 06-scoped 0.8
type_text "rope";                     shot 07-scoped-query 0.8
key_code 53;                          shot 08-unscoped 0.6
key_code 53
# The card made in act 1 is in history AND open, so both sections show.
key l "command down"
type_text "exam";                     shot 09-history-and-card 1
key_code 53
drive_log
drive_stop
