#!/bin/sh
# The screencast: a dev builds a small currency converter with two Claude
# sessions on the canvas. Unlike every other scenario here this one is not a
# test — it is the take, so the pacing is built for a camera: long holds
# before and after every move, one act at a time, and a printed timeline
# (elapsed seconds against act) that the edit can be cut from.
#
# Two modes, because a canvas full of work cannot be built on camera:
#
#   tools/drive/cast.sh seed    writes the demo repo, serves it, builds the
#                               canvas with ift and the palette, quits. Run
#                               this once; it leaves workspace.json behind.
#   tools/drive/cast.sh         the take, on the canvas seed left.
#
# The story only works if the agent beats are real: the demo repo carries its
# own .claude/settings.json pointing at the dev hook binary, so a Claude
# session inside a card drives that card's state the way it does in real use,
# and `wait_state` below polls `ift ls` for the change instead of guessing at
# a sleep. FAKE=1 skips Claude and flips the states with the hook binary
# directly — same beats, no tokens, for rehearsing the timing.
#
# Everything lives under $CAST_DIR, never the real data dir: the recording
# must not show Ekin's directories and must not write his canvas.
CAST_DIR=${CAST_DIR:-/tmp/infiniterm-cast}
APP_DIR=$CAST_DIR/converter
PORT=${PORT:-8087}
# The window frame the take is recorded at, seeded so two runs frame
# identically. 16:9 leaves no letterboxing when the edit goes to 1080p.
CAST_W=${CAST_W:-1920}
CAST_H=${CAST_H:-1080}
export INFINITERM_DATA_DIR="$CAST_DIR/data"
SHOTS=${SHOTS:-$CAST_DIR/shots}
KEEP=1
. "$(dirname "$0")/lib.sh"

IFT="$ROOT/target/debug/ift"
HOOK="$ROOT/target/debug/infiniterm-hook"

# --- the camera's clock ---------------------------------------------------
T0=$(date +%s)
mark() { printf '%4ds  %s\n' "$(($(date +%s) - T0))" "$1"; }
# A move needs air on both sides or the edit has nothing to cut on.
hold() { sleep "${1:-1.5}"; }
# Typing a whole string arrives as one event and reads as a paste on camera.
slow() {
    i=1
    while [ "$i" -le "${#1}" ]; do
        osascript -e "$(_se) to keystroke \"$(printf %s "$1" | cut -c"$i")\""
        sleep 0.06
        i=$((i + 1))
    done
}
enter() { key_code 36; }
esc() { key_code 53; }
# Arrow key codes, named so the acts read as directions.
LEFT=123
RIGHT=124
DOWN=125
UP=126

# The palette by label, never by key name: "browser open a url" matches
# "Browser: open a URL". A query that matches the wrong command first is the
# trap this file inherits from the test scenarios.
palette() {
    cmd_shift p
    hold 0.6
    slow "$1"
    hold 0.8
    enter
}

# --- reading the app back -------------------------------------------------
# `ift ls` is TSV: id, group, directory, state, remote.
card_in() { "$IFT" ls | awk -F'\t' -v d="$1" '$3 ~ d {print $1; exit}'; }
# The card a command just made. Every card in the demo shares one directory,
# so the only way to name the new one is the id that was not there before.
cards_now() { "$IFT" ls | cut -f1 | sort; }
new_card() {
    before=$(cards_now)
    "$@" >/dev/null 2>&1
    n=0
    while [ "$n" -lt 30 ]; do
        id=$(cards_now | grep -vxF "$before" | head -1)
        [ -n "$id" ] && {
            echo "$id"
            return 0
        }
        sleep 0.5
        n=$((n + 1))
    done
    echo "warn: no new card appeared" >&2
}
# Polls until a card reports the state, so the idle beat is cut on the real
# transition rather than on a sleep that guessed wrong.
wait_state() {
    n=0
    while [ "$n" -lt "${3:-180}" ]; do
        [ "$("$IFT" ls | awk -F'\t' -v i="$1" '$1==i {print $4}')" = "$2" ] && return 0
        sleep 1
        n=$((n + 1))
    done
    echo "warn: card $1 never reached $2" >&2
}

