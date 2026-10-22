#!/usr/bin/env bash
# The Linux update notice, end to end, ON the Linux box (a throwaway droplet:
# it installs into the box's real HOME and runs the app on its real data dir,
# because the updater is off for a scratch instance by design):
#
#   tools/drive/linux/box.sh sync
#   tools/drive/linux/box.sh x tools/drive/linux/update-test.sh
#
# Builds the app with INFINITERM_BUILD=5, installs it with install.sh, hands
# it a manifest naming build 9999 through INFINITERM_LATEST_URL (a file://
# URL), starts it from ~/.local/bin and checks the notice and the status bar
# segment by screenshot and by the app's log. Never run it on a Mac or any
# machine with a canvas you care about.
#
# Related: updater.rs (`start_notifier`), update.rs (`parse_linux_manifest`,
# `linux_install`), install-test.sh, publish-linux.sh.
set -euo pipefail
cd "$(dirname "$0")/../../.."
[ "$(uname -s)" = Linux ] || { echo "update-test: Linux boxes only" >&2; exit 2; }
[ -n "${INFINITERM_DATA_DIR:-}" ] && { echo "update-test: unset INFINITERM_DATA_DIR; the updater is off for scratch instances" >&2; exit 2; }

INFINITERM_BUILD=5 cargo build -q -p infiniterm-ui --no-default-features -p infiniterm-cli -p infiniterm-session -p infiniterm-hook
tools/linux-dist.sh target/debug >/dev/null
INFINITERM_TARBALL=$(ls dist/infiniterm-linux-*.tar.gz) sh site/public/install.sh --no-hooks </dev/null >/dev/null

arch=$(uname -m)
cat > /tmp/latest-linux.json <<EOF
{
  "version": "9.9.9",
  "build": 9999,
  "tag": "v9.9.9",
  "${arch}_url": "https://github.com/ekinertac/infiniterm/releases/download/v9.9.9/infiniterm-linux-${arch}.tar.gz",
  "${arch}_sha256": "00"
}
EOF

export DISPLAY=${DISPLAY:-:99}
bash tools/drive/linux/remote.sh layout us >/dev/null   # Xvfb and a window manager
pkill -x infiniterm || true
sleep 1
(cd /tmp && INFINITERM_LATEST_URL=file:///tmp/latest-linux.json exec setsid "$HOME/.local/bin/infiniterm") </dev/null >/tmp/update-test.log 2>&1 &
sleep 8
# Started, and not handed off to another instance holding the socket.
if ! pgrep -x infiniterm >/dev/null || grep -q "another infiniterm holds" /tmp/update-test.log; then
    echo "FAIL: the installed app did not start:"; tail -5 /tmp/update-test.log; exit 1
fi
import -window root /tmp/update-test.png
pkill -x infiniterm || true
echo "screenshot: /tmp/update-test.png (look for \"Build 9999 is out\" and \"build 5\" in the status bar)"
if grep -q "update check:" /tmp/update-test.log; then
    echo "FAIL: the check reported an error:"; grep "update check:" /tmp/update-test.log; exit 1
fi
