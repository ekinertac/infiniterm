#!/bin/sh
# tools/publish-linux.sh with DRY_RUN and fake .sha256 files: checks the
# manifest it writes is what the app parses (`update::parse_linux_manifest`
# keys) and install.sh reads (flat "key": "value" lines), and that the
# formula names both architectures and is valid Ruby. Uploads nothing,
# needs no network. Runs on the Mac or Linux.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
cd "$here"
fails=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fails=$((fails + 1)); fi; }

tag=$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null) || { echo "skip: no v* tag in this clone"; exit 0; }
shas=$(mktemp -d)
trap 'rm -rf "$shas"' EXIT
echo "1111111111111111111111111111111111111111111111111111111111111111  infiniterm-linux-x86_64.tar.gz" > "$shas/infiniterm-linux-x86_64.tar.gz.sha256"
echo "2222222222222222222222222222222222222222222222222222222222222222  infiniterm-linux-aarch64.tar.gz" > "$shas/infiniterm-linux-aarch64.tar.gz.sha256"

DRY_RUN=1 SHA_DIR="$shas" sh tools/publish-linux.sh "$tag" >/dev/null
m=target/dist/latest-linux.json
f=target/dist/infiniterm-linux.rb
field() { sed -n "s/^ *\"$1\": *\"\([^\"]*\)\".*/\1/p" "$m" | head -n 1; }

check "the manifest names the tag" '[ "$(field tag)" = "$tag" ]'
check "the build is the commit count at the tag" 'grep -q "\"build\": $(git rev-list --count "$tag")," "$m"'
check "install.sh finds the x86_64 url" '[ "$(field x86_64_url)" = "https://github.com/ekinertac/infiniterm/releases/download/$tag/infiniterm-linux-x86_64.tar.gz" ]'
check "install.sh finds the aarch64 sha256" '[ "$(field aarch64_sha256)" = 2222222222222222222222222222222222222222222222222222222222222222 ]'
check "the formula is not named infiniterm" 'grep -q "^class InfinitermLinux < Formula" "$f"'
check "the formula is Linux only" 'grep -q "depends_on :linux" "$f"'
check "the formula pins both tarballs" 'grep -q "sha256 \"1111" "$f" && grep -q "sha256 \"2222" "$f"'
if command -v ruby >/dev/null; then
    check "the formula is valid Ruby" 'ruby -c "$f" >/dev/null'
fi
# A bad sha file is refused, not written.
echo "not-a-sha  x" > "$shas/infiniterm-linux-aarch64.tar.gz.sha256"
check "a broken sha256 stops it" '! DRY_RUN=1 SHA_DIR="$shas" sh tools/publish-linux.sh "$tag" >/dev/null 2>&1'
check "a tag that is not v<version> is refused" '! DRY_RUN=1 SHA_DIR="$shas" sh tools/publish-linux.sh latest >/dev/null 2>&1'

[ "$fails" -eq 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }
