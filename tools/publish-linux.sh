#!/bin/sh
# The Linux half of a release, run on Ekin's Mac AFTER tools/publish.sh has
# published the release and the `desktop` job in .github/workflows/linux.yml
# has attached infiniterm-linux-<arch>.tar.gz and .sha256 to it:
#
#   tools/publish-linux.sh v0.6.3
#
# Writes target/dist/latest-linux.json (what install.sh and the app's
# update notice read: flat keys, one URL and sha256 per architecture; see
# `update::parse_linux_manifest`) and uploads it to the release, then
# writes target/dist/infiniterm-linux.rb, the Homebrew formula for
# ekinertac/homebrew-tap. The formula is NOT pushed: the tap is public, and
# that waits for Ekin's word (docs/agent/linux-port.md, "Packaging").
#
# The build number is the commit count at the tag, which is what the CI job
# stamped into the binaries (INFINITERM_BUILD).
#
# DRY_RUN=1 uploads nothing. SHA_DIR=<dir> reads the two .sha256 files from
# there instead of the release (tools/test-publish-linux.sh).
set -eu
cd "$(dirname "$0")/.."
tag=${1:?usage: tools/publish-linux.sh <tag, like v0.6.3>}
case "$tag" in v[0-9]*) ;; *) echo "publish-linux: a tag looks like v0.6.3, not $tag" >&2; exit 2 ;; esac
repo=ekinertac/infiniterm
version=${tag#v}
build=$(git rev-list --count "$tag") || { echo "publish-linux: no tag $tag here; git fetch --tags" >&2; exit 2; }
base="https://github.com/$repo/releases/download/$tag"
out=target/dist
mkdir -p "$out"

shas=${SHA_DIR:-}
if [ -z "$shas" ]; then
    shas=$(mktemp -d)
    trap 'rm -rf "$shas"' EXIT
    gh release download "$tag" --repo "$repo" --dir "$shas" \
        --pattern 'infiniterm-linux-*.tar.gz.sha256' \
        || { echo "publish-linux: the release has no Linux tarballs yet; wait for the desktop job" >&2; exit 1; }
fi
sha_of() {
    f="$shas/infiniterm-linux-$1.tar.gz.sha256"
    [ -f "$f" ] || { echo "publish-linux: no $f" >&2; exit 1; }
    s=$(cut -d' ' -f1 "$f")
    case "$s" in *[!0-9a-f]* | '') echo "publish-linux: $f holds no sha256" >&2; exit 1 ;; esac
    printf '%s' "$s"
}
x86=$(sha_of x86_64)
arm=$(sha_of aarch64)

cat > "$out/latest-linux.json" <<EOF
{
  "version": "$version",
  "build": $build,
  "tag": "$tag",
  "x86_64_url": "$base/infiniterm-linux-x86_64.tar.gz",
  "x86_64_sha256": "$x86",
  "aarch64_url": "$base/infiniterm-linux-aarch64.tar.gz",
  "aarch64_sha256": "$arm"
}
EOF

cat > "$out/infiniterm-linux.rb" <<EOF
# The infiniterm desktop app for Linux. Not named infiniterm: that is the
# Mac cask in this tap, and brew takes a formula over a cask of the same
# name. Written by tools/publish-linux.sh in ekinertac/infiniterm.
class InfinitermLinux < Formula
  desc "Terminals, editors and agents on one zoomable canvas"
  homepage "https://infiniterm.app"
  version "$version"
  depends_on :linux

  on_intel do
    url "$base/infiniterm-linux-x86_64.tar.gz"
    sha256 "$x86"
  end
  on_arm do
    url "$base/infiniterm-linux-aarch64.tar.gz"
    sha256 "$arm"
  end

  def install
    libexec.install Dir["*"]
    bin.install_symlink libexec/"infiniterm", libexec/"ift"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/ift --version")
  end
end
EOF

if [ -n "${DRY_RUN:-}" ]; then
    echo "publish-linux: DRY_RUN, nothing uploaded; $out/latest-linux.json and $out/infiniterm-linux.rb written"
    exit 0
fi
gh release upload "$tag" --repo "$repo" --clobber "$out/latest-linux.json"
echo "publish-linux: latest-linux.json is on $tag. The formula is in $out/infiniterm-linux.rb; push it to ekinertac/homebrew-tap on Ekin's word."
