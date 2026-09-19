# Browser tabs and focus lock

A browser card holds one page today; using the grid system to open every page as its own card is what Ekin called painful. This gives a browser card real tabs, Chrome's own, at the cost of a mode switch: a browser card you have clicked into locks the keyboard to it, Chrome shortcuts and all, until a fast double-Escape hands it back. Decided, not up for debate; this document is the shape of it.

## What it does

A browser card can hold more than one tab. `Cmd+T` while locked opens a new one; so does a popup, a `target=_blank` link, a Cmd+click on a link, and "Open link in new tab" on the right-click menu (new item, next to the existing "Open link in new card"). A page opened as a new CARD is still available from "Open link in new card" and from a plain click when the card is not locked — the point of tabs is not to remove that, only to give the other option.

Tabs stay live the way Chrome's do: switching away from one does not tear it down. A card with six tabs is six CEF surfaces running at once, the same cost six Chrome tabs have.

**Focus lock.** The first click on an unfocused browser card only focuses it, unchanged. The next interaction — a click that reaches the page, or (added after launch: arrow-focusing a card never gives the page focus, and there was no keyboard way in without one) a bare `Enter` on a card that is focused but not yet locked — locks the keyboard to the card: every Cmd chord and `Cmd+Shift+]` / `Cmd+Shift+[` go straight to the page, real Chrome bindings, not this app's. (Superseded during implementation: `Ctrl+Tab`/`Ctrl+Shift+Tab` is Chrome's cross-platform binding, but `Cmd+Shift+]`/`Cmd+Shift+[` is what a Mac Chrome/Edge user actually reaches for, and `keymap.rs` had already reserved those two chords for `workspace.prev`/`.next` as a stand-in "browser tab" chord before real tabs existed.) `Cmd+T` new tab, `Cmd+W` closes the active tab (closes the card only when it was the last tab), `Cmd+Shift+T` reopens the last closed tab, `Cmd+1..8` jumps to tab N, `Cmd+9` the last tab, `Cmd+L` the omnibox already does this. Every one of those chords is bound to something of ours today (zoom-to-fit, `card.close`, `card.place`, workspace-adjacent) — while locked, the page's meaning wins, the same way it would if you alt-tabbed into an actual Chrome window. The one exception is `Ctrl+1..9`: workspace switching, never a Chrome shortcut, stays ours even locked, because losing the ability to switch workspaces from inside a tab is a worse trade than the alternative.

A double-Escape, both presses inside 400 ms, unlocks. A single Escape while locked reaches the page, same as Chrome (closes an autocomplete, exits fullscreen video, cancels a navigation). Once unlocked, `Cmd+T` is `card.new.terminal` again. The status bar shows a lock indicator while any card is locked, since the keyboard now means something different than it did a keystroke ago and that has to be visible without hunting for it.

A browser card that is focused but **not yet locked** (arrow-keyed to, never clicked) keeps exactly today's behaviour: the small `browser_keys.rs` list (page zoom, `Cmd+R`, `Cmd+F`, `Cmd+[`/`Cmd+]`) redirects to the page, everything else is the app's. Lock is a superset of that, not a replacement for it.

The omnibox is unaffected by lock (it is an app overlay, not the page): `Enter` navigates the active tab in place, as now. `Alt+Enter` opens what was typed in a new tab instead, matching Chrome's own binding for it (not `Cmd+Enter`, which Chrome does not use for this).

## Data model

`Card` (`infiniterm-core/src/model/mod.rs`) gains:

```rust
/// Every tab's url, active one included. Empty for a single-tab card:
/// `url` alone still says where it is, the way it always has.
pub tabs: Vec<String>,
/// Index into `tabs`. Meaningless while `tabs` is empty.
pub active_tab: usize,
```

