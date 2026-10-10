#!/bin/sh
# #239: typing two letters offers words. Two files in one card (tabs): a.rs
# has `calculate_total` and a comment holding `secret_word`; typing in b.rs,
# `calc` offers calculate_total marked with the tab a.rs, and `sec` offers
# nothing, since secret_word is only in a comment. Enter after a word is a
# line break; Tab takes the top row.
. "$(dirname "$0")/lib.sh"
FRESH=1
mkdir -p "$DATA/words"
printf 'fn calculate_total(items: &[u32]) -> u32 {\n    // secret_word lives only in a comment\n    items.iter().sum()\n}\n' > "$DATA/words/a.rs"
printf 'fn configure_widget() {}\n' > "$DATA/words/b.rs"
drive_start
"$ROOT/target/bundle/infiniterm.app/Contents/MacOS/ift" "$DATA/words/a.rs" >/dev/null
                                shot 01-opened 1.0
# The tree lists the folder; Enter on b.rs opens it as a second tab.
cmd_shift p; type_text "show or hide the file tree"; key_code 36
key_code 125; key_code 36;      shot 02-b-in-a-tab 0.8
# A click in the text locks the card; then to the end of the file.
click 900 400;                  shot 03-locked 0.4
key_code 125 "command down"; key_code 36
type_text "calc";               shot 04-other-tab-word 0.6
key_code 53
key_code 36
type_text "sec";                shot 05-comment-word-not-offered 0.5
drive_stop
