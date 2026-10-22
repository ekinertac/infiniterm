#!/bin/sh
# The Linux desktop release: one tarball per architecture, from binaries
# already built with
#
#   cargo build --release --locked -p infiniterm-ui --no-default-features \
#       -p infiniterm-cli -p infiniterm-session -p infiniterm-hook
#
# and laid out FLAT, the way the app looks for its resources off macOS
# (`paths::shipped_resource`: themes and backgrounds beside the binary):
#
#   infiniterm/infiniterm  iftd  ift  infiniterm-hook
#   infiniterm/themes/  backgrounds/  icon.png  LICENSE.txt  VERSION
#
# Writes dist/infiniterm-linux-<arch>.tar.gz and its .sha256. install.sh
# (site/public/install.sh, the Linux branch) unpacks it into
# ~/.local/share/infiniterm/<VERSION>/. Run by the release job in
# .github/workflows/linux.yml, and by hand on a Linux box to test install.sh
# (tools/drive/linux/install-test.sh).
#
#   tools/linux-dist.sh [target-dir]     default target/release
#
# No browser cards: the app is built without CEF on Linux for now
# (docs/agent/linux-port.md).
set -eu
cd "$(dirname "$0")/.."

[ "$(uname -s)" = Linux ] || { echo "linux-dist: run this on Linux" >&2; exit 2; }
target=${1:-target/release}
case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) echo "linux-dist: unsupported architecture $(uname -m)" >&2; exit 2 ;;
esac
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
[ -n "$version" ] || { echo "linux-dist: no version in Cargo.toml" >&2; exit 1; }

for bin in infiniterm iftd ift infiniterm-hook; do
    [ -x "$target/$bin" ] || { echo "linux-dist: $target/$bin is missing; build it first" >&2; exit 1; }
done
for dir in assets/themes assets/backgrounds; do
    [ -d "$dir" ] || { echo "linux-dist: $dir is missing" >&2; exit 1; }
done

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
root="$stage/infiniterm"
mkdir -p "$root"
for bin in infiniterm iftd ift infiniterm-hook; do
    cp "$target/$bin" "$root/"
    strip "$root/$bin" 2>/dev/null || true
done
cp -R assets/themes assets/backgrounds "$root/"
cp assets/icon.png LICENSE.txt "$root/"
printf '%s\n' "$version" > "$root/VERSION"

mkdir -p dist
out="dist/infiniterm-linux-$arch.tar.gz"
tar -C "$stage" -czf "$out" infiniterm
(cd dist && sha256sum "${out#dist/}" > "${out#dist/}.sha256")
echo "$out ($version, $(du -h "$out" | cut -f1))"
