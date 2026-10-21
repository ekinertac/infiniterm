#!/bin/sh
# Replays the updater's checks (infiniterm-ui/src/updater.rs `check` and
# `stage`, infiniterm-core/src/update.rs) against a published feed, the way
# an installed copy would run them, so a release is known to update before
# anyone's copy tries: the feed parses and names the expected build, its
# zip comes from an allowed address, matches its checksum, unpacks, is
# signed by the team requirement, is notarized, and carries that build.
#
#   tools/check-update-feed.sh <latest.json url> [expected build]
#
# Called by tools/publish.sh for every feed it writes. Exit 0 when an
# installed copy would stage the update, 1 with the reason when not.
set -eu
[ $# -ge 1 ] || { echo "usage: check-update-feed.sh <latest.json url> [build]" >&2; exit 2; }
feed=$1 want=${2:-}
team='anchor apple generic and certificate leaf[subject.OU] = "QKN7RYV5PD"'
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail() { echo "check-update-feed: $feed: $*" >&2; exit 1; }
# GitHub's download redirect can lag a just-created release for a moment.
for i in 1 2 3 4 5 6; do
    curl -fsSL --max-time 20 "$feed" > "$tmp/latest.json" 2>/dev/null && break
    sleep 5
done
[ -s "$tmp/latest.json" ] || fail "no latest.json"
field() { sed -n "s/^ *\"$1\": \"\{0,1\}\([^\",]*\)\"\{0,1\},\{0,1\}$/\1/p" "$tmp/latest.json"; }
build=$(field build) url=$(field url) sha=$(field sha256)
[ -z "$want" ] || [ "$build" = "$want" ] || fail "names build $build, expected $want"
case "$url" in
    https://github.com/ekinertac/infiniterm/releases/download/*) ;;
    https://github.com/ekinertac/infiniterm-releases/releases/download/*) ;;
    *) fail "points outside the release downloads: $url" ;;
esac
# A feed served from the old repo must point into it: builds up to 0.4.0
# accept only that address.
case "$feed" in
    *infiniterm-releases/*) case "$url" in *infiniterm-releases/*) ;; *) fail "old builds would refuse $url" ;; esac ;;
esac
curl -fsSL --max-time 900 -o "$tmp/app.zip" "$url" || fail "cannot download $url"
[ "$(shasum -a 256 "$tmp/app.zip" | cut -d' ' -f1)" = "$sha" ] || fail "the zip does not match its checksum"
ditto -x -k "$tmp/app.zip" "$tmp/out" || fail "the zip does not unpack"
app="$tmp/out/infiniterm.app"
codesign --verify --deep --strict "-R=$team" "$app" 2>/dev/null || fail "not signed by infiniterm's developer"
spctl -a -t exec "$app" 2>/dev/null || fail "not notarized"
got=$(/usr/libexec/PlistBuddy -c "Print :CFBundleVersion" "$app/Contents/Info.plist")
[ "$got" = "$build" ] || fail "the app is build $got, the feed says $build"
echo "check-update-feed: $feed: build $build would update (download, checksum, signature, notarization, build all pass)"
