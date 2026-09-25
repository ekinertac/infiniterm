#!/bin/sh
# Runs site/public/install.sh (the curl | sh installer) against a scratch
# HOME and a scratch Applications folder, with the real latest release, and
# checks what it did. Never touches /Applications or your ~/.claude.
#
# Downloads the release once (~100 MB) through the installer's own path,
# then reuses that zip via INFINITERM_ZIP for the other cases. Must run
# without a controlling terminal for the "nobody to ask" case, which is how
# an agent runs it; from an interactive terminal that case is skipped.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
script="$here/site/public/install.sh"
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
fails=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fails=$((fails + 1)); fi; }

fresh() {
    rm -rf "$root/home" "$root/apps"
    mkdir -p "$root/home" "$root/apps"
}
run() {
    HOME="$root/home" INFINITERM_APPS_DIR="$root/apps" sh "$script" "$@" </dev/null >"$root/out" 2>&1
}

# 1. From nothing, over the network, nobody to ask.
fresh
if (: </dev/tty) 2>/dev/null; then
    echo "skip nobody-to-ask case: this shell has a terminal"
    run --no-hooks
else
    run
    check "no terminal: hooks skipped and the commands printed" 'grep -q "ift install-claude-hooks" "$root/out"'
    check "no terminal: ~/.claude left alone" '[ ! -e "$root/home/.claude/settings.json" ]'
fi
check "app installed" '[ -x "$root/apps/infiniterm.app/Contents/MacOS/ift" ]'
check "ift linked" '[ -L "$root/home/.local/bin/ift" ]'
ditto -c -k --keepParent "$root/apps/infiniterm.app" "$root/cached.zip"

# 2. --hooks claude wires Claude, no questions.
fresh
INFINITERM_ZIP="$root/cached.zip" HOME="$root/home" INFINITERM_APPS_DIR="$root/apps" \
    sh "$script" --hooks claude </dev/null >"$root/out" 2>&1
check "--hooks claude writes the hook" 'grep -q infiniterm "$root/home/.claude/settings.json"'

# 3. A second run leaves the installed app to its own updater.
touch "$root/apps/infiniterm.app/marker"
INFINITERM_ZIP="$root/cached.zip" run --no-hooks
check "existing app left as it is" '[ -e "$root/apps/infiniterm.app/marker" ] && grep -q "updates itself" "$root/out"'

# 4. A zip that is not ours is refused before anything is copied.
fresh
mkdir -p "$root/fake/infiniterm.app/Contents/MacOS"
ditto -c -k --keepParent "$root/fake/infiniterm.app" "$root/fake.zip"
set +e; INFINITERM_ZIP="$root/fake.zip" run --no-hooks; code=$?; set -e
check "unsigned zip refused with exit 1" '[ "$code" = 1 ] && [ ! -e "$root/apps/infiniterm.app" ]'

# 5. Bad usage is exit 2.
set +e; run --hooks codex; code=$?; set -e
check "unknown hook is exit 2" '[ "$code" = 2 ]'

[ "$fails" = 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }
