#!/bin/sh
# Spike: does CEF's Chrome bootstrap run the Claude in Chrome extension?
# Usage: ./run.sh [extra cefclient flags...]   e.g. ./run.sh --off-screen-rendering-enabled
# Logs to cefclient.log; the app is a GUI, so run it in the background.
cd "$(dirname "$0")"
APP=$(echo cef_binary_*/Release/cefclient.app/Contents/MacOS/cefclient)
exec "$APP" \
  --cache-path="$PWD/profile" \
  --load-extension="$PWD/extension" \
  --url=https://claude.ai/login \
  --user-agent="Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/152.0.0.0 Safari/537.36" \
  "$@" > cefclient.log 2>&1
