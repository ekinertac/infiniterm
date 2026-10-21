#!/bin/sh
# Restart every Claude Code card in this infiniterm with `claude --resume`,
# after changing something Claude only reads at start (a mod, a plugin, a
# setting). Needs an `ift` newer than 0.5.2: `ls --agents`, `send` and `read`.
#
# For each Claude card: one Ctrl+C (it clears a line left in the prompt, or
# arms Claude's own exit), then `/exit` and Enter. Claude prints what to resume
# on its way out, and that, exactly as printed (not the id saved on the card, which a
# /clear or a fork can leave behind, and a UUID or a quoted session name), goes
# into `claude --resume <that>` typed into the same card, so the shell is still
# in the same directory. An unsent line in the prompt is lost.
#
#   tools/restart-claude.sh [--dry-run] [--include-working] [--only 3,7]
#                           [--cmd 'claude --resume {id}'] [--wait SECONDS]
#                           [--shutdown-wait SECONDS]
#
# Skips: the card this runs in (it would end its own session), a card whose
# Claude is mid-turn (--include-working restarts it anyway and loses the turn),
# a card with no session id yet, a card where nothing is running or something
# other than Claude is (the app lists a Pi card as Claude, since Pi's adapter
# speaks Claude's events), and a card whose Claude does not print its hint
# within --wait seconds (default 20) or does not finish leaving within
# --shutdown-wait (default 60: a big session can take a while). Those are
# named, not touched. Exit status is 1 when a Claude would not leave, else 0.
#
# Related: infiniterm-core/src/ift.rs (the verbs), tools/drive/restart-claude.sh
# (the check, against a fake claude).
set -eu

IFT=${IFT:-ift}
DRY=0
FORCE=0
ONLY=""
TEMPLATE='claude --resume {id}'
WAIT=20
SHUTDOWN_WAIT=60

while [ $# -gt 0 ]; do
    case $1 in
        --dry-run) DRY=1 ;;
        --include-working) FORCE=1 ;;
        --only) shift; ONLY=${1:?--only takes card numbers, like 3,7} ;;
        --cmd) shift; TEMPLATE=${1:?--cmd takes a command template (see the header)} ;;
        --wait) shift; WAIT=${1:?--wait takes seconds} ;;
        --shutdown-wait) shift; SHUTDOWN_WAIT=${1:?--shutdown-wait takes seconds} ;;
        -h|--help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "restart-claude: unknown option $1" >&2; exit 2 ;;
    esac
    shift
done

TAB=$(printf '\t')
# The number of the card this runs in, so it is never restarted from inside.
SELF=""
if [ -n "${INFINITERM_CARD_ID:-}" ]; then
    SELF=$("$IFT" ls | awk -F'\t' -v id="$INFINITERM_CARD_ID" '$1 == id { sub(/^#/, "", $6); print $6 }')
fi

wanted() {
    [ -z "$ONLY" ] && return 0
    case ",$ONLY," in *",$1,"*) return 0 ;; esac
    return 1
}

# The shell pid of card $1, from `ift sessions`.
shell_pid() {
    "$IFT" sessions | awk -F'\t' -v n="#$1" '$6 == n { print $2 }' | head -1
}

# How many resume hints the card's history holds. A restarted session keeps
# its id, so the hint after leaving looks like the one before: the count
# going up is what says Claude has just left.
hints() {
    "$IFT" read "$1" --all | grep -c -- '--resume' || true
}

failed=0
list=$(mktemp)
trap 'rm -f "$list"' EXIT
"$IFT" ls --agents > "$list"
# Read from a file, not a pipe: a pipe runs the loop in a subshell and `failed`
# would not survive it.
while IFS=$TAB read -r number kind session state cwd; do
    [ -n "$number" ] || continue
    [ "$kind" = claude ] || continue
    wanted "$number" || continue
    if [ "$number" = "$SELF" ]; then
        echo "#$number: this card, left alone"
        continue
    fi
    if [ "$session" = "-" ]; then
        echo "#$number: no session id yet, left alone"
        continue
    fi
    if [ "$state" = working ] && [ "$FORCE" = 0 ]; then
        echo "#$number: mid-turn, left alone (--include-working to restart it)"
        continue
    fi
    if [ "$DRY" = 1 ]; then
        echo "#$number: would restart session $session in $cwd"
        continue
    fi
    # What runs in the card: a child of its shell. Nothing, or not Claude, and
    # the Ctrl+C would land on a prompt or on another program.
    spid=$(shell_pid "$number")
    child=""
    [ -n "$spid" ] && child=$(pgrep -P "$spid" | head -1)
    if [ -z "$child" ]; then
        echo "#$number: nothing is running in it, left alone"
        continue
    fi
    running=$(ps -o command= -p "$child" 2>/dev/null)
    case $running in
        *claude*) ;;
        *)
            echo "#$number: runs ${running%% *}, not Claude, left alone"
            continue
            ;;
    esac
    before=$(hints "$number")
    # `/exit` leaves in one step whatever the prompt holds, where counting
    # Ctrl+C presses depends on it: a line in the prompt eats the first, and two
    # presses sent too fast count as one. The Ctrl+C only clears that line.
    "$IFT" send "$number" --key ctrl-c
    sleep 0.6
    "$IFT" send "$number" "/exit" --enter
    id=""
    waited=0
    while [ "$waited" -lt "$WAIT" ]; do
        if [ "$(hints "$number")" -gt "$before" ]; then
            # Everything after --resume on the last hint line, as printed.
            id=$("$IFT" read "$number" --all | sed -n 's/.*--resume \(.*[^ ]\) *$/\1/p' | tail -1)
            [ -n "$id" ] && break
        fi
        waited=$((waited + 1))
        sleep 1
    done
    if [ -z "$id" ]; then
        echo "#$number: Claude did not leave within ${WAIT}s, left alone" >&2
        failed=1
        continue
    fi
    # Claude prints the hint and then needs a moment to shut down; text typed
    # in that moment is discarded with it, and the card is left at a prompt
    # with nothing run. So wait until the card's shell has no child left.
    gone=0
    while pgrep -P "$spid" > /dev/null 2>&1; do
        gone=$((gone + 1))
        [ "$gone" -gt $((SHUTDOWN_WAIT * 3)) ] && break
        sleep 0.33
    done
    if pgrep -P "$spid" > /dev/null 2>&1; then
        echo "#$number: Claude is still shutting down after ${SHUTDOWN_WAIT}s, left alone" >&2
        failed=1
        continue
    fi
    # The prompt is drawn a moment after the shell is free.
    sleep 0.5
    # The first {id} in the template, replaced without sed: a session name can
    # hold a slash or an ampersand.
    cmd="${TEMPLATE%%\{id\}*}$id${TEMPLATE#*\{id\}}"
    "$IFT" send "$number" "$cmd" --enter
    echo "#$number: restarted $id"
done < "$list"
exit "$failed"
