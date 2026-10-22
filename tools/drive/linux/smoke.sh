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
#   3. on Turkish Q, Ctrl + the key US prints `=` on (Turkish prints `-`)
#      makes the terminal font BIGGER: the chord took the physical key
#      (keycode.rs) and the command key read it as Cmd+= (platform_keys.rs);
#      3b: plain Ctrl+C reaches the shell, Alt+Shift+D is readline's
#      kill-word (why split down is not on it), Ctrl+Shift+T makes a card,
#      Alt+2 is the workspace chord;
#   4. a right-click on the card opens the drawn menu, and choosing Close
#      from it by keyboard closes the card;
#   5. a drag on the bare title bar moves the window and a double-click
#      maximises it;
#   6. under xfwm4 (client-side decorations), the drawn frame's buttons
#      maximise, restore and close, and its corner resizes the window;
#   7. with keyboard.commandModifier = super, Super + physical Equal on
#      Turkish Q is font bigger again (the Mac's chords, nothing rewritten).
# The terminal font being monospace and the Nerd Font icons drawing as icons
# are in screenshot 2, by eye.
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
# Nerd Font icons from the bundled font (folder, git branch, powerline
# arrow), for screenshot 2: drawn as icons, not boxes, by eye.
r ift send 2 "printf 'icons: \\uf07c \\ue0a0 \\ue0b0\\n'" --enter >/dev/null
sleep 1
"$BOX" shot 2-terminal >/dev/null
echo "ok 2: a terminal card runs a shell"

# 3. The physical key on Turkish Q, with the command key (platform_keys.rs):
# Ctrl + the key US prints `=` on is Cmd+= (font bigger); gpui's layout name
# (`-`) would have made it smaller.
r layout tr
r chord ctrl minus
sleep 1
settings=$(r settings)
r layout us
echo "$settings" | grep -Eq '"terminal.fontSize": *15' ||
    fail "Ctrl + physical Equal on Turkish Q did not make the font bigger: $settings"
r logged '\[platform\] ctrl\+= -> cmd\+=' || fail "the chord was not read as cmd+="
"$BOX" shot 3-turkish-bigger >/dev/null
echo "ok 3: a Turkish Q chord took the physical key"

# 3b. The command key with a terminal focused (card 2): plain Ctrl stays the
# terminal's, Ctrl+Shift is the app's, Alt+digit is a workspace.
r ift send 2 'sleep 100' --enter >/dev/null
sleep 0.5
r chord ctrl c
r ift send 2 'echo interrupted-ok' --enter >/dev/null
sleep 1
r ift read 2 | grep -q '^interrupted-ok' || fail "plain Ctrl+C did not reach the shell: $(r ift read 2 | tail -3)"
# Split down is NOT Alt+Shift+D because a terminal uses it: readline runs
# Meta-D as Meta-d, kill-word.
r type 'foo bar'
r chord ctrl a
r chord alt shift d
sleep 0.5
line=$(r ift read 2 | grep -v '^$' | tail -1)
r chord ctrl u
echo "$line" | grep -q '#  bar$' || fail "Alt+Shift+D was not kill-word in bash: $line"
cards_before=$(r ift ls | wc -l)
r chord ctrl shift t
sleep 1
cards_after=$(r ift ls | wc -l)
[ "$cards_after" -eq $((cards_before + 1)) ] || fail "Ctrl+Shift+T made no card ($cards_before, then $cards_after)"
r chord alt 2
r logged '\[platform\] alt\+2 -> ctrl\+2' || fail "Alt+2 was not read as the workspace chord"
r chord alt 1
# Shifted punctuation: gpui reports Ctrl+Shift+[ as `{`, which would read
# back as Shift and pick workspace.prev (Cmd+Shift+[) instead of focus.prev.
# Ctrl+Shift+] then moves the focus back, and is itself shifted punctuation.
r chord ctrl shift bracketleft
r logged '\[chord\] cmd\+\[ \(' || fail "Ctrl+Shift+[ was not read as cmd+["
r chord ctrl shift bracketright
r logged '\[chord\] cmd\+\] \(' || fail "Ctrl+Shift+] was not read as cmd+]"
# Ctrl+1 is Cmd+1, fit the focused card: the menu check below right-clicks
# the middle of the window and needs a card there.
r chord ctrl 1
r logged '\[chord\] cmd\+1 \(' || fail "Ctrl+1 was not read as cmd+1"
sleep 1
echo "ok 3b: plain Ctrl+C is the shell's, Alt+Shift+D is kill-word, Ctrl+Shift+T makes a card, Alt+2 is a workspace"

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
# From a known place, so the drag has room on every side; the double-click
# goes where the window WAS measured after the drag, not where it should be.
r move 200 100
read -r wx wy ww _ <<<"$(r win)"
bx=$((wx + ww - 300)) by=$((wy + 15))
r drag "$bx" "$by" $((bx - 100)) $((by + 80))
read -r mx my _ _ <<<"$(r win)"
[ $((wx - mx)) -ge 80 ] && [ $((my - wy)) -ge 60 ] ||
    fail "a drag on the title bar did not move the window: $wx,$wy then $mx,$my"
