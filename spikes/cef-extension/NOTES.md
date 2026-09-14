# CEF + Claude in Chrome spike (2026-09-14)

Question: can CEF's Chrome bootstrap run the Claude in Chrome extension, in off-screen rendering mode, well enough that Claude Code drives it like a real Chrome? Answer: yes, all four checks passed on CEF 152.0.6 (Chromium 152.0.7977.83, macOS arm64) with the prebuilt `cefclient` and no code.

## What was checked

1. Extension loads unpacked from a copy of `~/Library/Application Support/Google/Chrome/Default/Extensions/fcoeoabgfenejglbffodgkkbkcdhcgfn/<version>/`. The Web Store copy carries `key` in `manifest.json`, so the unpacked load keeps the store id, which is what the native messaging manifests' `allowed_origins` name. Shows up in `chrome://extensions`, service worker and offscreen document start.
2. Native messaging. CEF looks for host manifests in `<cache-path>/NativeMessagingHosts/`, not in `~/Library/Application Support/Google/Chrome/` or `.../Chromium/`. Symptom before the copy: `Can't find manifest for native messaging host com.anthropic.claude_browser_extension` in `profile/chrome_debug.log`. After copying both `com.anthropic.*.json` files from Chrome's directory there, a second `chrome-native-host` process spawned from cefclient.
3. Claude Code sees it. The extension connects per claude.ai account, so it must be signed in inside cefclient once (email code; Google sign-in refuses CEF and a Chrome `--user-agent` does not clear it). `list_connected_browsers` then lists it beside real Chrome, with a deviceId that persists in the profile across restarts. `select_browser`, `tabs_context_mcp`, `navigate`, `computer screenshot` all worked.
4. Off-screen rendering. `--off-screen-rendering-enabled` alone leaves cefclient on Chrome style and windowed rendering (and the extension's onboarding tab crashed there, error 15, not investigated since that mode is not the target). `--off-screen-rendering-enabled --use-alloy-style` gives "Alloy style; Native-hosted window; Windowless rendering", and steps 2 and 3 pass again in that mode: connect, new window, navigate, screenshot. One console error on start, `No current window` from the service worker, harmless so far.

## How to run

```
./run.sh                                             # windowed, Chrome style
./run.sh --off-screen-rendering-enabled --use-alloy-style   # windowless, the target mode
```

`run.sh` points cefclient at `profile/` (cache path) and `extension/` (both gitignored; the extension is Anthropic's, copy it from Chrome as above) and logs to `cefclient.log`. `profile/NativeMessagingHosts/` must hold the two Anthropic manifests. The profile keeps the claude.ai session and the deviceId.

## What this settles for the port

The browser card is CEF, Chrome bootstrap, Alloy-style windowless browsers, extension loaded with `--load-extension` (or `CefRequestContext` equivalents once we host it ourselves). The app writes the native messaging manifests into its own cache path at startup, copying from Chrome's directory, so a Claude Code update that rewrites the host path is picked up on the next launch.

Not checked: the extension's side panel (Alloy style has no browser UI for it; the MCP tools did not need it), `chrome.debugger` beyond what `computer` uses, several browsers at once, and the frame path out of `OnPaint`, which is the next spike.
