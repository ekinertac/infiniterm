# Remote canvas: one infiniterm shown on another Mac

Ekin left the mini at home for 15 days and worked from the Air through `wezterm connect mini`: a second WezTerm window whose tabs were the mini's live sessions, every project already bootstrapped there. This is the same thing for infiniterm: `ift connect mini` on the Air opens a window that shows the mini's canvas, cards, colours and all, and what he types there runs on the mini.

Both apps must be running. There is no offline mode and no "drive the daemons without the app": the host's app is the one authority and the viewer talks to it. Any Mac running infiniterm is a host; the direction is whatever the ssh host name says.

## Words

- **host**: the Mac whose canvas is shown. Its app owns the model, the save file, the hooks and the daemons.
- **viewer**: the app instance on the other Mac that shows the host's canvas. It owns a window and a camera and nothing else.

## What the viewer is

A viewer is the ordinary app started with `INFINITERM_DATA_DIR=<data>/remote/<host>` (so the socket lock and the save file of the local canvas are untouched) and `INFINITERM_REMOTE=<ssh host>`. In that mode:

- Its model is a **replica**: it is loaded from the host's save file and reloaded, keeping the viewport, every time the host pushes a new one. Nothing in `model/` learns about viewers; the load path is the restart's.
- It **executes no command**. A chord resolves to a command id as today (`keymap.rs`, `handle_chord`), and the id is sent to the host as an `ift` request; the host runs it against its own model and the next push shows the result. A palette pick and a menu item are commands too. A label drag's drop and a resize's release are the one thing with no command id, a rect, so they go up as a new verb, `place <card> <x> <y> <w> <h>`, which the host runs through `drop_card` and the resize path with the undo trail as if the pointer had been its own. The mouse on bare canvas (pan, zoom) is the camera and stays local.
- **Focus is shared.** One person, so the host's focused card is the one the viewer's keys go to and there is no per-viewer focus to invent. The host's screen following along is accepted: from afar it cannot be seen, and at home there is one of Ekin.
- **The camera is the viewer's.** The Air's window is a different size; the host's viewport is never read past the initial load and never written.
- Terminal bodies are the real `TerminalBody` over a **remote pane**: bytes from the host card's `iftd`, keys back to it. The pty size is the host's; the viewer never resizes it. If the viewer's `terminal.fontSize` differs, the grid is drawn at the viewer's cell size and may not fill the card edge to edge. Said so, not fixed.
- Browser, editor, diff and transcript cards draw a **placeholder** body: the card's frame, label and agent colour as usual, and the words "browser on mini" in the middle. A browser is a CEF texture, the rest read files on the host's disk. Terminals are the vacation case; the rest can follow one kind at a time if they turn out to matter.
- The title bar carries the host's name and a tint (`chrome.remote_bg`, the colour already reserved for "not yours"), so the viewer window is never mistaken for the local canvas, the same reason the WezTerm config recolours a mux window.

## Transport: ssh, as WezTerm does it

No listener on the host, no port forward, no TLS. Everything rides `ssh -T <host> <app bundle>/Contents/MacOS/ift proxy <what>`, the proxy splicing its stdin/stdout to a unix socket on the host. The user's `~/.ssh/config` (`mini-m4`, `air`) does the auth and the Tailscale address. `ControlMaster auto` is set by the viewer on its own ssh invocations (`-o ControlMaster=auto -o ControlPath=<data>/remote/<host>/cm-%C -o ControlPersist=60`) so N proxies are one TCP connection; without a master each proxy is its own handshake.

Two socket kinds are proxied:

1. `ift proxy app`: the host's app socket (`paths::socket_path()`). One connection per viewer, held open for the whole session. Carries the feed down and the requests up.
2. `ift proxy session <id>`: `<data>/s/<id>.sock`, one per visible terminal card, as a viewer client of that `iftd` (below).