r dblclick $((mx + ww - 300)) $((my + 15))
read -r _ _ zw _ <<<"$(r win)"
[ "$zw" -ge 1900 ] || fail "a double-click on the title bar did not maximise the window (width $zw)"
"$BOX" shot 5-maximised >/dev/null
r dblclick 960 15
echo "ok 5: the title bar moves and maximises the window"

# 6. The frame the app draws (window_frame.rs), where gpui gives it
# client-side decorations: xfwm4 with its compositor. The buttons sit at the
# bar's right end, close last; their centres are measured from the window's
# right edge (8 px of padding, 22 px circles 6 px apart).
r stop >/dev/null
r wm xfwm4
r start >/dev/null
read -r wx wy ww wh <<<"$(r win)"
r click 1 $((wx + ww - 47)) $((wy + 21))
read -r _ _ mw _ <<<"$(r win)"
[ "$mw" -ge 1900 ] || fail "the maximise button did not maximise the window (width $mw)"
"$BOX" shot 6-frame-maximised >/dev/null
read -r mx my mw _ <<<"$(r win)"
r click 1 $((mx + mw - 47)) $((my + 21))
read -r wx wy ww wh <<<"$(r win)"
[ "$ww" -lt 1900 ] || fail "the restore button did not restore the window (width $ww)"
r drag $((wx + ww - 2)) $((wy + wh - 2)) $((wx + ww - 302)) $((wy + wh - 202))
read -r _ _ rw rh <<<"$(r win)"
[ $((ww - rw)) -ge 250 ] && [ $((wh - rh)) -ge 150 ] ||
    fail "a drag on the bottom-right corner did not resize the window: ${ww}x$wh then ${rw}x$rh"
"$BOX" shot 6-frame >/dev/null
read -r wx wy ww _ <<<"$(r win)"
r click 1 $((wx + ww - 19)) $((wy + 21))
sleep 2
[ -z "$(r pid)" ] || fail "the close button did not quit the app"
r wm openbox
echo "ok 6: the drawn frame maximises, restores, resizes and closes"

# 7. keyboard.commandModifier = super: the Mac's chords with Super for Cmd,
# nothing rewritten, so Super + physical Equal on Turkish Q is font bigger.
r setting '{"terminal.fontSize": 14, "keyboard.commandModifier": "super"}' fresh
r start >/dev/null
r layout tr
r chord super minus
sleep 1
settings=$(r settings)
r layout us
echo "$settings" | grep -Eq '"terminal.fontSize": *15' ||
    fail "with super, Super + physical Equal did not make the font bigger: $settings"
r stop >/dev/null
echo "ok 7: keyboard.commandModifier super keeps the Mac's chords on Super"

echo "smoke passed; screenshots in ${SHOTS:-.scratchpad/linux-shots}"
