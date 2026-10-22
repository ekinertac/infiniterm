#!/usr/bin/env bash
# The Linux smoke scenario: what the #464 spike checked by hand, as one run
# that fails loudly. Run after `box.sh up` (or with LINUX_HOST set):
#
#   tools/drive/linux/smoke.sh
#
# It syncs and builds this checkout, starts a fresh scratch instance and
# checks, each with a screenshot in $SHOTS:
#   1. the window draws (the first frame is not black);
#   2. a terminal card runs a shell under iftd and `ift read` shows its output;
#   3. on Turkish Q, Super + the key US prints `=` on (Turkish prints `-`)
#      makes the terminal font BIGGER: the chord took the physical key
#      (keycode.rs); gpui's layout name would have made it smaller;
#   4. a right-click on the card opens the drawn menu, and choosing Close
#      from it by keyboard closes the card;
#   5. a drag on the bare title bar moves the window and a double-click
#      maximises it.
# The terminal font being monospace is in screenshot 2, by eye.
# Exit 0 only when every check held. SKIP_BUILD=1 skips sync and build.
#
# Related: box.sh, remote.sh, docs/agent/linux-port.md.
set -euo pipefail

BOX=$(dirname "$0")/box.sh
fail() { echo "FAIL: $*" >&2; exit 1; }
r() { "$BOX" r "$@"; }

if [ "${SKIP_BUILD:-}" != 1 ]; then
    "$BOX" sync
    "$BOX" build
fi

r stop >/dev/null
r layout us
r setting '{"terminal.fontSize": 14}' fresh
r start >/dev/null

# 1. The first frame draws.
shot=$("$BOX" shot 1-first-frame)
mean=$(r mean /tmp/1-first-frame.png)
[ "$mean" -gt 0 ] || fail "the window is black (mean $mean): $shot"
echo "ok 1: the window draws (mean brightness $mean)"

# 2. A terminal card runs a shell. A first launch shows the welcome page
# alone, so the terminal is card 2.
r ift run card.new.terminal >/dev/null
for _ in $(seq 30); do r ift read 2 >/dev/null 2>&1 && break; sleep 0.5; done
r ift send 2 'echo smoke-$((6*7))' --enter >/dev/null
sleep 1
r ift read 2 | grep -q '^smoke-42' || fail "no shell output in card 2: $(r ift read 2 | tail -3)"
"$BOX" shot 2-terminal >/dev/null
echo "ok 2: a terminal card runs a shell"

# 3. The physical key on Turkish Q.
r layout tr
r chord super minus
sleep 1
settings=$(r settings)
r layout us
echo "$settings" | grep -Eq '"terminal.fontSize": *15' ||
    fail "Super + physical Equal on Turkish Q did not make the font bigger: $settings"
"$BOX" shot 3-turkish-bigger >/dev/null
echo "ok 3: a Turkish Q chord took the physical key"

# 4. The right-click menu the app draws (menu_view.rs): a right-click on
# card 2 opens its menu; Up from nothing selected is the last row, Close;
# Enter runs it and the card is gone.
r click 3 960 540
sleep 1
"$BOX" shot 4-menu-open >/dev/null
cards_before=$(r ift ls | wc -l)
r chord Up
r chord Return
sleep 1
cards_after=$(r ift ls | wc -l)
[ "$cards_after" -eq $((cards_before - 1)) ] ||
    fail "the menu's Close did not close card 2 ($cards_before cards, then $cards_after)"
"$BOX" shot 4-menu-closed >/dev/null
echo "ok 4: the right-click menu opens and runs a command"

# 5. The title bar (not_macos.rs::title_drag): a drag on its bare part moves
# the window, a double-click maximises it. The point is right of the tabs,
# inside the app's own bar (the window manager's bar is above the window).
read -r wx wy ww _ <<<"$(r win)"
bx=$((wx + ww - 300)) by=$((wy + 15))
r drag "$bx" "$by" $((bx - 100)) $((by + 80))
read -r mx my _ _ <<<"$(r win)"
[ $((wx - mx)) -ge 80 ] && [ $((my - wy)) -ge 60 ] ||
    fail "a drag on the title bar did not move the window: $wx,$wy then $mx,$my"
r dblclick $((bx - 100)) $((by + 80))
read -r _ _ zw _ <<<"$(r win)"
[ "$zw" -ge 1900 ] || fail "a double-click on the title bar did not maximise the window (width $zw)"
"$BOX" shot 5-maximised >/dev/null
r dblclick 960 15
echo "ok 5: the title bar moves and maximises the window"

r stop >/dev/null
echo "smoke passed; screenshots in ${SHOTS:-.scratchpad/linux-shots}"
