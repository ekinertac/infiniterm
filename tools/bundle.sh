#!/bin/sh
# Lays out target/bundle/infiniterm.app: the binary, CEF's framework and
# the four helper apps (cef-rs's bundle-cef-app does that part; it also
# runs `cargo build` for the two binaries, debug only), then the
# reference's 521 themes as resources and the plist keys the bundler
# leaves out. A binary started from a shell gets no key events in gpui
# and no CEF framework beside it, so everything runs from here.
#   tools/bundle.sh [debug|release]
# The icon key is required even as a placeholder (global rule: launchers
# drop bundles without one as helper daemons).
set -e
cd "$(dirname "$0")/.."
profile=${1:-debug}
app=target/bundle/infiniterm.app
cargo run -q --manifest-path "$HOME/Code/cef-rs/Cargo.toml" -p cef --bin bundle-cef-app -- \
    infiniterm -o target/bundle --identifier dev.ekinertac.infiniterm --display-name infiniterm >/dev/null
if [ "$profile" = release ]; then
    # The bundler only knows debug; the release binaries go in over them.
    cargo build --release -p infiniterm-ui
    cp target/release/infiniterm "$app/Contents/MacOS/infiniterm"
    for h in "$app"/Contents/Frameworks/*.app/Contents/MacOS/*; do
        cp target/release/infiniterm-helper "$h"
    done
fi
# ift and the hook ride along as sidecars, which is what `ift install`
# symlinks and `ift install-claude-hooks` points the hooks at.
cargo build $( [ "$profile" = release ] && echo --release ) -p infiniterm-cli -p infiniterm-hook
cp "target/$profile/ift" "target/$profile/infiniterm-hook" "$app/Contents/MacOS/"
# The Tauri app's icon, as it was.
cp assets/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
themes="$HOME/Code/infiniterm-tauri/src-tauri/resources/themes"
mkdir -p "$app/Contents/Resources/themes"
[ -d "$themes" ] && ditto "$themes" "$app/Contents/Resources/themes"
plist="$app/Contents/Info.plist"
for kv in "CFBundleIconFile AppIcon" "CFBundleIconName AppIcon" "CFBundleName infiniterm" "LSMinimumSystemVersion 13.0"; do
    set -- $kv
    /usr/libexec/PlistBuddy -c "Set :$1 $2" "$plist" 2>/dev/null || /usr/libexec/PlistBuddy -c "Add :$1 string $2" "$plist"
done
/usr/libexec/PlistBuddy -c "Set :NSHighResolutionCapable true" "$plist" 2>/dev/null || /usr/libexec/PlistBuddy -c "Add :NSHighResolutionCapable bool true" "$plist"
echo "$app"
