#!/bin/sh
# The build that leaves this Mac: target/dist/ gets the notarized, stapled
# DMG friends install from, the notarized, stapled zip the updater
# downloads (updater.rs), and latest.json, the manifest the updater polls.
# `tools/publish.sh` puts all three on ekinertac/infiniterm (#52; on
# ekinertac/infiniterm-releases before, which it can still bridge to).
#
# Order matters. The app is notarized and stapled FIRST, because both the
# zip and the DMG must carry a stapled app (a Mac offline at first launch
# still passes Gatekeeper). Then the DMG around it is signed, notarized and
# stapled on its own; a DMG's ticket does not cover the app inside it once
# the app is copied out. Then everything is checked the way a friend's Mac
# checks it, and any refusal stops the run.
#
# Refuses a dirty tree: the build number is the commit count, and a build
# with uncommitted changes would carry a number that names a different
# tree. Needs the AC_PASSWORD notary profile, as `tools/sign.sh notarize`.
set -e
cd "$(dirname "$0")/.."
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    echo "dist: the tree has uncommitted changes; commit first so the build number names this tree" >&2
    exit 2
fi
make release
app=target/bundle/infiniterm.app
identity="Developer ID Application: EKIN ERTAC (QKN7RYV5PD)"
plist="$app/Contents/Info.plist"
version=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$plist")
build=$(/usr/libexec/PlistBuddy -c "Print :CFBundleVersion" "$plist")
name="infiniterm-$version-$build-arm64"
# Semver, the open source convention (#19); the build rides in the
# release title and notes. Was v<version>-<build>.
tag="v$version"
out=target/dist
rm -rf "$out"
mkdir -p "$out"

# notarytool's exit status is 0 for an Invalid submission too; the status
# line is the answer, and the log is what says why. Its status is caught
# with `|| true`: under `set -e` a failing `$(...)` ended the script before
# the result was printed, and an expired Apple agreement (HTTP 403) showed
# as nothing but "Error 1" (#35).
notarize() {
    result=$(xcrun notarytool submit "$1" --keychain-profile AC_PASSWORD --wait 2>&1) || true
    echo "$result" | tail -3
    if ! echo "$result" | grep -q "status: Accepted"; then
        id=$(echo "$result" | sed -n 's/^ *id: //p' | head -1)
        [ -n "$id" ] && xcrun notarytool log "$id" --keychain-profile AC_PASSWORD
        echo "dist: $1 was not accepted by the notary" >&2
        exit 1
    fi
}

echo "--- the app"
ditto -c -k --keepParent "$app" "$out/notarize.zip"
notarize "$out/notarize.zip"
rm "$out/notarize.zip"
xcrun stapler staple "$app"

echo "--- the update payload"
ditto -c -k --keepParent "$app" "$out/$name.zip"

echo "--- the DMG"
stage=$(mktemp -d)
ditto "$app" "$stage/infiniterm.app"
ln -s /Applications "$stage/Applications"
hdiutil create -quiet -volname infiniterm -srcfolder "$stage" -ov -format UDZO "$out/$name.dmg"
rm -rf "$stage"
codesign --timestamp --sign "$identity" "$out/$name.dmg"
notarize "$out/$name.dmg"
xcrun stapler staple "$out/$name.dmg"

echo "--- checked as a friend's Mac would"
spctl -a -t exec -vv "$app" 2>&1 | grep -q "source=Notarized Developer ID" \
    || { spctl -a -t exec -vv "$app"; echo "dist: Gatekeeper refuses the app" >&2; exit 1; }
spctl -a -t open --context context:primary-signature -vv "$out/$name.dmg" 2>&1 | grep -q "source=Notarized Developer ID" \
    || { spctl -a -t open --context context:primary-signature -vv "$out/$name.dmg"; echo "dist: Gatekeeper refuses the DMG" >&2; exit 1; }
xcrun stapler validate -q "$app"
xcrun stapler validate -q "$out/$name.dmg"

echo "--- latest.json"
sha=$(shasum -a 256 "$out/$name.zip" | cut -d' ' -f1)
base="https://github.com/ekinertac/infiniterm/releases/download/$tag"
cat > "$out/latest.json" <<JSON
{
  "version": "$version",
  "build": $build,
  "tag": "$tag",
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "commit": "$(git rev-parse --short HEAD)",
  "url": "$base/$name.zip",
  "sha256": "$sha",
  "dmg": "$base/$name.dmg"
}
JSON
ls -la "$out"
echo "dist: $tag ready in $out; tools/publish.sh puts it on the releases repo"
