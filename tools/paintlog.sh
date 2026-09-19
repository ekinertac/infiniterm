#!/bin/sh
# Relaunches the installed app with the paint log on, into ~/infiniterm.log.
# Quit the app first (Cmd+Q); read the log with `grep '\[paint\]' ~/infiniterm.log`.
: > "$HOME/infiniterm.log"
open --stderr "$HOME/infiniterm.log" --stdout "$HOME/infiniterm.log" --env INFINITERM_KEYLOG=1 -a infiniterm
