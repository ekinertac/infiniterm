#!/bin/sh
# Checks tools/usage-stats.py against tools/fixtures/releases.json with the
# clock pinned: the newest published release is the one estimated, drafts
# are skipped, and 80 checks over 4 days at 4 a day is 5 Macs.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
fails=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fails=$((fails + 1)); fi; }
out=$(USAGE_STATS_JSON="$here/fixtures/releases.json" USAGE_STATS_NOW=2026-09-30T00:00:00Z "$here/usage-stats.py")
check "the newest published release is estimated" 'printf "%s" "$out" | grep -q "^v0.2.0-356: 80 update checks in 4.0 days, 20 a day, about 5 Macs"'
check "a draft release is left out" '! printf "%s" "$out" | grep -q v9.9.9'
check "newest first in the table" '[ "$(printf "%s\n" "$out" | sed -n 2p | cut -c1-10)" = "v0.2.0-356" ]'
check "zip and dmg counted per release" 'printf "%s\n" "$out" | grep -q "^v0.1.0-310 .* 90 *5 *2$"'
[ "$fails" = 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }
