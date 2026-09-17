# Session daemon: persistent shells without a second emulator

2026-09-17. Replaces the tmux backend (docs/tmux-handover.md) rather than fixing it.

## Why

tmux gave us persistent shells and cost an evening of bugs. Six of the ten came from one fact: tmux is a second terminal emulator. It keeps its own grid, measures characters with its own width tables, answers `\e]11;?` before we can, and `capture-pane` hands back a flattened, padded copy of a grid rather than the bytes that made it. The open bug is the same thing again: Claude Code redraws inline, moving the cursor up and erasing exactly the rows it believes it drew, so a one row disagreement never heals. Every other TUI tested (vim, htop, btop, glances, llmfit) repaints whole screens and is immune.

abduco and dtach avoid all of it by doing no emulation, but neither can replay history, and shpool keeps a `vt100` screen so it lands in tmux's category.

We own the PTY layer, the VT parser and a unix socket already. A daemon of our own gets persistence with no emulator in the path and no history that was ever anything but the original bytes.

## What this is

A tiny sidecar binary, `iftd`, beside `ift` and the hook in the bundle. **One daemon per card**, not one server for all of them.

That is the load bearing decision. One socket per pane makes backpressure the kernel's: our ack ledger stops reading the socket, the daemon's write blocks, it stops reading the pty, the child blocks at the pty buffer. tmux bug 8, where `refresh-client -A pause` silently discarded a paused pane's output, existed only because every pane shared one socket and flow control had to be a protocol we wrote. Here there is no flow control to write.

The daemon does no VT parsing at all. It is `forkpty`, a ring of raw bytes, and one client. The bytes our emulator sees on reattach are byte for byte the bytes it would have seen live, which is the property tmux structurally cannot offer.

```
app  ──unix socket──  iftd  ──pty──  zsh / claude
     framed bytes           raw bytes
                       4 MiB ring of output
```

## Decisions

| | |
|---|---|
| Replay | 4 MiB raw byte ring per pane, replayed on attach as `PaneEvent::Replay`. Configurable. |
| Default | `terminal.backend` defaults to `daemon` during the shakedown. `pty` and `tmux` stay selectable. |
| Outside attach | `ift attach <id>` works with the app dead. That is the escape hatch. |
| Clients | One at a time. A new attach replaces the old, which is dropped. Fan-out means per client backpressure, which is the hole again. |

## The seam already exists

`backend/mod.rs` was written for a second backend and needs no new concepts:

- `PaneEvent::Replay(Vec<u8>)` exists and is documented as "bulk history replayed into a fresh emulator".
- `adopt(&str)`, `live_windows()`, `kill_orphans(&[String])`, `window_id()` are all keyed on an opaque session string. Nothing in them is tmux shaped.
- `Panes` is already an enum precisely because `SessionBackend` is not dyn compatible. It grows a third arm.

Two things in the ui must change and are easy to miss:

1. `Panes::start(want_tmux: bool)` becomes `start(backend: TerminalBackend)`.
2. `terminals.rs` gates emulator query answers on `!is_tmux()`. Under the daemon WE are the terminal and the replies must be sent. The predicate becomes `Panes::we_are_the_terminal()`, true for local and daemon, false for tmux. Getting this backwards reproduces tmux bug 9 exactly: duplicate answers arriving at the program as keystrokes.

## Files

```
infiniterm-core/src/backend/session_protocol.rs   NEW  frames + the ring, pure, tested without a daemon (mirrors tmux_protocol.rs)
infiniterm-core/src/backend/daemon.rs             NEW  the client side: spawn iftd, connect, reader thread, credit
infiniterm-core/src/backend/mod.rs                     Panes gains Daemon; start() takes TerminalBackend; we_are_the_terminal()
infiniterm-core/src/backend/local_pty.rs               terminal_identity / INHERITED_TERMINAL_VARS / default_shell become pub for iftd
infiniterm-core/src/config.rs                          TerminalBackend::Daemon, default; terminal.sessionBuffer (MiB)
infiniterm-core/src/settings_doc.rs                    the new setting, or the doc test fails
infiniterm-core/src/saved_layout.rs                    tmux_window -> session, reading tmuxWindow as a legacy alias
infiniterm-session/                               NEW  crate producing iftd
infiniterm-cli/src/main.rs                             ift attach, ift sessions
infiniterm-ui/src/terminals.rs                         we_are_the_terminal(); adopt by session id
infiniterm-ui/src/runtime.rs                           orphan sweep at launch, unchanged in shape
tools/bundle.sh                                        iftd rides along as a third sidecar
tools/drive/daemon.sh                             NEW  detach, relaunch, reattach, screenshot
```

## The protocol

Length prefixed binary frames, both directions, on one stream: `[kind: u8][len: u32 big endian][payload]`. Unlike tmux's `%output` there is no escaping, no octal, and no text to parse. A payload over 1 MiB is a corrupt stream and the connection is dropped.

App to daemon:

| kind | payload | meaning |
|---|---|---|
| `Data` | bytes | keystrokes and pastes, straight to the pty |
| `Resize` | cols u16, rows u16 | `TIOCSWINSZ` |
| `Kill` | none | terminate the child, unlink the socket, exit |

