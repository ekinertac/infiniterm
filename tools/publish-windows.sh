#!/bin/sh
# The Windows half of a release, run on Ekin's Mac AFTER tools/publish.sh has
# published the release and AFTER tools/dist.ps1 has built the zip on the
# Windows box and the zip and its manifest were copied to target/dist-win/:
#
#   tools/publish-windows.sh v0.6.18
#
# Checks that the manifest and the zip belong together (the tag, the version,
# the sha256 of the zip, the url's file name), then attaches the zip and
# latest-windows.json to the release. The manifest is a SEPARATE file from the
# Mac's latest.json (see the comment at the top of tools/dist.ps1): an app
# already shipped polls latest.json and must never see a shape it does not
# expect.
#
# Experimental by design: there is no Authenticode signature (it needs a paid
# certificate), so SmartScreen warns on a first run, and the manifest is not
# signed until an update key is committed (`update::UPDATE_PUBLIC_KEY`). Say
# so in the release notes; this script does not touch them.
#
# DRY_RUN=1 checks everything and uploads nothing, with no network. The
# directory defaults to target/dist-win; pass another as the second argument.
# Related: tools/dist.ps1 (builds), tools/publish-linux.sh (the Linux half),
# tools/test-publish-windows.sh (this script's checks).
set -eu
cd "$(dirname "$0")/.."
tag=${1:?usage: tools/publish-windows.sh <tag, like v0.6.18> [dir]}
case "$tag" in v[0-9]*) ;; *) echo "publish-windows: a tag looks like v0.6.18, not $tag" >&2; exit 2 ;; esac
dir=${2:-target/dist-win}
repo=ekinertac/infiniterm
version=${tag#v}
manifest="$dir/latest-windows.json"

[ -f "$manifest" ] || { echo "publish-windows: no $manifest; copy it from the Windows box" >&2; exit 1; }
zips=$(ls "$dir"/infiniterm-"$version"-*-x86_64.zip 2>/dev/null || true)
case "$zips" in
    '') echo "publish-windows: no $dir/infiniterm-$version-<build>-x86_64.zip for $tag" >&2; exit 1 ;;
    *'
'*) echo "publish-windows: more than one zip for $version in $dir; keep the one to publish" >&2; exit 1 ;;
esac
zip=$zips
name=$(basename "$zip")

sha256() {
    if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1; else sha256sum "$1" | cut -d' ' -f1; fi
}
field() { sed -n "s/^ *\"$1\": *\"\([^\"]*\)\".*/\1/p" "$manifest" | head -n 1; }
fail() { echo "publish-windows: $1" >&2; exit 1; }

[ "$(field tag)" = "$tag" ] || fail "the manifest names tag \"$(field tag)\", not $tag (dist.ps1 from before aa467b8f wrote v<version>-<build>)"
[ "$(field version)" = "$version" ] || fail "the manifest names version \"$(field version)\", not $version"
[ "$(field sha256)" = "$(sha256 "$zip")" ] || fail "the manifest's sha256 is not the zip's; the manifest and the zip are from different builds"
[ "$(field url)" = "https://github.com/$repo/releases/download/$tag/$name" ] || fail "the manifest's url is \"$(field url)\", not the release download of $name"

if [ -n "${DRY_RUN:-}" ]; then
    echo "publish-windows: DRY_RUN, checks passed, nothing uploaded ($name, $(wc -c < "$zip" | tr -d ' ') bytes)"
    exit 0
fi
gh release view "$tag" --repo "$repo" >/dev/null 2>&1 || fail "no release $tag on $repo; publish the Mac release first"
gh release upload "$tag" --repo "$repo" --clobber "$zip" "$manifest"
echo "publish-windows: $name and latest-windows.json are on $tag (unsigned, experimental; say so in the release notes)"
