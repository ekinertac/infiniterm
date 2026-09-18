# Browser card: input and feature gaps

Written 2026-09-18, after reading every CEF handler trait cef-rs exposes (`ImplRenderHandler` through `ImplServerHandler`, ~28 total) against what `infiniterm-browser/src/surface.rs` wires up. Only five are implemented: Render, LifeSpan, Display, Load, Find. Everything below traces to that gap or to a specific line, not to guessing. Owner: whoever picks up `infiniterm-browser/` and `infiniterm-ui/src/browser_body.rs` / `browsers.rs` next; check with the other session working this repo before touching those files, per `CLAUDE.md`.

Ekin's call: skip permission prompts (mic/camera/location), Cmd+P/print, and the accessibility tree for now. Context menu is last on the list and stays slim: not a rebuild of Chrome's menu.

## Confirmed, exact line found

1. **Hover is broken.** `surface.rs:398-402`, `Surface::mouse_move`, sends CEF's `mouseLeave` flag as `(!left) as i32` where `left` is `left_down` (button state, not leave state). Every ordinary hover move has `left_down == false`, so `mouseLeave = 1` fires on every move: CEF is told the mouse left the browser on every single move. Only mid-drag (`left_down == true`) does it read correctly. Fix: pass `0` on a normal move; add a real leave event for when the pointer exits the card (nothing calls this today either).

2. **Backspace / Delete / Arrows / Home / End don't work in text fields.** `surface.rs::key`, every `KeyEvent` is built with `..Default::default()`, so `native_key_code` is always `0`. macOS resolves editing commands (delete, cursor move, select-word) through Cocoa's key-binding tables, keyed on the real macOS virtual keycode, not `windows_key_code`. `0` is the keycode for the `A` key, so the lookup silently resolves to nothing. Typing plain characters still works because those go through CHAR events, a separate path that doesn't need this. `infiniterm-ui/src/keycode.rs` already captures the real NSEvent keycode for the terminal keymap; it just isn't threaded through to the browser. Fix: add a `name -> macOS virtual keycode` table (or reuse `keycode::last_code()`) and set `native_key_code` when forwarding named keys.

## Todo, in order

1. ~~Fix hover~~ — done.
2. ~~Fix backspace/delete/arrows/home/end~~ — done. Known side effect: doubles arrow-key moves inside a page that handles keys itself and doesn't fully block CEF's native Cocoa editing-command fallback (hit in a Monaco/VS Code Web test); left as-is, narrow case, reverting would re-break plain inputs.
3. ~~Redo~~ — done.
4. ~~Cursor shape~~ — done.
5. HTML5 drag-and-drop: implement `start_dragging` / `update_drag_cursor` on `RenderHandler`. OSR has no default for these; without them a page can't run any native drag (dragging an image/link/selection out, or a page's own drag-reorder UI). Separate from the app's own file-drop handling in `drop.rs`, which is unaffected.
6. HTTP auth + bad certificates: `RequestHandler`, two callbacks. `GetAuthCredentials` for a Basic/Digest login box; `on_certificate_error` for a proceed-anyway prompt. Both fail silently today (auth: load just fails; cert: load just fails, no override).
7. Context menu, slim: `ContextMenuHandler`. Build the menu model from CEF's `ContextMenuParams` flags, not a full Chrome menu. Contents: Back / Forward / Reload; Cut/Copy/Paste when the target is editable; Copy when there's a selection; Copy link address plus "open link in new card" (reuses the existing popup-to-card path in `life_span_handler`) when over a link. No inspect element, no text-services submenu. Last, as asked.

## Explicitly skipped

- Permission prompts (mic/camera/location/notifications) — needs `PermissionHandler`, not built.
- Print (`Cmd+P` / `window.print()`) — needs `PrintHandler`, not built.
- Accessibility tree (VoiceOver sees nothing on a page) — needs `AccessibilityHandler`, not built.

## Verified already working, no handler needed

Cmd+A/C/V/X/Z in inputs (`edit_chord`, direct CEF frame API calls, no global keybinding intercepts them first); click-through focus; double/triple-click word/line select; middle-click opening a popup as a card; Tab; Enter; scroll with modifiers; drag-select of text. All go through the handlers already implemented or through plain mouse-event forwarding, none of which need anything on the todo list above.

Confirmed on screen by Ekin, 2026-09-19, so also dropped from the todo without writing a handler: JS dialogs (`alert()`/`confirm()`/`prompt()`), downloads, and the file input picker (`<input type="file">`) all already work under CEF's OSR defaults. The assumption in the original audit, that no handler meant broken, held for hover, the native keycode and cursor shape but not for these three.
