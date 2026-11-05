# Extension probes (2026-10-07 and 2026-10-08)

Two small MV3 extensions that print what the `chrome.*` APIs return inside a browser card. They back the table in `docs/superpowers/specs/2026-10-09-browser-extensions-cef-fork-design.md` and are the regression check for the CEF patches in that spec.

- `tabs/`: the background records tab ids from `webNavigation`; the popup calls `tabs.get`, `scripting.executeScript`, `tabs.sendMessage` and `tabs.query` on them.
- `capabilities/`: the popup checks codecs, Widevine, WebAuthn, and about 25 `chrome.*` calls (`action`, `contextMenus`, `bookmarks`, `history`, `windows.create` ...). Its `notifications.create` check uses a broken icon, so that result is not valid.

## Run

1. Start a scratch instance: `INFINITERM_DATA_DIR=/tmp/x open -n target/bundle/infiniterm.app --env INFINITERM_DATA_DIR=/tmp/x`.
2. Install: `INFINITERM_DATA_DIR=/tmp/x ift install-extension spikes/ext-probe/capabilities`.
3. Relaunch the instance.
4. Find the extension id. An unpacked extension without a key gets its id from the sha256 of its path: the first 16 bytes, each nibble mapped to `a`..`p`. The path is the installed copy under `/private/tmp/x/browser/extensions/<name>`.
5. Open a browser card on `chrome-extension://<id>/popup.html`. The popup prints the results.

## Traps

- Edit a probe, then install it under a NEW directory name. CEF keeps the old service worker for the same extension id, even after the manifest version changes.
- `cp` may be an alias for `rsync` on this Mac. Use `rsync -a src/ dst/`.
- `windows.create` opens a real native Chromium window. Quit the scratch instance by pid to close it.
