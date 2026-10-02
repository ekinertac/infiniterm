#!/bin/sh
# Points the Homebrew cask (Casks/infiniterm.rb in the public
# ekinertac/homebrew-tap) at a new release: the "version,build" line and the
# DMG's sha256. Called by tools/publish.sh right after the GitHub release is
# created, so `brew install --cask ekinertac/tap/infiniterm` gets the build
# that was just published. The app updates itself (the cask is auto_updates),
# so a stale cask only ever hurt fresh installs, which is why this is a step
# of publishing and not a separate chore to remember.
#
# Edits through the GitHub contents API, one commit, no clone. The cask's
# URL is built from the version; the version and sha256 lines change, and
# the url line is set to the semver tag's download path (#19: releases were
# tagged v<version>-<build> before) and the main repo's releases (#52; the
# old ekinertac/infiniterm-releases before), a no-op once the tap has them.
# Anything
# else in the file is left exactly as the tap has it.
#
#   tools/bump-cask.sh <version> <build> <dmg>
#   DRY_RUN=1 tools/bump-cask.sh ...   print the new cask, change nothing
#
# tools/test-bump-cask.sh checks it against the cask as published.
set -eu
[ $# -eq 3 ] || { echo "usage: bump-cask.sh <version> <build> <dmg>" >&2; exit 2; }
version=$1 build=$2 dmg=$3
[ -f "$dmg" ] || { echo "bump-cask: no such file: $dmg" >&2; exit 2; }
tap=ekinertac/homebrew-tap
path=Casks/infiniterm.rb
sha=$(shasum -a 256 "$dmg" | cut -d' ' -f1)

meta=$(gh api "repos/$tap/contents/$path")
blob=$(printf '%s' "$meta" | python3 -c 'import json,sys; print(json.load(sys.stdin)["sha"])')
old=$(printf '%s' "$meta" | python3 -c 'import json,sys,base64; print(base64.b64decode(json.load(sys.stdin)["content"]).decode(), end="")')
new=$(printf '%s\n' "$old" | sed -E \
    -e "s/^(  version \")[^\"]*(\")/\1$version,$build\2/" \
    -e "s/^(  sha256 \")[0-9a-f]{64}(\")/\1$sha\2/" \
    -e 's|/download/v#{version.csv.first}-#{version.csv.second}/|/download/v#{version.csv.first}/|' \
    -e 's|github.com/ekinertac/infiniterm-releases/|github.com/ekinertac/infiniterm/|g' \
    -e 's|^# The DMG on ekinertac/infiniterm-releases is|# The DMG on ekinertac/infiniterm'"'"'s releases is|' \
    -e 's|^# current for fresh installs. Release tags are v<version>-<build>, so the$|# current for fresh installs. Release tags are v<version> and the DMG names|' \
    -e 's|^# version is "<version>,<build>" and the csv parts rebuild the URL.$|# the build, so the version is "<version>,<build>" and the csv parts rebuild the URL.|')
printf '%s\n' "$new" | grep -q "version \"$version,$build\"" || { echo "bump-cask: no version line to update in $path" >&2; exit 1; }
printf '%s\n' "$new" | grep -q "sha256 \"$sha\"" || { echo "bump-cask: no sha256 line to update in $path" >&2; exit 1; }

if [ -n "${DRY_RUN:-}" ]; then
    printf '%s\n' "$new"
    exit 0
fi
if [ "$new" = "$old" ]; then
    echo "bump-cask: $path already at $version,$build"
    exit 0
fi
printf '%s\n' "$new" | base64 | tr -d '\n' > "${TMPDIR:-/tmp}/cask.b64"
gh api -X PUT "repos/$tap/contents/$path" \
    -f message="infiniterm: $version ($build)" \
    -F content=@"${TMPDIR:-/tmp}/cask.b64" \
    -f sha="$blob" >/dev/null
rm -f "${TMPDIR:-/tmp}/cask.b64"
echo "bump-cask: $tap $path -> $version,$build"
