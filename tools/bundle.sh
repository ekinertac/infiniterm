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
# symlinks and `ift install-claude-hooks` points the hooks at. `iftd` rides
# along too: it is how a card's shell outlives the app, and
# `DaemonBackend::find_iftd` looks beside the executable first, which is
# exactly this directory.
cargo build $( [ "$profile" = release ] && echo --release ) -p infiniterm-cli -p infiniterm-hook -p infiniterm-session
cp "target/$profile/ift" "target/$profile/infiniterm-hook" "target/$profile/iftd" "$app/Contents/MacOS/"
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
# A real version on every bundle, not the bundler's 1.0.0: the updater
# (updater.rs) compares the running build with latest.json's, so the build
# number has to grow with every commit. The version is the workspace's;
# the build is the commit count, monotonic on master.
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
build=$(git rev-list --count HEAD)
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $build" "$plist"
# cef-rs's bundler sets LSFileQuarantineEnabled because it is modelled on
# the CEF browser sample, and a browser SHOULD quarantine what it downloads.
# Here it means every file every shell in every card writes is stamped
# com.apple.quarantine with infiniterm named as the agent: a binary you just
# compiled in a card is treated by Gatekeeper as an internet download. A
# terminal must not do that. When browser cards learn to download, the
# quarantine belongs on the downloaded file, not on everything the app
# touches.
/usr/libexec/PlistBuddy -c "Set :LSFileQuarantineEnabled false" "$plist" 2>/dev/null || true
echo "$app"
