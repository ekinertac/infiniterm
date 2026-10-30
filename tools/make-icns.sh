#!/bin/bash
# make-icns.sh <1024px.png> [out.icns]: builds a macOS icon file from one
# square PNG. The default output is assets/AppIcon.icns, the file bundle.sh
# copies into the app. The source must already be shaped as a macOS icon
# (rounded square with its transparent margin): nothing here masks it.
#
# Related: assets/icons/default.png (the source of the bundle's icon),
# tools/bundle.sh, infiniterm-ui/src/instance_icon.rs (the Dock icon at runtime).
set -euo pipefail
src=${1:?usage: tools/make-icns.sh <1024px.png> [out.icns]}
out=${2:-"$(cd "$(dirname "$0")/.." && pwd)/assets/AppIcon.icns"}
[ "$(sips -g pixelWidth "$src" | awk '/pixelWidth/ {print $2}')" = 1024 ] || { echo "make-icns: $src is not 1024 px wide" >&2; exit 1; }
set=$(mktemp -d)/AppIcon.iconset
mkdir -p "$set"
# iconutil wants exactly these names: each size at 1x, and at 2x (double the pixels).
for s in 16 32 128 256 512; do
    sips -z "$s" "$s" "$src" --out "$set/icon_${s}x${s}.png" >/dev/null
    sips -z $((s * 2)) $((s * 2)) "$src" --out "$set/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$set" -o "$out"
rm -rf "$(dirname "$set")"
echo "wrote $out"
