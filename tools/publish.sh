#!/bin/sh
# Puts what tools/dist.sh left in target/dist/ on ekinertac/infiniterm as a
# GitHub Release marked latest, on the source tag v<version>: the DMG for a
# first install, the zip the updater downloads, and latest.json, which the
# running app polls at releases/latest/download/latest.json (update.rs), a
# URL GitHub always points at the newest release. Then points the Homebrew
# cask in ekinertac/homebrew-tap at it, and replays the updater's checks
# against every feed it wrote (tools/check-update-feed.sh).
#
# BRIDGE=1 also publishes the release on ekinertac/infiniterm-releases,
# where releases lived while the source was private (#52). Builds up to
# 0.4.0 poll that repo and accept downloads only from it, so the bridge
# carries its own copy of the zip and DMG and a latest.json pointing at
# them; the build they install polls the main repo from then on.
#
# Notes default to one line; pass NOTES="..." for more.
set -e
cd "$(dirname "$0")/.."
out=target/dist
[ -f "$out/latest.json" ] || { echo "publish: nothing in $out; run tools/dist.sh first" >&2; exit 2; }
field() { sed -n "s/^ *\"$1\": \"\{0,1\}\([^\",]*\)\"\{0,1\},\{0,1\}$/\1/p" "$out/latest.json"; }
tag=$(field tag)
version=$(field version)
build=$(field build)
commit=$(field commit)
[ "$(git rev-parse --short HEAD)" = "$commit" ] \
    || { echo "publish: $out was built from $commit, HEAD is $(git rev-parse --short HEAD); run dist again" >&2; exit 2; }
repo=ekinertac/infiniterm
old=ekinertac/infiniterm-releases
# One version, one release: a second build of 0.3.0 would need a tag that
# already exists. Bump `version` in Cargo.toml and run dist again.
if gh release view "$tag" --repo "$repo" >/dev/null 2>&1 || git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    echo "publish: $tag is already published; bump the version in Cargo.toml, then run dist again" >&2
    exit 2
fi
notes=${NOTES:-"infiniterm $version, build $build."}
# The tag first, on the commit that was built, so the release sits on it
# rather than on whatever master is by then.
git tag -a "$tag" -m "infiniterm $version, build $build"
git push -q origin "$tag"
gh release create "$tag" --repo "$repo" --verify-tag --latest \
    --title "infiniterm $version ($build)" --notes "$notes" \
    "$out"/*.dmg "$out"/*.zip "$out/latest.json"
if [ -n "${BRIDGE:-}" ]; then
    bridge=$(mktemp -d)
    sed "s|github.com/$repo/releases/download/|github.com/$old/releases/download/|g" \
        "$out/latest.json" > "$bridge/latest.json"
    gh release create "$tag" --repo "$old" --latest \
        --title "infiniterm $version ($build)" \
        --notes "$notes

Releases are published on https://github.com/$repo/releases from this version on." \
        "$out"/*.dmg "$out"/*.zip "$bridge/latest.json"
    rm -rf "$bridge"
fi
# The Homebrew cask follows the release, or `brew install` hands out the
# previous build until someone remembers (tools/bump-cask.sh).
tools/bump-cask.sh "$version" "$build" "$(ls "$out"/*.dmg)"
# What every installed copy will do, done now: a feed that does not pass
# here would leave people on the old build with an error in the log.
tools/check-update-feed.sh "https://github.com/$repo/releases/latest/download/latest.json" "$build"
if [ -n "${BRIDGE:-}" ]; then
    tools/check-update-feed.sh "https://github.com/$old/releases/latest/download/latest.json" "$build"
fi
echo "publish: https://github.com/$repo/releases/tag/$tag"