`ift proxy` is dumb: `copy(stdin → socket)` and `copy(socket → stdout)` on two threads, exit when either side closes. It does not parse anything. The absolute path to `ift` on the host is `/Applications/infiniterm.app/Contents/MacOS/ift`; a setting `remote.iftPath` overrides it for a Mac that installed elsewhere.

## The feed, on the app socket

A new request `subscribe` on the app socket turns that connection into a feed. Today a request is one line in, one line out; a subscribed connection is left open and the app writes a line to it whenever it would write the save file, and once immediately on subscribe. Each line is:

```
{"layout": <the save file's JSON>, "agents": {"<card id>": "working" | "waiting" | "done", ...}, "focused": "<card id>"}
```

`layout` is exactly `saved_layout::save_text`'s output, so the viewer feeds it to the same loader. `agents` is there because agent state is runtime and not in the save file; only cards with a state are listed. `focused` is in the save file already (`Workspace::focused` for the active workspace) but is named again so the viewer does not have to dig.

The host pushes on the layout's `SAVE_DEBOUNCE_MS` and additionally on every agent state change (those set no `dirty_layout` today; the hook path sets a `dirty_feed` beside it). A push is a few KB; the debounce keeps a drag from sending sixty a second.

Requests up the same connection are today's `CliRequest` lines: `ift`'s verbs, and two new ones, `command <id>`, which runs a registered command as a chord would (`dev-run` already does this behind the development-build gate; `command` is it without the gate, and `dev-run` stays as the harness's name for it), and `place`. Replies come back with the correlation id as they do now. The viewer sends `command` for chords and palette picks and the existing verbs where the driver already has one.

The socket thread in `hooks.rs` today reads a line and answers it. A subscribed connection is handed to a small writer thread that owns the connection's write half and receives feed lines and replies over a channel. `CliState` grows a list of feed senders; `write_layout` and the hook path send to each; a send that fails drops that feed (the viewer went away).

## `iftd` viewer clients

`iftd` serves one client and a new connection evicts the old, on purpose: eviction is how `ift attach` takes a card, and the app's read blocking the daemon's write is the backpressure chain. A viewer is neither.

A connection that opens with a `viewer` hello (a new first frame in `session_protocol.rs`; the absence of it keeps every existing client as it is) becomes a **viewer** of the session:

- It does not evict the attached client and is not evicted by an attach.
- It receives the ring replay first, as an attach does, so the card is drawn in full at once, then live bytes.
- Its writes are **best-effort**. `iftd` keeps a bounded queue per viewer (`VIEWER_QUEUE_BYTES`, 1 MiB); a viewer that falls further behind is closed. It reconnects and gets a replay, which is the resync. The attached client's chain is untouched: a slow viewer never blocks the pty read.
- Its input frames go to the pty exactly as the attached client's do. There is one pty; whoever types, types.
- It sends no resize. `Resize` frames from a viewer are ignored and logged once.
- `Meta` gains `viewers: N`, informational; `ift sessions` shows it.

The queue is bounded in bytes, not frames, because a Claude redraw is many small frames and one big one alike.

## The viewer's backend

`backend/remote.rs`, a fourth `Panes` variant beside pty, tmux and daemon. `spawn` is never called: cards come from the replica with their `session` handle set, and `adopt(session_id)` is what `terminals.rs` reaches for, as under the daemon backend after a restart. `adopt` starts `ssh -T <host> ift proxy session <id>` with the ControlMaster options, sends the `viewer` hello, and from there the child's stdout is the pane's byte stream and stdin its input, on the same tagged event stream the other backends feed. A closed child is `PaneEvent::Detached`; `terminals.rs` re-adopts after a second the way it does for a displaced card, which is also the resync.

`we_are_the_terminal()` is true: the viewer's emulator answers queries. That is wrong when two emulators answer one program, the host's and the viewer's. So a viewer pane **suppresses replies** (`TerminalBody::replies` gated off for remote panes): the host's emulator is the terminal, the viewer only watches and types. Get this backwards and every `CSI ? u` query gets two answers (tmux bug 9 by a third road).

Keys typed on the viewer go down the pane's input as encoded bytes, the same bytes the local encoder produces, because the kitty flag the card saved travels in the layout and the viewer's `Grid` sees the same replayed preamble.

## `ift connect <host>`

```
ift connect mini            # open a viewer window onto the host at ssh alias `mini`
ift connect mini --check    # ssh in, ask the host's app for its version, print it, exit 0/1
```

`connect` checks that the local app bundle exists, runs `ssh -T <host> ift proxy app` once to confirm the host's app is up (a request `version`; a mismatch is refused with both versions named, because a replica loaded by a different build is the read-only-mode problem in a new coat), then `open -n /Applications/infiniterm.app` with `INFINITERM_DATA_DIR=<data>/remote/<host>` and `INFINITERM_REMOTE=<host>` in the environment (`open --env`). It runs in the foreground until the viewer window closes, like `wc-mini`, so Ctrl-C tears it down and there is no pid to hunt. The viewer window closing detaches everything; nothing on the host notices beyond `viewers` dropping.

The Makefile's `install` already ships `ift` beside the app, so the host side needs no install step beyond having the same build.

## Where the code goes

- `infiniterm-cli/src/proxy.rs`: `ift proxy app|session <id>`. `connect.rs`: the launcher.
- `infiniterm-core/src/hooks.rs` + `cli.rs`: the `subscribe` request, feed senders, the writer thread per subscribed connection. `model/ift_in.rs`: `command`, `place`, `version`.
- `infiniterm-core/src/model/persist.rs` + `hooks_in.rs`: `dirty_feed`; `feed_line()` building the JSON from `save_text` and the cards' states.
- `infiniterm-core/src/backend/remote.rs` + `session_protocol.rs`: the `viewer` hello, the `Remote` variant of `Panes`.
- `infiniterm-session/src/main.rs`: viewer connections, the bounded queue, `viewers` in `Meta`.
- `infiniterm-ui/src/runtime.rs`: remote mode startup (read `INFINITERM_REMOTE`, start the feed connection, load replicas), `apply_replica(line)` keeping the viewport; command forwarding in `input.rs` where chords are dispatched; `placeholder_body.rs` for the non-terminal kinds; the title bar's host name and tint in `overlays.rs`.
- `settings_doc.rs`: `remote.iftPath`.

## Not in this slice

- Any card kind but the terminal, past the placeholder.
- A viewer resizing the pty, or a per-viewer font.
- Two viewers on one host at once (nothing forbids it; nothing is tested for it).
- Connecting to a host whose app is down.
- A mux protocol replacing one ssh process per card. Ceiling: one process per visible terminal card, on one master; 13 cards is fine, a hundred would want the mux.
- The viewer writing anything to the host's disk: it has no save file of its own to write and sends no layout.

## Tests

- `session_protocol`: the `viewer` hello round-trips; a connection without it is the old client.
- `iftd`: a viewer does not evict the attached client; an attach does not evict a viewer; a viewer that stops reading is closed after `VIEWER_QUEUE_BYTES` while the attached client keeps receiving; a viewer's input reaches the pty; a viewer's resize is ignored.
- `hooks.rs`: `subscribe` receives a line at once and one on each save; a dropped subscriber is removed; a `command` request runs the registered command (Harness).
- `persist.rs`: `feed_line` carries every card with a state and none without.
- `remote.rs`: an `adopt` over a fake proxy (a child that echoes) produces the pane's events; a closed child is `Detached`.
- Runtime: `apply_replica` on a model with a viewport keeps the viewport and replaces the cards; a replica whose card kind is not a terminal gets the placeholder body.
- `ift connect --check` against a scratch instance (`tools/drive/ift.sh` has the pattern) with `ssh` replaced by a script that runs the proxy locally; the same stand-in drives a headless end-to-end: a viewer instance on a second data dir shows the scratch host's cards, a command sent from the viewer moves a card on the host, the next feed line shows it.
