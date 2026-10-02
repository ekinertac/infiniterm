#!/bin/sh
# Checks tools/bump-cask.sh against the cask as it is published in
# ekinertac/homebrew-tap, in DRY_RUN mode, so nothing is pushed: bumping to
# a different release changes exactly the version and sha256 lines, and bad
# usage is exit 2. Needs gh and network (it reads the live cask).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fails=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fails=$((fails + 1)); fi; }

gh api repos/ekinertac/homebrew-tap/contents/Casks/infiniterm.rb --jq .content | base64 -d > "$tmp/live.rb"
printf 'not really a dmg\n' > "$tmp/fake.dmg"
sha=$(shasum -a 256 "$tmp/fake.dmg" | cut -d' ' -f1)

DRY_RUN=1 "$here/bump-cask.sh" 9.8.7 999 "$tmp/fake.dmg" > "$tmp/new.rb"
check "version line bumped" 'grep -q "^  version \"9.8.7,999\"$" "$tmp/new.rb"'
check "sha256 line is the dmg's" 'grep -q "^  sha256 \"$sha\"$" "$tmp/new.rb"'
check "the url downloads from the semver tag" 'grep -q "/download/v#{version.csv.first}/infiniterm-#{version.csv.first}-#{version.csv.second}-arm64.dmg" "$tmp/new.rb"'
check "the cask downloads and checks from the main repo" '! grep -q "infiniterm-releases/" "$tmp/new.rb" && grep -q "github.com/ekinertac/infiniterm/releases/latest/download/latest.json" "$tmp/new.rb"'
# The version and sha256 lines always; the url, livecheck and comment lines
# only until the tap carries the main repo's addresses.
check "nothing else changed" 'n=$(diff "$tmp/live.rb" "$tmp/new.rb" | grep -c "^[<>]"); [ "$n" -le 10 ] && [ "$n" -ge 4 ]'

set +e
"$here/bump-cask.sh" 1 2 >/dev/null 2>&1; code=$?
"$here/bump-cask.sh" 1 2 "$tmp/missing.dmg" >/dev/null 2>&1; code2=$?
set -e
check "wrong argument count is exit 2" '[ "$code" = 2 ]'
check "missing dmg is exit 2" '[ "$code2" = 2 ]'

[ "$fails" = 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }
