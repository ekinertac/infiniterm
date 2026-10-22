#!/usr/bin/env bash
# The Linux branch of site/public/install.sh, run for real against a scratch
# HOME, with a tarball built here by tools/linux-dist.sh. Runs ON the Linux
# box, from the checkout box.sh copied there:
#
#   tools/drive/linux/box.sh sync && tools/drive/linux/box.sh build
#   tools/drive/linux/box.sh x tools/drive/linux/install-test.sh
#
# Uses the box's DEBUG build (install.sh does not care how the binaries were
# built; the release build is the CI job's). Checks: the versioned folder and
# `current`; the `infiniterm` and `ift` links through `current`; the desktop
# entry; that a second and a third install keep the two newest versions;
# that the Claude hook command goes through `current`
# (`paths::stable_install_path`); and that the installed app starts, finds
# its themes beside itself and runs a shell in a card. Never touches the
# real HOME. Exit 0 only when every check held.
#
# Related: install.sh, tools/linux-dist.sh, box.sh, remote.sh.
set -euo pipefail

cd "$(dirname "$0")/../../.."
fails=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fails=$((fails + 1)); fi; }

scratch=$(mktemp -d)
trap 'kill "${app_pid:-}" 2>/dev/null || true; rm -rf "$scratch"' EXIT
export HOME="$scratch/home"
mkdir -p "$HOME"
root="$HOME/.local/share/infiniterm"

tools/linux-dist.sh target/debug >/dev/null
tarball=$(ls dist/infiniterm-linux-*.tar.gz)
version=$(tar -xzOf "$tarball" infiniterm/VERSION)

# A copy of the tarball claiming another version, for the update cases.
repack() {
    rm -rf "$scratch/re" && mkdir -p "$scratch/re"
    tar -xzf "$tarball" -C "$scratch/re"
    printf '%s\n' "$1" > "$scratch/re/infiniterm/VERSION"
    tar -C "$scratch/re" -czf "$scratch/v$1.tar.gz" infiniterm
}

# 1. A first install, nobody to ask about hooks.
INFINITERM_TARBALL="$tarball" sh site/public/install.sh --no-hooks </dev/null >"$scratch/out" 2>&1 \
    || { cat "$scratch/out"; exit 1; }
check "the version folder holds the app" '[ -x "$root/$version/infiniterm" ] && [ -x "$root/$version/iftd" ]'
check "themes and backgrounds sit beside it" '[ -d "$root/$version/themes" ] && [ -d "$root/$version/backgrounds" ]'
check "current names the version" '[ "$(readlink "$root/current")" = "$version" ]'
check "infiniterm is linked through current" '[ "$(readlink "$HOME/.local/bin/infiniterm")" = "$root/current/infiniterm" ]'
check "ift is linked through current" '[ "$(readlink "$HOME/.local/bin/ift")" = "$root/current/ift" ]'
check "a desktop entry runs current" 'grep -qx "Exec=$root/current/infiniterm" "$HOME/.local/share/applications/infiniterm.desktop"'

# 2. Two updates: three versions installed, the two newest kept.
repack 99.0.1
INFINITERM_TARBALL="$scratch/v99.0.1.tar.gz" sh site/public/install.sh --no-hooks </dev/null >/dev/null 2>&1
repack 99.0.2
INFINITERM_TARBALL="$scratch/v99.0.2.tar.gz" sh site/public/install.sh --no-hooks </dev/null >/dev/null 2>&1
check "current moved to the newest" '[ "$(readlink "$root/current")" = 99.0.2 ]'
check "the two newest stay" '[ -d "$root/99.0.1" ] && [ -d "$root/99.0.2" ]'
check "the oldest is gone" '[ ! -e "$root/$version" ]'

# 3. The hook command survives the next update.
"$HOME/.local/bin/ift" install-claude-hooks >/dev/null 2>&1
check "the Claude hook goes through current" 'grep -q "$root/current/infiniterm-hook" "$HOME/.claude/settings.json"'

# 4. The installed app starts, finds its themes, runs a shell.
export DISPLAY=${DISPLAY:-:99}
bash tools/drive/linux/remote.sh layout us >/dev/null   # starts Xvfb and openbox
export INFINITERM_DATA_DIR="$scratch/data"
mkdir -p "$INFINITERM_DATA_DIR"
# No PATH help: iftd must be found beside the installed binary.
(cd /tmp && exec setsid "$HOME/.local/bin/infiniterm") </dev/null >"$scratch/app.log" 2>&1 &
app_pid=$!
for _ in $(seq 100); do [ -S "$INFINITERM_DATA_DIR/infiniterm.sock" ] && break; sleep 0.1; done
check "the installed app is up" '[ -S "$INFINITERM_DATA_DIR/infiniterm.sock" ]'
sleep 3
check "it found its themes" '! grep -q "could not load theme" "$scratch/app.log"'
ift="$HOME/.local/bin/ift"
"$ift" run card.new.terminal >/dev/null
for _ in $(seq 30); do "$ift" read 2 >/dev/null 2>&1 && break; sleep 0.5; done
"$ift" send 2 'echo installed-$((6*7))' --enter >/dev/null
sleep 1
check "a card runs a shell under the installed iftd" '"$ift" read 2 | grep -q "^installed-42"'
import -window root /tmp/install-test.png 2>/dev/null || true

[ "$fails" -eq 0 ] && echo "install test passed" || { echo "$fails failed; app log:"; tail -20 "$scratch/app.log"; exit 1; }