`card.url` keeps its current meaning, the active tab's page, so nothing downstream — omnibox prefill, the palette's `Card: <label>`, `card.mask`, corner labels (now the tab strip's, see below), `ift ls` — has to learn about tabs to keep working. `card.zoom` and `card.title`/history stay singular too, the active tab's, for the same reason.

Saved (`saved_layout.rs`) only when `tabs.len() > 1`, the exact rule `session`, `kittyKeys`, `agentSession` and `number` already follow: a card that never opened a second tab writes no `tabs` or `activeTab` key and round-trips byte for byte. On load, an absent `tabs` means a single-tab card; `active_tab` defaults to 0.

`BrowserBody` (`infiniterm-ui/src/browser_body.rs`) moves from one `Option<Surface>` to a small `Vec<Tab>`, `struct Tab { surface: Option<Surface>, url: String, title: Option<String> }`, mirroring today's single-surface fields per tab instead of once. The active index is read from `card.active_tab` each `reconcile_browsers`, the way `card.zoom` already flows in every frame. Opening, closing and reordering tabs are all "the `Vec` changed since last frame," reconciled the same way `reconcile_browsers` already diffs cards against bodies.

## Focus lock, precisely

No new flag: lock **is** `page_focused`, the field that already flips when a click reaches the page rather than the scrim. `input.rs::key_down` gains one check, before the existing `handle_chord` call: if the focused card is a browser and its body is `page_focused`, skip the app's chord table (except `Ctrl+1..9`) and hand the keystroke to the body instead — that is the entire mechanism. `browser_body::key` already special-cases `k.modifiers.platform` for the edit chords; it grows to forward every Cmd chord to `Surface::key`/`edit_chord` while locked, falling back to today's narrower `browser_keys.rs` list while merely focused.

Double-Escape needs one piece of new state, a timestamp on `BrowserBody` (`last_escape_ms: Option<f64>`), cleared on unlock and on any other key. `DOUBLE_ESCAPE_MS: f64 = 400.` as the window, named the way every other timing constant in this codebase is.

## Tab strip

Drawn by `browser_body::paint`, a band at the top of the card, under the same content mask the page's texture paints under. Sized in screen pixels divided by zoom like every other piece of chrome, `ui_scale` reaches it the way it reaches card labels and the status bar. Each tab shows its page's title, elided with `crate::text::elide` rather than left to run past its own width the way the omnibox's rows once did; the active tab is visually distinct (the same selected-row treatment the omnibox and palette already use). The strip carries the card's number (`#7`) at its edge — this is what the corner label did before it was removed for exactly this reason.

Click a tab to switch (sets `active_tab`, no lock state change). A small `×` closes it (same rule as `Cmd+W`: closes the card if it was the last tab). A `+` opens a new tab at `about:blank`, the same default an empty browser card opens with today. Drag-to-reorder and a tab-specific right-click menu are not in this slice, see below.

## Where the code goes

```
infiniterm-core/src/model/mod.rs       Card.tabs, Card.active_tab
infiniterm-core/src/saved_layout.rs    conditional read/write, the same shape as session/kittyKeys
infiniterm-core/src/browser_keys.rs    unchanged: still the focused-not-locked list
infiniterm-core/src/keymap.rs          no new bindings; Cmd+T/W/1..9 etc. keep their app meanings, lock is what shadows them
infiniterm-ui/src/browser_body.rs      Vec<Tab>, the tab strip's paint, key() forwarding while locked, last_escape_ms
infiniterm-ui/src/browsers.rs          reconcile_browsers diffs tabs like it diffs cards; new-tab/close-tab plumbing; "open link in new tab" on the context menu; every tab's navigation still recorded through the existing history path
infiniterm-ui/src/input.rs             the lock check ahead of handle_chord; Ctrl+1..9 carve-out
infiniterm-ui/src/main.rs              status bar's lock indicator
```

## Not in this slice

Drag-to-reorder tabs. A tab's own right-click menu (close, close others, duplicate) — the strip's `×` covers closing for now. Tab pinning, tab groups, tab search. Restoring closed tabs across a restart (the reopen-last-closed-tab chord only reaches back to the same run).

## Tests

- `saved_layout`: a single-tab card writes no `tabs`/`activeTab` key; a two-tab card round-trips both; an old file with neither loads as one tab at index 0.
- `browser_keys`: unchanged, still tested as the focused-not-locked list.
- A pure function for the double-Escape window (`is_double_escape(first_ms, second_ms) -> bool`) rather than testing it through real keystrokes.
- The lock-vs-not-locked chord routing decision, wherever it lands as a pure function (which chord table applies given `locked: bool` and the chord), gets a test per branch: `Cmd+T` locked forwards, unlocked opens a card; `Ctrl+5` reaches the workspace switch either way.
