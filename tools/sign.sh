#!/bin/sh
# Signs target/bundle/infiniterm.app with Ekin's Developer ID. Notarizing
# is tools/dist.sh's, for the build that leaves this Mac.
#
# Why sign at all for an app that is only ever installed by hand: macOS
# keys a privacy grant (Screen Recording, Accessibility) to the app's code
# signature, and the ad-hoc, linker-only signature cargo leaves has no
# stable identity, so the toggle in System Settings flipped itself back
# off. One certificate across rebuilds makes the grant stick. Inside out,
# as Apple wants it: the sidecars, then the app, with the same entitlements
# (tools/entitlements.plist). There is no framework and no helper app since
# CEF was removed (#826).
set -e
cd "$(dirname "$0")/.."
app=target/bundle/infiniterm.app
identity="Developer ID Application: EKIN ERTAC (QKN7RYV5PD)"
ent=tools/entitlements.plist
sign() { codesign --force --timestamp --options runtime --sign "$identity" --entitlements "$ent" "$@"; }
for bin in ift iftd infiniterm-hook; do sign "$app/Contents/MacOS/$bin"; done
sign "$app"
codesign --verify --deep --strict "$app"