Daemon to app:

| kind | payload | meaning |
|---|---|---|
| `Hello` | pid u32, cols u16, rows u16 | first frame on every attach; the pid feeds `pids_source` |
| `Replay` | bytes | the ring, in as many frames as it takes |
| `ReplayEnd` | none | everything after this is live |
| `Data` | bytes | live output |
| `Exited` | code i32 | the child is gone; the daemon exits after this |

No ack frames. The socket itself is the backpressure.

## The ring

A fixed 4 MiB buffer of raw output bytes per daemon, filled whether or not a client is attached, because a detached daemon that stopped reading would hang the child forever.

When it overflows it drops from the front up to the next `\n`, so a replay never begins in the middle of a line. Replay is prefixed with `\e[0m` so a truncated front cannot leave the first line wearing colours set before the cut.

Known imperfection, accepted for v1: state set before the cut and never set again is lost. Colours are handled by the reset above. Modes are not: if the ring cuts while a program is in the alternate screen, the replay has no enter sequence. Two things make this narrow rather than serious:

1. It only affects content older than 4 MiB of output.
2. **After `ReplayEnd` the client resizes the pane by one column and back.** The `SIGWINCH` makes the program repaint its current screen over whatever the replay left. Ink, vim, htop and every full screen program self correct within a frame. A shell redraws its prompt.

The upgrade path, if 4 MiB proves too shallow for a Claude card (Ink repaints are large and repetitive): parse what is about to be dropped into a headless grid with our own parser and serialise that grid back to bytes as the new front of the ring. Deferred, not designed here.

## Lifecycle

**Spawn.** The app runs `iftd --socket <path> --cwd <dir> [--cmd <s>] --env K=V ...`. iftd binds the listener **before** it forks, then double forks and the original process exits 0. The app waits on that exit and then connects, so there is no readiness race and nothing to poll. Env scrubbing (`INHERITED_TERMINAL_VARS`, `terminal_identity`) happens in iftd, because iftd is what spawns the child.

**Sockets** live at `<data>/sessions/<id>.sock`, with `<id>.meta` beside them holding one JSON line: pid, cwd, cmd, started. Because they are under the data dir, `INFINITERM_DATA_DIR` isolates a scratch instance for free. tmux needed a `session_name()` with a dev and a real name to get the same property.

**Detach** is closing the socket. The daemon keeps reading into the ring and waits on `accept()` again.

**Quit** (`Panes::leave`) closes every socket and leaves every daemon running. That is the whole point.

**Close a card** sends `Kill`. Killing a card kills its shell, as it does today.

**Adopt.** `live_windows()` scans `<data>/sessions/*.sock` and tries to connect to each; one with no listener is stale and is unlinked. `adopt(id)` connects and takes `Hello` + `Replay` + `ReplayEnd`.

**Orphans.** At launch, any live session no card claims gets a `Kill`. Same sweep as tmux's, on a directory we own.

## Attaching from outside

`ift sessions` lists what is running: id, pid, cwd, command, started. It reads `<data>/sessions/*.meta` directly and needs no app, which is the point.

`ift attach <id>` connects to that socket, puts stdin in raw mode, forwards stdin as `Data` and `Data` to stdout, sends `Resize` on `SIGWINCH`, and restores the termios on exit. `Ctrl-\` detaches. It is the same client code the app uses, one screen of glue on top.

Both work with the app dead or crashed. That was tmux's one real advantage over a daemon of our own, and it costs about two hours to keep.

## Testing

Pure, no daemon needed:

- frame round trip; a frame split across two reads; a truncated frame; an oversized length rejected.
- ring: fills, wraps, drops at a newline boundary, replays in order, never exceeds its cap.

With a real daemon, on a temp data dir, never the real one:

- spawn, write `echo hi\n`, read `hi` back.
- detach, reattach, assert the replay contains `hi` and `ReplayEnd` follows it.
- kill, assert the socket is gone and the child is reaped.
- a daemon whose client never reads: assert the child stalls rather than the daemon growing without bound.

On screen, which is where the premise actually gets tested:

- `tools/drive/apps.sh` already runs one take under two configs. A third, `daemon`, gives a three way A/B of the same TUI programs.
- `tools/drive/daemon.sh`: run Claude in a card, quit the app, relaunch, screenshot. This is the falsification test for the entire design. If Claude comes back corrupted here, the problem was never tmux and the ring replay does not save us.

## Risks

1. **The premise could be wrong.** If Claude corrupts under our own replay too, the fault is in our widths or our parser, not in tmux. `daemon.sh` finds this out on day one rather than on day three.
2. **N processes.** 25 cards is 25 daemons: roughly 2 MB RSS each plus the ring, so about 150 MB worst case with every ring full. `terminal.sessionBuffer` exists so it can be turned down.
3. **A reboot still kills everything.** Known since the persistent sessions discussion and not addressed here.
4. **`we_are_the_terminal` inverted** reproduces tmux bug 9. Called out in the plan as its own test.

## Not in this

Bookmarks of sessions, session names chosen by a human, sharing one session between two cards, remote hosts, and the grid snapshot compaction above.
