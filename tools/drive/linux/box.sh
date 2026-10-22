#!/usr/bin/env bash
# The Linux GUI driver's Mac half: a throwaway Linux box with a virtual
# display, this checkout copied onto it, built and driven there, and the
# screenshots brought back. So a Linux change is checked on screen, not
# only compiled (#464).
#
# Verbs:
#   up             creates the droplet $NAME (doctl, context ekinertac) and
#                  installs what the app needs. Prints the address.
#   sync           copies THIS checkout to the box (rsync, so uncommitted
#                  changes go too; target/ and .git stay here).
#   build          builds the app (no browser cards), ift, iftd and the hook.
#   r <verb ...>   runs remote.sh on the box: start, stop, chord, type, ...
#   shot <name>    a screenshot, copied to $SHOTS/<name>.png; prints the path.
#   ssh            an interactive shell on the box.
#   x <cmd ...>    a command in the box's checkout (install-test.sh uses it).
#   down           deletes the droplet and waits until it is gone.
#
# LINUX_HOST=<user@host> drives any machine instead of a droplet (up and down
# then refuse). The droplet's address is kept in $STATE between calls.
#
# doctl's default context is the employer's production account: every doctl
# call here passes --context ekinertac, and the droplet named vpn2 is never
# touched. A droplet costs money while it exists; `down` when done.
#
# Related: remote.sh (the box half), smoke.sh (the scenario),
# docs/agent/linux-port.md (what the spike found, why openbox, why held keys).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
NAME=${NAME:-ift-linux-drive}
STATE=${STATE:-${TMPDIR:-/tmp}/infiniterm-linux-drive.host}
SHOTS=${SHOTS:-$ROOT/.scratchpad/linux-shots}
DOCTL=(doctl --context ekinertac)
# Ekin's key on the DigitalOcean account.
SSH_KEY_ID=${SSH_KEY_ID:-23752258}
SIZE=${SIZE:-s-8vcpu-16gb}
REGION=${REGION:-fra1}
DIR=/root/ift
# Keepalives: the package install and the build are minutes with no output,
# and a connection with nothing on it was dropped once ("Operation timed out").
SSH_OPTS=(-o StrictHostKeyChecking=accept-new -o ConnectTimeout=15 -o ServerAliveInterval=15 -o ServerAliveCountMax=8)

# The packages: most of zed's script/linux (the set for gpui) plus Mesa's
# software Vulkan, the virtual display, a window manager and the tools the
# driver calls. docs/agent/linux-port.md has the reasoning.
PACKAGES="gcc g++ libasound2-dev libfontconfig-dev libglib2.0-dev libssl-dev libvulkan1 libwayland-dev libx11-xcb-dev libxkbcommon-x11-dev libzstd-dev make cmake clang lld llvm git curl pkg-config build-essential rsync lsof mesa-vulkan-drivers vulkan-tools xvfb x11-utils x11-xkb-utils imagemagick xdotool openbox fonts-dejavu-core"

host() {
    if [ -n "${LINUX_HOST:-}" ]; then echo "$LINUX_HOST"; return; fi
    [ -s "$STATE" ] || { echo "no box: run \`$0 up\` or set LINUX_HOST" >&2; exit 1; }
    echo "root@$(cat "$STATE")"
}

on_box() { ssh "${SSH_OPTS[@]}" "$(host)" "$@"; }

# Quotes each argument for the box's shell, so `r type "a b"` stays one word.
quoted() { printf '%q ' "$@"; }

case "${1:-}" in
up)
    [ -z "${LINUX_HOST:-}" ] || { echo "LINUX_HOST is set: nothing to create" >&2; exit 2; }
    [ "$NAME" != vpn2 ] || { echo "refusing the name vpn2" >&2; exit 2; }
    if ! "${DOCTL[@]}" compute droplet list --format Name --no-header | grep -qx "$NAME"; then
        "${DOCTL[@]}" compute droplet create "$NAME" --image ubuntu-24-04-x64 \
            --size "$SIZE" --region "$REGION" --ssh-keys "$SSH_KEY_ID" --wait >/dev/null
    fi
    "${DOCTL[@]}" compute droplet list --format Name,PublicIPv4 --no-header |
        awk -v n="$NAME" '$1 == n { print $2 }' >"$STATE"
    echo "box: $(cat "$STATE")"
    # sshd comes up a little after the droplet reports active.
    for _ in $(seq 30); do on_box true 2>/dev/null && break; sleep 3; done
    on_box "export DEBIAN_FRONTEND=noninteractive
        set -e
        # A fresh droplet runs its own apt at boot; two at once corrupt the
        # cache (seen on the first run).
        cloud-init status --wait >/dev/null 2>&1 || true
        apt-get update -qq && apt-get install -y -qq $PACKAGES >/dev/null
        [ -x ~/.cargo/bin/cargo ] || curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --component clippy >/dev/null
        echo provisioned"
    ;;
sync)
    # Not -a: it keeps each file's time, and a checkout edited before the
    # box's last build looked older than that build, so cargo skipped the
    # rebuild (seen syncing another branch onto a used box). Without -t a
    # copied file gets the time it lands; --checksum leaves files that did
    # not change alone, so their times stay and nothing extra rebuilds.
    rsync -rlpz --checksum --delete --exclude /target --exclude /.git --exclude /.claude \
        --exclude node_modules --exclude /.scratchpad --exclude /site/dist \
        -e "ssh ${SSH_OPTS[*]}" "$ROOT/" "$(host):$DIR/"
    echo "synced to $(host):$DIR"
    ;;
build)
    on_box ". ~/.cargo/env && cd $DIR && cargo build -p infiniterm-ui --no-default-features -p infiniterm-cli -p infiniterm-session -p infiniterm-hook 2>&1 | grep -E '^(error|warning: unused)|Finished' ; exit \${PIPESTATUS[0]}"
    ;;
r)
    shift
    on_box "bash $DIR/tools/drive/linux/remote.sh $(quoted "$@")"
    ;;
shot)
    name=${2:?shot <name>}
    mkdir -p "$SHOTS"
    on_box "bash $DIR/tools/drive/linux/remote.sh shot /tmp/$name.png" >/dev/null
    scp -q "${SSH_OPTS[@]}" "$(host):/tmp/$name.png" "$SHOTS/$name.png"
    echo "$SHOTS/$name.png"
    ;;
ssh)
    ssh "${SSH_OPTS[@]}" "$(host)"
    ;;
x)
    # Any command in the box's checkout, with cargo on PATH.
    shift
    on_box ". ~/.cargo/env && cd $DIR && $(quoted "$@")"
    ;;
down)
    [ -z "${LINUX_HOST:-}" ] || { echo "LINUX_HOST is set: nothing to delete" >&2; exit 2; }
    [ "$NAME" != vpn2 ] || { echo "refusing the name vpn2" >&2; exit 2; }
    "${DOCTL[@]}" compute droplet delete "$NAME" --force
    # delete has no --wait.
    while "${DOCTL[@]}" compute droplet list --format Name --no-header | grep -qx "$NAME"; do sleep 5; done
    rm -f "$STATE"
    echo "deleted $NAME"
    ;;
*)
    # The header comment is the help.
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"
    exit 2
    ;;
esac
