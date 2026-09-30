#!/bin/sh
# Puts what tools/dist.sh left in target/dist/ on ekinertac/infiniterm-releases
# as a GitHub Release marked latest: the DMG for a first install, the zip
# the updater downloads, and latest.json, which the running app polls at
# releases/latest/download/latest.json (updater.rs), a URL GitHub always
# points at the newest release. Then points the Homebrew cask in
# ekinertac/homebrew-tap at it and tags the source commit v<version>, the
# same semver tag as the release (#19; release-<build> before).
#
# The releases repo is PUBLIC: notes go there as written. They default to
# one line; pass NOTES="..." for more. Commit subjects are not copied over,
# because this repo is private and its history is not for the public page.
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
repo=ekinertac/infiniterm-releases
# One version, one release: a second build of 0.3.0 would need a tag that
# already exists. Bump `version` in Cargo.toml and run dist again.
if gh release view "$tag" --repo "$repo" >/dev/null 2>&1 || git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    echo "publish: $tag is already published; bump the version in Cargo.toml, then run dist again" >&2
    exit 2
fi
notes=${NOTES:-"infiniterm $version, build $build."}
gh release create "$tag" --repo "$repo" --latest \
    --title "infiniterm $version ($build)" --notes "$notes" \
    "$out"/*.dmg "$out"/*.zip "$out/latest.json"
# The Homebrew cask follows the release, or `brew install` hands out the
# previous build until someone remembers (tools/bump-cask.sh).
tools/bump-cask.sh "$version" "$build" "$(ls "$out"/*.dmg)"
git tag -a "$tag" -m "infiniterm $version, build $build, published on $repo"
git push -q origin "$tag"
echo "publish: https://github.com/$repo/releases/tag/$tag"
