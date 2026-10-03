#!/bin/sh
# Restart every Claude Code card in this infiniterm with `claude --resume`,
# after changing something Claude only reads at start (a mod, a plugin, a
# setting). Needs an `ift` newer than 0.5.2: `ls --agents`, `send` and `read`.
#
# For each Claude card: Ctrl+C makes Claude leave (up to three: a prompt with
# unsent text uses the first to clear itself), it prints what to resume on its
# way out, and that, exactly as printed (not the id saved on the card, which a
# /clear or a fork can leave behind, and a UUID or a quoted session name), goes
# into `claude --resume <that>` typed into the same card, so the shell is still
# in the same directory. An unsent line in the prompt is lost.
#
#   tools/restart-claude.sh [--dry-run] [--include-working] [--only 3,7]
#                           [--cmd 'claude --resume {id}'] [--wait SECONDS]
#
# Skips: the card this runs in (it would end its own session), a card whose
# Claude is mid-turn (--include-working restarts it anyway and loses the turn),
# a card with no session id yet, and a card whose Claude does not leave within
# --wait seconds (default 20). Those are named, not touched. Exit status is 1
# when a Claude would not leave, else 0.
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

while [ $# -gt 0 ]; do
    case $1 in
        --dry-run) DRY=1 ;;
        --include-working) FORCE=1 ;;
        --only) shift; ONLY=${1:?--only takes card numbers, like 3,7} ;;
        --cmd) shift; TEMPLATE=${1:?--cmd takes a command template (see the header)} ;;
        --wait) shift; WAIT=${1:?--wait takes seconds} ;;
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
    before=$(hints "$number")
    presses=0
    while [ "$presses" -lt 3 ]; do
        "$IFT" send "$number" --key ctrl-c
        presses=$((presses + 1))
        sleep 0.6
        [ "$(hints "$number")" -gt "$before" ] && break
    done
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
    # The shell prompt is back a moment after the hint.
    sleep 0.5
    # The first {id} in the template, replaced without sed: a session name can
    # hold a slash or an ampersand.
    cmd="${TEMPLATE%%\{id\}*}$id${TEMPLATE#*\{id\}}"
    "$IFT" send "$number" "$cmd" --enter
    echo "#$number: restarted $id"
done < "$list"
exit "$failed"
