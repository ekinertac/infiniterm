#!/bin/sh
# tools/publish-windows.sh with DRY_RUN and a fake zip: checks that it accepts
# a zip and a manifest that belong together, and refuses each way they can
# disagree (the tag, the version, the sha256, the url) and a missing or doubled
# zip, so a wrong file never reaches a release. Uploads nothing, needs no
# network and no tag in the clone. Runs on the Mac or Linux.
# Related: tools/publish-windows.sh, tools/dist.ps1 (writes the manifest).
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
cd "$here"
fails=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fails=$((fails + 1)); fi; }

d=$(mktemp -d)
trap 'rm -rf "$d"' EXIT
tag=v9.9.9
zip=infiniterm-9.9.9-1234-x86_64.zip
echo "not really a zip" > "$d/$zip"
if command -v shasum >/dev/null 2>&1; then sha=$(shasum -a 256 "$d/$zip" | cut -d' ' -f1); else sha=$(sha256sum "$d/$zip" | cut -d' ' -f1); fi
manifest() { # tag version sha url
    cat > "$d/latest-windows.json" <<EOF
{
  "version": "$2",
  "build": 1234,
  "tag": "$1",
  "pub_date": "2026-11-23T00:00:00Z",
  "commit": "abcdef0",
  "url": "$4",
  "sha256": "$3"
}
EOF
}
url="https://github.com/ekinertac/infiniterm/releases/download/$tag/$zip"
run() { DRY_RUN=1 sh tools/publish-windows.sh "${1:-$tag}" "$d" >/dev/null 2>&1; }

manifest "$tag" 9.9.9 "$sha" "$url"
check "a zip and a manifest that belong together pass" 'run'
check "a tag that is not v<version> is refused" '! run latest'
manifest v9.9.9-1234 9.9.9 "$sha" "$url"
check "a manifest with the build in its tag is refused" '! run'
manifest "$tag" 9.9.8 "$sha" "$url"
check "a manifest of another version is refused" '! run'
manifest "$tag" 9.9.9 0000000000000000000000000000000000000000000000000000000000000000 "$url"
check "a manifest whose sha256 is not the zip's is refused" '! run'
manifest "$tag" 9.9.9 "$sha" "https://example.com/$zip"
check "a manifest whose url is not the release download is refused" '! run'
manifest "$tag" 9.9.9 "$sha" "$url"
echo "another" > "$d/infiniterm-9.9.9-1235-x86_64.zip"
check "two zips for one version are refused" '! run'
rm "$d/infiniterm-9.9.9-1235-x86_64.zip" "$d/$zip"
check "a missing zip is refused" '! run'
echo "not really a zip" > "$d/$zip"
rm "$d/latest-windows.json"
check "a missing manifest is refused" '! run'

[ "$fails" -eq 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }
