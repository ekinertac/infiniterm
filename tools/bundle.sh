#!/bin/sh
# Wraps the built binary in a minimal .app so macOS activates it: a binary
# started from a shell gets no key events in gpui (see HANDOVER.md). Copies
# the reference's 521 themes in as resources so a fresh install has colours.
#   tools/bundle.sh [debug|release]    -> target/bundle/infiniterm.app
# The icon key is required even as a placeholder (global rule: launchers
# drop bundles without one as helper daemons).
set -e
cd "$(dirname "$0")/.."
profile=${1:-debug}
app=target/bundle/infiniterm.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/themes"
cp "target/$profile/infiniterm" "$app/Contents/MacOS/infiniterm"
themes="$HOME/Code/infiniterm/src-tauri/resources/themes"
[ -d "$themes" ] && ditto "$themes" "$app/Contents/Resources/themes"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>infiniterm</string>
  <key>CFBundleIdentifier</key><string>dev.ekinertac.infiniterm.native</string>
  <key>CFBundleName</key><string>infiniterm</string>
  <key>CFBundleDisplayName</key><string>infiniterm</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleIconName</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
echo "$app"
