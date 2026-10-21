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
# Built, then run as a plain binary, NOT through `cargo run`: `cargo run`
# hands the program CARGO_MANIFEST_DIR and the other CARGO_* variables, the
# bundler's own `cargo build` of this workspace inherited them, and `ring`'s
# build script took the change for a new environment. Every bundle then
# made the next `make check` rebuild ring, rustls, CEF, gpui and the ui
# crate: 2.5 to 12 minutes a ship (2026-09-26).
cargo build -q --manifest-path "$HOME/Code/cef-rs/Cargo.toml" -p cef --bin bundle-cef-app
# `cargo run` also gave it CEF_PATH from .cargo/config.toml; a plain run
# needs it said.
CEF_PATH="${CEF_PATH:-$HOME/.local/share/cef}" \
    "$HOME/Code/cef-rs/target/debug/bundle-cef-app" \
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
# The app icon (#57): four cards whose borders are the four card states.
cp assets/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
# The 522 iTerm2-Color-Schemes files, kept in the repo (assets/themes, MIT,
# licence inside). They were copied from the archived Tauri checkout, and a
# Mac without it built a bundle with no themes at all, silently; a missing
# folder now stops the build instead.
[ -d assets/themes ] || { echo "bundle: assets/themes is missing" >&2; exit 1; }
mkdir -p "$app/Contents/Resources/themes"
ditto assets/themes "$app/Contents/Resources/themes"
# The pictures ui.backgroundImage can name (assets/backgrounds, #97).
[ -d assets/backgrounds ] || { echo "bundle: assets/backgrounds is missing" >&2; exit 1; }
mkdir -p "$app/Contents/Resources/backgrounds"
ditto assets/backgrounds "$app/Contents/Resources/backgrounds"
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
# Privacy usage descriptions (#152, #170). infiniterm is a terminal: any program
# in a card can ask macOS for a protected resource, and the system shows its
# dialog for an app only when Info.plist has the matching key. Nothing is asked
# at install or launch; the dialog appears when a program first touches the
# resource, and the text says whose request it is. cef-rs's bundler already
# adds some of these with the value "infiniterm", which tells the person
# nothing, so every key is Set over. The list is checked at the end: a missing
# key fails the bundle instead of showing no prompt on somebody's Mac.
usage_keys="NSAppleEventsUsageDescription|control another app
NSAppleMusicUsageDescription|use your media library
NSBluetoothAlwaysUsageDescription|use Bluetooth
NSCalendarsUsageDescription|read your calendars
NSCalendarsFullAccessUsageDescription|read and change your calendars
NSCalendarsWriteOnlyAccessUsageDescription|add events to your calendars
NSCameraUsageDescription|use the camera
NSContactsUsageDescription|read your contacts
NSDesktopFolderUsageDescription|read files on your Desktop
NSDocumentsFolderUsageDescription|read files in your Documents folder
NSDownloadsFolderUsageDescription|read files in your Downloads folder
NSFileProviderDomainUsageDescription|read files in a cloud storage folder
NSFocusStatusUsageDescription|see your Focus status
NSHomeKitUsageDescription|use your Home devices
NSLocalNetworkUsageDescription|reach devices on your local network
NSLocationUsageDescription|use your location
NSLocationWhenInUseUsageDescription|use your location
NSLocationAlwaysAndWhenInUseUsageDescription|use your location
NSMicrophoneUsageDescription|use the microphone
NSMotionUsageDescription|use motion data
NSNetworkVolumesUsageDescription|read files on a network volume
NSPhotoLibraryUsageDescription|read your Photos library
NSPhotoLibraryAddUsageDescription|add pictures to your Photos library
NSRemindersUsageDescription|read your reminders
NSRemindersFullAccessUsageDescription|read and change your reminders
NSRemovableVolumesUsageDescription|read files on a removable drive
NSSiriUsageDescription|use Siri
NSSpeechRecognitionUsageDescription|use speech recognition
NSSystemAdministrationUsageDescription|do administrator tasks"
# Finder's Services > "Open in infiniterm" (#344): a terminal card for each
# selected folder. NSPortName is the bundle's name, which is how macOS finds
# the process to send the message to; infiniterm-ui/src/finder_service.rs
# answers `openFolder:userData:error:`. Deleted first so a re-run does not
# stack a second entry. NSSendFileTypes, not NSSendTypes: Finder sends files,
# and a service for a selected file or folder is matched by its file type;
# NSSendTypes names pasteboard types, and public.folder is not one, so the
# item never showed (2026-10-10). No entitlement: the app is not sandboxed.
/usr/libexec/PlistBuddy -c "Delete :NSServices" "$plist" 2>/dev/null || true
for cmd in \
    "Add :NSServices array" \
    "Add :NSServices:0 dict" \
    "Add :NSServices:0:NSMenuItem dict" \
    "Add :NSServices:0:NSMenuItem:default string Open in infiniterm" \
    "Add :NSServices:0:NSMessage string openFolder" \
    "Add :NSServices:0:NSPortName string infiniterm" \
    "Add :NSServices:0:NSSendFileTypes array" \
    "Add :NSServices:0:NSSendFileTypes:0 string public.folder"; do
    /usr/libexec/PlistBuddy -c "$cmd" "$plist"
done
echo "$usage_keys" | while IFS='|' read -r key what; do
    text="A program running in an infiniterm card wants to $what. infiniterm itself never does; it only passes the request on to macOS."
    /usr/libexec/PlistBuddy -c "Set :$key $text" "$plist" 2>/dev/null || /usr/libexec/PlistBuddy -c "Add :$key string $text" "$plist"
done
missing=$(echo "$usage_keys" | while IFS='|' read -r key what; do
    /usr/libexec/PlistBuddy -c "Print :$key" "$plist" >/dev/null 2>&1 || echo "$key"
done)
[ -z "$missing" ] || { echo "bundle: missing from Info.plist: $missing" >&2; exit 1; }
echo "$app"
