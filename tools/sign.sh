#!/bin/sh
# Signs target/bundle/infiniterm.app with Ekin's Developer ID, and with
# `notarize` sends it to Apple and staples the ticket.
#
# Why sign at all for an app that is only ever installed by hand: macOS
# keys a privacy grant (Screen Recording, Accessibility) to the app's code
# signature, and the ad-hoc, linker-only signature cargo leaves has no
# stable identity, so the toggle in System Settings flipped itself back
# off. One certificate across rebuilds makes the grant stick. Inside out,
# as Apple wants it: sidecars, the framework's dylibs and the framework,
# the helpers, then the app,
# with the same entitlements everywhere because the renderer helper JITs
# and the rest load the framework unsigned by Apple.
#   tools/sign.sh [notarize]
# The notary credentials are the AC_PASSWORD keychain profile
# (`xcrun notarytool store-credentials`), the one ScreenCop uses.
set -e
cd "$(dirname "$0")/.."
app=target/bundle/infiniterm.app
identity="Developer ID Application: EKIN ERTAC (QKN7RYV5PD)"
ent=tools/entitlements.plist
sign() { codesign --force --timestamp --options runtime --sign "$identity" --entitlements "$ent" "$@"; }
for bin in ift iftd infiniterm-hook; do sign "$app/Contents/MacOS/$bin"; done
# A framework's signature does not cover the dylibs under Libraries/; the
# notary refuses each one unsigned.
fw="$app/Contents/Frameworks/Chromium Embedded Framework.framework"
for lib in "$fw"/Libraries/*.dylib; do sign "$lib"; done
sign "$fw"
for h in "$app"/Contents/Frameworks/*.app; do sign "$h"; done
sign "$app"
codesign --verify --deep --strict "$app"
if [ "$1" = notarize ]; then
    zip=target/bundle/infiniterm.zip
    rm -f "$zip"
    ditto -c -k --keepParent "$app" "$zip"
    xcrun notarytool submit "$zip" --keychain-profile AC_PASSWORD --wait
    xcrun stapler staple "$app"
    spctl -a -vv -t install "$app"
fi