# --- seeding --------------------------------------------------------------
seed_repo() {
    rm -rf "$APP_DIR"
    mkdir -p "$APP_DIR"
    cat >"$APP_DIR/index.html" <<'HTML'
<!doctype html>
<meta charset="utf-8">
<title>try/eur</title>
<link rel="stylesheet" href="style.css">
<main>
  <h1>TRY to EUR</h1>
  <input id="amount" value="1000" inputmode="decimal">
  <output id="out">…</output>
  <p id="asof"></p>
</main>
<script type="module" src="app.js"></script>
HTML
    cat >"$APP_DIR/app.js" <<'JS'
// Rates from Frankfurter (ECB reference rates, no key, daily).
const API = "https://api.frankfurter.dev/v1/latest?base=TRY&symbols=EUR";

async function rate() {
  const r = await fetch(API);
  const j = await r.json();
  return { value: j.rates.EUR, date: j.date };
}

function render(amount, { value, date }) {
  out.value = (amount * value).toFixed(2) + " EUR";
  asof.textContent = "ECB rate for " + date;
}

const live = await rate();
render(Number(amount.value), live);
amount.addEventListener("input", () => render(Number(amount.value), live));
JS
    cat >"$APP_DIR/style.css" <<'CSS'
body { font: 16px/1.5 ui-sans-serif, system-ui; margin: 0; display: grid; place-items: center; height: 100vh; background: #14161a; color: #e6e6e6; }
main { width: 22rem; }
input { font: inherit; width: 100%; padding: .5rem; background: #1d2026; color: inherit; border: 1px solid #2c313a; border-radius: 6px; }
output { display: block; font-size: 2rem; margin: .75rem 0 0; }
p { color: #8b939f; font-size: .8rem; }
CSS
    cat >"$APP_DIR/README.md" <<'MD'
# try/eur

A one-page converter over the ECB's daily reference rates.

    python3 -m http.server 8087
MD
    (
        cd "$APP_DIR"
        git init -q
        git add -A
        git -c user.name=dev -c user.email=dev@localhost commit -qm "the converter, one page"
    )
    # An uncommitted change so the diff card has something to show in act 6.
    printf '\n// TODO: remember the last amount across reloads\n' >>"$APP_DIR/app.js"
}

# The hooks the agent beats depend on, project-local so ~/.claude is untouched.
seed_hooks() {
    mkdir -p "$APP_DIR/.claude"
    python3 - "$APP_DIR/.claude/settings.json" "$HOOK" <<'PY'
import json, sys
path, hook = sys.argv[1], sys.argv[2]
events = ["UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop",
          "StopFailure", "Notification", "SessionStart", "SessionEnd"]
json.dump({"hooks": {e: [{"hooks": [{"type": "command",
                                     "command": f"{hook} {e}"}]}] for e in events}},
          open(path, "w"), indent=2)
PY
}

seed_window() {
    mkdir -p "$CAST_DIR/data"
    printf '{"x":100.0,"y":100.0,"w":%s.0,"h":%s.0,"mode":"windowed"}\n' \
        "$CAST_W" "$CAST_H" >"$CAST_DIR/data/window.json"
}

# The canvas the take opens on: seven cards with real scrollback in them, the
# way a canvas looks an hour into a day. The take adds the rest on camera.
seed_canvas() {
    FRESH=1 drive_start
    # 1: the hero card, the one act 1 sits inside.
    slow "cd $APP_DIR && git log --oneline"
    enter
    hold 1
    slow "ls -la"
    enter
    hold 1
    # 2: the dev server, so the browser card's requests show up as log lines.
    cmd t
    hold 1
    slow "cd $APP_DIR && python3 -m http.server $PORT"
    enter
    hold 1.5
    # 3: a shell with room to work in.
    cmd t
    hold 1
    slow "cd $APP_DIR && git status"
    enter
    hold 1
    # 4 and 5: the editor and the diff, opened the way they are in real use.
    "$IFT" "$APP_DIR/app.js" >/dev/null
    hold 1
    INFINITERM_CARD_ID=$(card_in "converter") "$IFT" diff "$APP_DIR" >/dev/null
    hold 1.5
    # 6: the page itself.
    palette "browser open a url"
    hold 1
    slow "http://localhost:$PORT"
    enter
    hold 3
    # 7: somewhere else entirely, so the canvas is not one project deep.
    cmd t
    hold 1
    slow "cd $APP_DIR && git log --stat -1"
    enter
    hold 1
    # Every card here is in one directory, so the derived labels are seven
    # copies of the same path. Names make the canvas readable zoomed out and
    # give the take somewhere to navigate TO.
    i=1
    for n in converter server shell app.js diff page log; do
        id=$("$IFT" ls | sed -n "${i}p" | cut -f1)
        INFINITERM_CARD_ID=$id "$IFT" name "$n" >/dev/null
        i=$((i + 1))
    done
    hold 1
    cmd 2
    hold 1
    shot seed-canvas 1
    # The take opens on the hero card: from the last card made, left along the
    # bottom row and up into the corner.
    cmd_alt $LEFT
    cmd_alt $LEFT
    cmd_alt $LEFT
    cmd_alt $UP
    hold 1
    quit
    drive_stop
}

# --- the take -------------------------------------------------------------
take() {
    drive_start
    # ACT 0 — the dev server, in the card seeded for it. A restored card is a
    # fresh shell in its directory, so the command has to be typed again; it
    # doubles as the thing any dev does first.
    mark "act 0: the server"
    cmd_alt $RIGHT
    hold 1
    slow "python3 -m http.server $PORT"
    enter
    hold 1.5
    cmd_alt $LEFT
    hold 1

    # ACT 1 — one card. At actual size the card is wider than the window, so
    # nothing else is on screen and this reads as an ordinary terminal.
    mark "act 1: one card"
    cmd 0
    hold 2
    shot 01-one-card 1
    slow "curl -s 'https://api.frankfurter.dev/v1/latest?base=TRY&symbols=EUR'"
    enter
    hold 4

    # ACT 2 — the reveal. The lie in act 1 is the scale, and this is where it
    # is paid off, so it gets the longest hold in the file.
    mark "act 2: cmd+2, the reveal"
    cmd 2
    hold 5
    shot 02-reveal 1

    # ACT 3 — moving. The same two keys at two zooms is the point: nothing
    # re-tiles, the view goes to the card.
    mark "act 3: moving, zoomed out then in"
    cmd_alt $RIGHT
    hold 1.2
    cmd_alt $DOWN
    hold 1.2
    cmd_alt $LEFT
    hold 1.5
    cmd 1
    hold 2
    cmd_alt $RIGHT
    hold 2
    cmd_alt $LEFT
    hold 2
    shot 03-fit-card 1
    cmd f
    hold 2
    esc
    hold 1
    key_code 36 "command down, shift down"
    hold 2.5
    key_code 36 "command down, shift down"
    hold 1.5

    # ACT 4 — cards where you are looking. The phantom is the idea: the slot
    # is chosen by walking to it, not by a dialog.
    mark "act 4: phantom slots and splits"
    cmd 2
    hold 1
    cmd_alt $RIGHT
    cmd_alt $RIGHT
    hold 1.5
    shot 04-phantom 1
    cmd t
    hold 2
    slow "cd $APP_DIR && git diff --stat"
    enter
    hold 2
    cmd_shift t
    hold 1.5
    shot 05-placement 1
    esc
    hold 1
    cmd d
    hold 2
    cmd g
    hold 1.5
    cmd 3
    hold 2
    shot 06-group 1
    cmd 2
    hold 1.5

    # ACT 5 — the agents. The longest act; everything before it is setup.
    mark "act 5: two agents"
    HERO=$(card_in "converter")
    CH=$(new_card palette "claude code in this directory")
    hold 6
    if [ "${FAKE:-}" = 1 ]; then
        echo '{"transcript_path":"/tmp/cast.jsonl"}' | INFINITERM_CARD_ID=$CH "$HOOK" UserPromptSubmit
    else
        slow "add a 30 day rate chart under the amount, canvas only, no libraries"
        hold 1
        enter
    fi
    hold 3
    shot 07-first-agent 1
    # Leaving a working agent is the whole argument: the second one starts
    # while the first is still going.
    cmd 2
    hold 1.5
    cmd_alt $DOWN
    hold 1
    CT=$(new_card palette "claude code in this directory")
    hold 6
    if [ "${FAKE:-}" = 1 ]; then
        echo '{"transcript_path":"/tmp/cast.jsonl"}' | INFINITERM_CARD_ID=$CT "$HOOK" UserPromptSubmit
    else
        slow "write a test for the rate cache, plain node, no framework"
        hold 1
        enter
    fi
    hold 2
    cmd 2
    hold 4
    shot 08-both-working 1
    mark "act 5: waiting for the first agent to go idle"
    if [ "${FAKE:-}" = 1 ]; then
        sleep 8
        echo '{}' | INFINITERM_CARD_ID=$CH "$HOOK" Stop
    else
        wait_state "$CH" idle
    fi
    hold 3
    shot 09-idle-signal 1
    # Straight to the card that asked, from wherever the view was.
    cmd_alt $UP
    hold 1
    cmd 1
    hold 3
    cmd i
    hold 3
    shot 10-transcript 1
    cmd w
    hold 1.5

    # ACT 6 — reading the work.
    mark "act 6: diff, editor, page"
    cmd 2
    hold 1.5
    INFINITERM_CARD_ID=$HERO "$IFT" diff "$APP_DIR" >/dev/null
    hold 2
    cmd 1
    hold 2
    cmd b
    hold 3
    shot 11-blame 1
    cmd 2
    hold 1
    cmd_alt $RIGHT
    cmd 1
    hold 2
    key_code 42 "command down"
    hold 2
    shot 12-sidebar 1
    cmd 2
    hold 1.5

    # ACT 7 — making it yours. The theme preview repaints every card as the
    # selection moves, which only means anything zoomed out.
    mark "act 7: theme, interface scale, settings"
    palette "theme switch"
    hold 1.5
    for _ in 1 2 3 4 5 6; do
        key_code $DOWN
        sleep 0.9
    done
    shot 13-theme-preview 1
    enter
    hold 2.5
    key_code 24 "command down, shift down"
    key_code 24 "command down, shift down"
    hold 2
    shot 14-ui-bigger 1
    key_code 29 "command down, shift down"
    hold 1.5
    key "," "command down"
    hold 3
    shot 15-settings 1
    cmd w
    hold 1
    key "/" "command down"
    hold 3
    shot 16-shortcuts 1
    esc
    hold 1.5

    # ACT 8 — close. Two workspaces, then the whole canvas held long enough
    # to end on.
    mark "act 8: workspaces and the hold"
    key "2" "control down"
    hold 2.5
    key "1" "control down"
    hold 2
    cmd 2
    hold 6
    shot 17-final 1
    mark "done"
    drive_log
}

case "${1:-take}" in
# The demo repo alone: a take that dirtied it (an agent really does edit
# app.js) is re-armed with this instead of a full reseed.
repo)
    seed_repo
    seed_hooks
    echo "demo repo at $APP_DIR"
    ;;
seed)
    seed_repo
    seed_hooks
    seed_window
    seed_canvas
    echo "seeded $CAST_DIR; run tools/drive/cast.sh for the take"
    ;;
take)
    [ -f "$CAST_DIR/data/workspace.json" ] || {
        echo "no seeded canvas: run tools/drive/cast.sh seed first" >&2
        exit 1
    }
    take
    ;;
*)
    echo "usage: cast.sh [seed|take]" >&2
    exit 2
    ;;
esac
