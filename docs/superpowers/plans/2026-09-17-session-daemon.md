# Session daemon implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cards keep their shells when infiniterm quits, with no second terminal emulator anywhere in the path.

**Architecture:** One sidecar daemon per card (`iftd`), holding a pty and a 4 MiB ring of the raw output bytes, talking length-prefixed binary frames over a unix socket at `<data>/sessions/<id>.sock`. The daemon never parses VT. On reattach it replays the ring, and the client then resizes by a column and back so the program repaints over it.

**Tech Stack:** Rust, portable-pty (already a dependency), libc for fork/setsid and termios. No new runtime dependencies beyond libc.

**Spec:** `docs/superpowers/specs/2026-09-17-session-daemon-design.md`

## Global Constraints

- Every file starts with a header block: responsibility, where it fits, what calls it, related files, constraints. Comments say why. Numbers are named constants with the reason beside them.
- No attribution trailers of any kind in commit messages. Messages say WHY.
- `make check` (fmt on the app crates only, `clippy -D warnings`, `cargo test`) must pass before each commit. Never `cargo fmt --all`.
- Re-read a file after `cargo fmt` before patching it; a patch built from a stale copy silently matches nothing.
- No test may bind `/tmp/infiniterm.sock` or write to the real data dir. Tests use `tempfile`-style temp dirs and pass paths in.
- Frame payloads are capped at `MAX_PAYLOAD = 1 MiB`. A larger length is a corrupt stream, not a big message.
- `terminal.backend` accepts `"pty"`, `"tmux"`, `"daemon"` and defaults to `"daemon"` at the end of this plan, not before.
- The tmux backend is not deleted by this plan. It stays selectable.

---

### Task 1: The frame protocol and the ring

Pure logic, no daemon and no socket. Mirrors `tmux_protocol.rs`, which is the file to read first for the house style.

**Files:**
- Create: `infiniterm-core/src/backend/session_protocol.rs`
- Modify: `infiniterm-core/src/backend/mod.rs` (add `pub mod session_protocol;` beside the tmux ones)

**Interfaces:**
- Produces: `Frame` (the enum below), `Frame::encode(&self) -> Vec<u8>`, `FrameReader::default()`, `FrameReader::feed(&mut self, &[u8])`, `FrameReader::next(&mut self) -> Result<Option<Frame>, ProtoError>`, `Ring::new(cap: usize)`, `Ring::push(&mut self, &[u8])`, `Ring::replay(&self) -> Vec<u8>`, `MAX_PAYLOAD`.
- Consumes: nothing.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_survives_a_round_trip() {
        for f in [
            Frame::Data(b"ls -la\n".to_vec()),
            Frame::Resize { cols: 120, rows: 40 },
            Frame::Kill,
            Frame::Hello { pid: 4321, cols: 80, rows: 24 },
            Frame::Replay(vec![0, 27, 255]),
            Frame::ReplayEnd,
            Frame::Exited(-1),
        ] {
            let mut r = FrameReader::default();
            r.feed(&f.encode());
            assert_eq!(r.next().unwrap(), Some(f.clone()), "{f:?}");
            assert_eq!(r.next().unwrap(), None, "nothing left after {f:?}");
        }
    }

    // A socket read boundary lands wherever it lands; a frame split across
    // two reads must not be lost or misread.
    #[test]
    fn a_frame_split_across_reads_is_rejoined() {
        let bytes = Frame::Data(b"hello".to_vec()).encode();
        let mut r = FrameReader::default();
        for chunk in bytes.chunks(1) {
            r.feed(chunk);
        }
        assert_eq!(r.next().unwrap(), Some(Frame::Data(b"hello".to_vec())));
    }

    #[test]
    fn a_partial_frame_is_not_a_frame_yet() {
        let bytes = Frame::Data(b"hello".to_vec()).encode();
        let mut r = FrameReader::default();
        r.feed(&bytes[..4]);
        assert_eq!(r.next().unwrap(), None);
    }

    // A length nothing could have meant means the stream is not ours.
    #[test]
    fn an_oversized_length_is_an_error() {
        let mut r = FrameReader::default();
        let mut bad = vec![KIND_DATA];
        bad.extend((MAX_PAYLOAD as u32 + 1).to_be_bytes());
        r.feed(&bad);
        assert!(matches!(r.next(), Err(ProtoError::TooLarge(_))));
    }

    #[test]
    fn an_unknown_kind_is_an_error() {
        let mut r = FrameReader::default();
        r.feed(&[200, 0, 0, 0, 0]);
        assert!(matches!(r.next(), Err(ProtoError::Kind(200))));
    }

    #[test]
    fn a_ring_under_its_cap_replays_everything_it_was_given() {
        let mut ring = Ring::new(1024);
        ring.push(b"one\n");
        ring.push(b"two\n");
        assert_eq!(ring.replay(), b"\x1b[0mone\ntwo\n".to_vec());
    }

    // Anything older than the cap is gone, and what is left starts at a line
    // boundary so a replay never begins halfway through one.
    #[test]
    fn an_overflowing_ring_drops_whole_lines_from_the_front() {
        let mut ring = Ring::new(16);
        ring.push(b"aaaa\nbbbb\ncccc\ndddd\n");
        let out = ring.replay();
        assert!(out.starts_with(b"\x1b[0m"), "the reset is always first");
        let body = &out[4..];
        assert!(body.len() <= 16);
        assert!(body.starts_with(b"bbbb\n") || body.starts_with(b"cccc\n"), "got {body:?}");
        assert!(body.ends_with(b"dddd\n"), "the newest output is never the part dropped");
    }

    // Binary output has no newlines to cut at; the ring must still be bounded
    // rather than scanning itself to death looking for one.
    #[test]
    fn a_ring_with_no_newlines_is_still_bounded() {
        let mut ring = Ring::new(64);
        for _ in 0..100 {
            ring.push(&[0xffu8; 32]);
        }
        assert!(ring.replay().len() <= 64 + 4);
    }
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p infiniterm-core session_protocol`
Expected: FAIL, the module does not exist.

- [ ] **Step 3: Write the module**

Header block first, in the style of `tmux_protocol.rs`: this is the wire between the app and `iftd`, it is pure so it can be tested without a daemon, and the reason it is length-prefixed binary is that tmux's text-and-octal `%output` cost us a day.

```rust
pub const MAX_PAYLOAD: usize = 1024 * 1024;
const HEADER: usize = 5; // kind + u32 length

pub const KIND_DATA: u8 = 1;
const KIND_RESIZE: u8 = 2;
const KIND_KILL: u8 = 3;
const KIND_HELLO: u8 = 4;
const KIND_REPLAY: u8 = 5;
const KIND_REPLAY_END: u8 = 6;
const KIND_EXITED: u8 = 7;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    Data(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Kill,
    Hello { pid: u32, cols: u16, rows: u16 },
    Replay(Vec<u8>),
    ReplayEnd,
    Exited(i32),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProtoError {
    Kind(u8),
    TooLarge(usize),
    Malformed(&'static str),
}
```

`encode` writes `[kind][len: u32 be][payload]`, where `Resize` is four bytes, `Hello` is eight, `Exited` is four, and `Kill`/`ReplayEnd` are empty.

`FrameReader` holds a `Vec<u8>`; `next` returns `Ok(None)` while fewer than `HEADER + len` bytes are buffered, and drains exactly one frame otherwise. Errors are terminal: the caller drops the connection.

The ring:

```rust
/// How far into the buffer to look for a line boundary after a trim.
///
/// Without a bound this is O(buffer) on every push once full, which a pane
/// printing binary (no newline anywhere) pays on all 4 MiB forever. Past
/// this we accept a raw cut: the replayed first line is then partial, which
/// costs one line of history and nothing else.
const LINE_SCAN: usize = 64 * 1024;

pub struct Ring {
    buf: std::collections::VecDeque<u8>,
    cap: usize,
}
```

`push` extends and then trims: drop `len - cap` bytes, then drop forward to just past the first `\n` within `LINE_SCAN`. `replay` returns `\x1b[0m` followed by the buffer, because a front that was cut may have been wearing colours set before the cut.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p infiniterm-core session_protocol`
Expected: PASS, 8 tests.

- [ ] **Step 5: Commit**

```bash
git add infiniterm-core/src/backend/session_protocol.rs infiniterm-core/src/backend/mod.rs
git commit -m "The wire and the ring for our own session daemon

Length-prefixed binary frames rather than tmux's text-and-octal %output:
there is no escaping to get wrong and no line framing to desynchronise.
The ring holds raw output bytes, so a replay is the same bytes our parser
would have seen live, which is the one thing capture-pane could not do."
```

---

### Task 2: `iftd`, the daemon

**Files:**
- Create: `infiniterm-session/Cargo.toml`, `infiniterm-session/src/main.rs`
- Modify: `Cargo.toml` (workspace members)
- Test: `infiniterm-session/tests/roundtrip.rs`

**Interfaces:**
- Consumes: `infiniterm_core::backend::session_protocol::{Frame, FrameReader, Ring}`, and `infiniterm_core::backend::local_pty::{terminal_identity, default_shell, INHERITED_TERMINAL_VARS}` (made `pub` in this task).
- Produces: the binary `iftd`, invoked as
  `iftd --socket <path> --cwd <dir> [--cmd <string>] [--buffer <MiB>] [--env K=V]...`
  It binds the socket, forks twice, and the first process exits 0. When it exits 0 the socket is bound and connectable. Any other exit code means it never started.

- [ ] **Step 1: Make the env helpers public**

In `infiniterm-core/src/backend/local_pty.rs`, change `const INHERITED_TERMINAL_VARS` and `fn default_shell` to `pub`. `terminal_identity` is already `pub`. Add to the header block: iftd spawns the child now, so the scrubbing lives here and is used from there. Without it every card claims to be a WezTerm pane again, which is a trap already recorded in CLAUDE.md.

- [ ] **Step 2: Write the failing integration test**

`infiniterm-session/tests/roundtrip.rs`. It runs the real binary against a temp directory.

```rust
use infiniterm_core::backend::session_protocol::{Frame, FrameReader};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

/// Runs iftd and returns a connected stream. The socket is in a temp dir, so
/// this can never reach the real one at <data>/sessions.
fn start(dir: &std::path::Path, name: &str) -> UnixStream {
    let sock = dir.join(format!("{name}.sock"));
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_iftd"))
        .args(["--socket", sock.to_str().unwrap(), "--cwd", "/tmp"])
        .status()
        .expect("iftd runs");
    assert!(status.success(), "iftd bound the socket before it forked");
    UnixStream::connect(&sock).expect("connectable the instant iftd exits")
}

/// Reads frames until `want` matches one, or the deadline passes.
fn wait_for(s: &mut UnixStream, mut want: impl FnMut(&Frame) -> bool) -> Frame { /* 5s deadline, read into FrameReader */ }

#[test]
fn a_shell_starts_and_echoes() {
    let dir = tempdir();
    let mut s = start(dir.path(), "a");
    let hello = wait_for(&mut s, |f| matches!(f, Frame::Hello { .. }));
    assert!(matches!(hello, Frame::Hello { pid, .. } if pid > 0));
    s.write_all(&Frame::Resize { cols: 80, rows: 24 }.encode()).unwrap();
    s.write_all(&Frame::Data(b"echo ready-1\n".to_vec()).encode()).unwrap();
    let seen = collect_output(&mut s, "ready-1");
    assert!(seen.contains("ready-1"));
}

// The whole point: the shell outlives the client.
#[test]
fn a_reattach_replays_what_was_missed() {
    let dir = tempdir();
    let mut s = start(dir.path(), "b");
    wait_for(&mut s, |f| matches!(f, Frame::Hello { .. }));
    s.write_all(&Frame::Data(b"echo marker-42\n".to_vec()).encode()).unwrap();
    collect_output(&mut s, "marker-42");
    drop(s); // detach

    let mut again = UnixStream::connect(dir.path().join("b.sock")).unwrap();
    let mut replay = Vec::new();
    // Hello, then Replay frames, then ReplayEnd.
    let text = read_until_replay_end(&mut again, &mut replay);
    assert!(text.contains("marker-42"), "the ring came back: {text}");
}

#[test]
fn kill_ends_the_child_and_removes_the_socket() {
    let dir = tempdir();
    let mut s = start(dir.path(), "c");
    let Frame::Hello { pid, .. } = wait_for(&mut s, |f| matches!(f, Frame::Hello { .. })) else { unreachable!() };
    s.write_all(&Frame::Kill.encode()).unwrap();
    // The daemon unlinks on its way out.
    assert!(wait_until(|| !dir.path().join("c.sock").exists()), "socket gone");
    assert!(wait_until(|| !process_alive(pid)), "child gone");
}

// A client that stops reading must stall the child, not grow the daemon.
#[test]
fn a_client_that_never_reads_stalls_the_child() {
    let dir = tempdir();
    let mut s = start(dir.path(), "d");
    wait_for(&mut s, |f| matches!(f, Frame::Hello { .. }));
    s.write_all(&Frame::Data(b"yes\n".to_vec()).encode()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let rss = rss_kb(daemon_pid(dir.path(), "d"));
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert!(rss_kb(daemon_pid(dir.path(), "d")) < rss + 8192, "bounded, not growing");
}
```

Write the helpers (`tempdir`, `wait_until`, `collect_output`, `process_alive`, `rss_kb`) at the bottom of the file; they are a few lines each over `std::process::Command` and `/bin/ps`.

- [ ] **Step 3: Run them and watch them fail**

Run: `cargo test -p infiniterm-session`
Expected: FAIL, the crate does not exist.

- [ ] **Step 4: Create the crate**

`infiniterm-session/Cargo.toml`:

```toml
[package]
name = "infiniterm-session"
version.workspace = true
edition.workspace = true
publish.workspace = true

# `iftd`, beside `ift`: one daemon per card, spawned by the app and outliving it.
[[bin]]
name = "iftd"
path = "src/main.rs"

[dependencies]
infiniterm-core = { path = "../infiniterm-core" }
portable-pty = "0.9"
# fork/setsid, and nothing else. A daemon that must survive its parent has
# no portable spelling.
libc = "0.2"

[dev-dependencies]
infiniterm-core = { path = "../infiniterm-core" }
```

Add `"infiniterm-session"` to the workspace `members` in the root `Cargo.toml`.

- [ ] **Step 5: Write the daemon**

Header block: this is one card's shell, outliving the app. It does NO VT parsing, which is the entire reason it exists rather than tmux. Related: `session_protocol.rs` for the wire, `backend/daemon.rs` for the other end, `docs/superpowers/specs/2026-09-17-session-daemon-design.md` for why.

Order in `main`, and the order matters:

1. Parse args. Unknown flag, or a socket path that already exists and is connectable: exit non-zero with a message on stderr.
2. `UnixListener::bind(socket)` **before any fork**, so the socket exists the moment the first process exits and the app never polls for readiness.
3. Daemonise:

```rust
// Bound already, so the app's wait() returning is proof the socket is there.
// Two forks: the first frees the app's wait(), setsid() leaves its session
// so a terminal closing cannot HUP us, the second makes reparenting to init
// permanent (a session leader could reacquire a controlling terminal).
unsafe {
    if libc::fork() != 0 { std::process::exit(0); }
    libc::setsid();
    if libc::fork() != 0 { std::process::exit(0); }
}
```

4. Open the pty at 80x24, build the command exactly as `local_pty::spawn_now` does (`-lc` when `--cmd` is given, `INHERITED_TERMINAL_VARS` removed, `terminal_identity()` applied, then `--env` pairs), and spawn. On failure: write `{"error": "..."}` to `<socket>.meta`, exit 1.
5. Write `<socket with .sock replaced by .meta>`: one JSON line, `{"pid":N,"cwd":"...","cmd":"...","started":"<rfc3339>"}`. `ift sessions` reads this with no app running.
6. Run four threads:

| thread | does |
|---|---|
| pty reader | `read` → `ring.lock().push(&buf)` → `client.lock()` write `Frame::Data`. A write error clears the client and keeps going: detached is normal. |
| accept | `accept()` → `shutdown(Both)` on the previous stream so its reader thread ends → store the new one → send `Hello`, then `Replay` in `MAX_PAYLOAD` chunks, then `ReplayEnd` → spawn the client reader below |
| client reader | `FrameReader` over the stream: `Data` → pty writer, `Resize` → `master.resize`, `Kill` → kill the child and exit the process. EOF clears the client and ends this thread. |
| child waiter | `child.wait()` → send `Frame::Exited(code)` if a client is there → unlink the socket and the meta file → `exit(0)` |

The ring is `Arc<Mutex<Ring>>`, the client is `Arc<Mutex<Option<UnixStream>>>`. Nothing else is shared.

There is deliberately no ack frame anywhere. When the app stops reading, this daemon's write blocks, so it stops reading the pty, so the child blocks on the kernel's pty buffer. That chain IS the backpressure, and it is why tmux's `refresh-client -A pause` (which discards) has no counterpart here.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p infiniterm-session`
Expected: PASS, 4 tests. They take a few seconds; they run real shells.

- [ ] **Step 7: Commit**

```bash
git add infiniterm-session Cargo.toml infiniterm-core/src/backend/local_pty.rs
git commit -m "iftd: one card's shell, outliving the app

It binds the socket before it forks, so the app's wait() returning is
proof the socket is connectable and there is no readiness race to poll
for. It parses nothing: bytes from the pty go to a ring and to whoever
is attached, unchanged, which is the property tmux could not give us."
```

---

### Task 3: The client side of the socket

**Files:**
- Create: `infiniterm-core/src/backend/daemon.rs`
- Modify: `infiniterm-core/src/backend/mod.rs` (`pub mod daemon;`)
- Test: in-file `#[cfg(test)]`, plus the roundtrip from Task 2 now exercised through this type

**Interfaces:**
- Consumes: `session_protocol`, `iftd` on `PATH` or beside the executable.
- Produces: `DaemonBackend::new(sessions_dir: PathBuf, buffer_mib: usize) -> (Self, Receiver<(PaneId, PaneEvent)>)`, and the same `_now` method set `LocalPtyBackend` has: `spawn_now`, `write_now`, `resize_now`, `ack_now`, `kill_now`, `kill_all`, `pids_source`. Plus `session_id(PaneId) -> Option<String>`, `adopt(&str) -> Option<PaneId>`, `live_sessions(dir) -> Vec<String>` (associated fn), `kill_orphans(&[String])`, `detach()`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_session_id_is_a_filename_and_nothing_else() {
    // It names a socket in a directory we own; a slash or a dot-dot in it
    // would name a file outside that directory.
    let id = new_session_id();
    assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    assert_ne!(new_session_id(), id, "two cards never collide");
}

#[test]
fn stale_sockets_are_not_live_sessions() {
    let dir = tempdir();
    std::fs::write(dir.path().join("dead.sock"), b"").unwrap();
    assert!(DaemonBackend::live_sessions(dir.path()).is_empty());
    assert!(!dir.path().join("dead.sock").exists(), "and it is swept");
}
```

An integration test in `infiniterm-core/tests/daemon.rs` covering the real path: spawn through `DaemonBackend`, assert `PaneEvent::Output` arrives, drop the backend, build a second one on the same directory, `adopt` the id, and assert a `PaneEvent::Replay` arrives carrying the marker. Gate it on `iftd` having been built (`env!("CARGO_BIN_EXE_iftd")` is not available across crates, so resolve `target/<profile>/iftd` from `std::env::current_exe`).

- [ ] **Step 2: Run, watch fail**

Run: `cargo test -p infiniterm-core daemon`
Expected: FAIL, no such module.

- [ ] **Step 3: Write it**

Model it on `local_pty.rs` structure for structure: a `Pane` struct, a `HashMap<PaneId, Pane>` behind an `Arc<Mutex<..>>`, the same `Credit` mechanism for `ack_now`, one reader thread per pane funnelling into one `Sender<(PaneId, PaneEvent)>`.

The differences from `local_pty`:

- `spawn_now` builds a session id, runs `iftd` with `Command::status()`, connects, reads the `Hello` frame (with a 5 second deadline) to learn the pid, and only then registers the pane.
- The reader thread runs a `FrameReader` instead of raw reads. `Frame::Data` becomes `PaneEvent::Output`, `Frame::Replay` becomes `PaneEvent::Replay`, `Frame::Exited(c)` becomes `PaneEvent::Exited { code: c }`. `ProtoError` ends the thread.
- **After `ReplayEnd`, resize by one column and back**, with the reason in a comment: the replayed ring may have been cut mid-state, and a `SIGWINCH` makes the program repaint its current screen over whatever the replay left. This is the safety net the spec's risk 1 depends on.
- `kill_now` sends `Frame::Kill`; `detach()` drops the sockets and leaves every daemon running; `kill_all` sends `Kill` to all.
- `live_sessions` reads the directory, tries `UnixStream::connect` on each `.sock`, and unlinks the ones that refuse (`ECONNREFUSED` means a daemon died without cleaning up).

Find `iftd` beside the current executable first (that is where the bundle puts it), then on `PATH`. A missing `iftd` returns an error from `spawn_now`, which the card shows; it must not panic.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p infiniterm-core daemon`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add infiniterm-core/src/backend/daemon.rs infiniterm-core/src/backend/mod.rs infiniterm-core/tests/daemon.rs
git commit -m "Talk to iftd with the same shape local_pty already had

One reader thread per pane onto the one tagged stream, and the same credit
ledger for backpressure, so nothing above this file learns that a socket
replaced a pty. After a replay the pane is resized a column and back: the
SIGWINCH makes the program repaint over a ring that may have been cut
mid-state, which is what keeps an imperfect replay from lasting a frame."
```

---

### Task 4: Three backends, one enum

This task includes the mechanical rename through the ui, so the tree compiles and `make check` passes at the end of it. The behavioural ui change is Task 5.

**Files:**
- Modify: `infiniterm-core/src/backend/mod.rs`, `infiniterm-core/src/config.rs`, `infiniterm-core/src/settings_doc.rs`, `infiniterm-core/src/saved_layout.rs`, `infiniterm-core/src/paths.rs`, `infiniterm-ui/src/terminals.rs`, `infiniterm-ui/src/runtime.rs`

**Interfaces:**
- Consumes: `DaemonBackend` from Task 3.
- Produces: `Panes::Daemon`, `Panes::start(backend: TerminalBackend, buffer_mib: usize)`, `Panes::we_are_the_terminal() -> bool`, `paths::sessions_dir() -> PathBuf`, `TerminalBackend::Daemon`, `Config::terminal.session_buffer: f64`, `SavedCard::session` replacing `tmux_window`.

- [ ] **Step 1: Write the failing tests**

```rust
// tmux is the ONLY backend that answers terminal queries itself. Getting
// this backwards sends our answers into a program that already got tmux's
// and they arrive as keystrokes: that was tmux bug 9, and it is what made
// Claude Code "a mess" for an evening.
#[test]
fn only_tmux_answers_for_itself() {
    assert!(Panes::start(TerminalBackend::Pty, 4).0.we_are_the_terminal());
    assert!(Panes::start(TerminalBackend::Daemon, 4).0.we_are_the_terminal());
    // tmux's arm is asserted in the tmux tests, which have a tmux to talk to.
}

#[test]
fn a_session_survives_the_save_file() {
    let card = saved_card_with_session("abc-123");
    let text = render_layout(&[card.clone()]);
    assert!(text.contains("\"session\": \"abc-123\""));
    assert_eq!(parse_layout(&text).unwrap().cards[0].session.as_deref(), Some("abc-123"));
}

// Canvases written by the tmux evening still load.
#[test]
fn the_old_tmux_window_key_is_still_read() {
    let text = r#"{"cards":[{"id":"a","kind":"terminal","tmuxWindow":"@3", ... }]}"#;
    assert_eq!(parse_layout(text).unwrap().cards[0].session.as_deref(), Some("@3"));
}

#[test]
fn the_backend_setting_names_three_things() {
    for (text, want) in [("pty", TerminalBackend::Pty), ("tmux", TerminalBackend::Tmux), ("daemon", TerminalBackend::Daemon)] {
        let c = merge_config(&serde_json::json!({"terminal": {"backend": text}}));
        assert_eq!(c.terminal.backend, want);
    }
}
```

- [ ] **Step 2: Run, watch fail**

Run: `cargo test -p infiniterm-core`
Expected: FAIL on the new names.

- [ ] **Step 3: Make the changes**

1. `paths.rs`: `pub fn sessions_dir() -> PathBuf { data_dir(None).join("sessions") }`, created on demand. Because it is under the data dir, `INFINITERM_DATA_DIR` isolates a scratch instance for free; say so in a comment, and that this is what tmux needed a `session_name()` for.
2. `config.rs`: `TerminalBackend::Daemon`; add `("daemon", TerminalBackend::Daemon)` to the `one(..)` table. Add `session_buffer: f64` to `Terminal`, default `4.` (MiB). **Leave the default backend at `Pty` in this task**; Task 7 flips it, after there is something to flip to.
3. `settings_doc.rs`: rewrite the `terminal.backend` entry for three values, and add `terminal.sessionBuffer`. A setting without an entry fails the doc test, which is the point of that test.
4. `saved_layout.rs`: rename the field to `session`, render it as `"session"`, and parse `session` with `tmuxWindow` as a fallback. One comment saying why both: the key is the backend's opaque handle for this card's shell, and the tmux spelling is one evening of save files.
5. `backend/mod.rs`: `Panes::Daemon(DaemonBackend)`, every match arm, `start` taking `TerminalBackend`, and:

```rust
/// Whether OUR emulator is the thing programs are talking to. False only
/// under tmux, which is a terminal in its own right and answers colour and
/// device queries before we can; our second answer then reaches the program
/// as keystrokes. See `TerminalBody::replies` and tmux bug 9.
pub fn we_are_the_terminal(&self) -> bool {
    !matches!(self, Panes::Tmux(_))
}
```

`window_id` becomes `session_id`, `live_windows` becomes `live_sessions`, `kill_orphans` keeps its name. Under `Panes::Daemon`, `pids_source` answers real pids from the `Hello` frames, unlike tmux.

- [ ] **Step 4: Rename through the ui**

`card.tmux_window` → `card.session` in `terminals.rs` (the adopt block around line 131 and the save-back around line 184) and `runtime.rs` (the orphan sweep around line 517). The comments there talk about tmux windows; rewrite them for sessions, keeping the reason: a card that was here before takes its shell back rather than starting a second one beside it. `window_id` → `session_id` and `live_windows` → `live_sessions` at those call sites, and `AppView::live_windows` becomes `live_sessions`.

- [ ] **Step 5: Run the tests**

Run: `make check`
Expected: PASS. Nothing has changed behaviour yet: the daemon is selectable but not selected, and every rename is mechanical.

- [ ] **Step 6: Commit**

```bash
git add infiniterm-core/src infiniterm-ui/src
git commit -m "A card's shell is a session id, whatever holds it

The save file's handle was named for tmux windows; it was always just the
backend's opaque handle, and now three backends answer it. Old canvases
keep loading: tmuxWindow is still read, it is only no longer written."
```

---

### Task 5: Turn it on

**Files:**
- Modify: `infiniterm-ui/src/terminals.rs`, `infiniterm-ui/src/runtime.rs`

**Interfaces:**
- Consumes: everything from Task 4.

- [ ] **Step 1: Fix the query-reply gate**

In `terminals.rs`, `let answer = !self.backend.pty.is_tmux();` becomes `let answer = self.backend.pty.we_are_the_terminal();`. Same meaning today, correct meaning under the daemon. This is the single most dangerous line in the change: inverted, it reproduces the corruption this whole plan exists to remove.

- [ ] **Step 2: Start the right backend**

In `runtime.rs` startup, pass `app.model.config.terminal.backend` and `session_buffer` into `Panes::start`. Keep the existing fallback notice path: a `daemon` backend whose `iftd` is missing falls back to local shells and says so, exactly as a missing tmux does.

- [ ] **Step 3: Verify on screen**

Run with the daemon backend on a scratch data dir:

```bash
make run DATA=/tmp/infiniterm-daemon
```

with `/tmp/infiniterm-daemon-config/settings.json` setting `"terminal": {"backend": "daemon"}` (pass `INFINITERM_CONFIG_DIR`). Open a card, run `echo hi`, quit, relaunch, and confirm the card comes back with `hi` still on it and a live prompt.

- [ ] **Step 4: Commit**

```bash
git add infiniterm-ui/src
git commit -m "Cards can live in our own session daemon

The query-reply gate moves from is_tmux() to we_are_the_terminal(): under
our own daemon we ARE the terminal and our answers must go, where under
tmux they must not, because tmux answered first and a second answer reaches
the program as keystrokes."
```

---

### Task 6: `ift sessions` and `ift attach`

**Files:**
- Modify: `infiniterm-cli/src/main.rs`, `infiniterm-cli/Cargo.toml`
- Create: `infiniterm-cli/src/attach.rs`

**Interfaces:**
- Consumes: `session_protocol`, `paths::sessions_dir`.
- Produces: `ift sessions`, `ift attach <id>`.

- [ ] **Step 1: Add the dependency, with the reason**

`infiniterm-cli/Cargo.toml` gains `libc = "0.2"` and `infiniterm-core = { path = "../infiniterm-core" }`. The existing comment in that file says dependencies are kept out deliberately; amend it rather than delete it: raw mode is `tcgetattr`/`cfmakeraw`/`tcsetattr` and `SIGWINCH` is a signal handler, and shelling out to `stty` to avoid a dependency is the kind of cleverness that breaks in a pipe.

- [ ] **Step 2: Write the failing test**

```rust
// It must work with no app running: that is the entire reason it exists.
#[test]
fn sessions_are_listed_from_the_meta_files_alone() {
    let dir = tempdir();
    std::fs::write(dir.path().join("x.meta"), r#"{"pid":42,"cwd":"/tmp","cmd":"zsh","started":"2026-09-17T10:00:00Z"}"#).unwrap();
    std::fs::write(dir.path().join("x.sock"), b"").unwrap();
    let rows = list_sessions(dir.path());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].pid, 42);
    assert_eq!(rows[0].cwd, "/tmp");
}
```

- [ ] **Step 3: Run, watch fail**, then write `attach.rs`:

- `list_sessions(dir) -> Vec<SessionRow>` reads every `*.meta` and pairs it with its socket. `ift sessions` prints id, pid, cwd, command, started, tab separated, because that is what the rest of `ift` prints and it pipes.
- `attach(id)`: connect, `tcgetattr` the tty and keep the original, `cfmakeraw` a copy and set it, then two threads: stdin → `Frame::Data`, and frames → stdout (`Data` and `Replay` both, `Exited` ends it). Send `Frame::Resize` at the start from `TIOCGWINSZ` and again on `SIGWINCH`. Restore the termios on every exit path, including the signal one, or the user's shell is left in raw mode.
- `Ctrl-\` (0x1c) in the stdin stream detaches: stop forwarding, restore, exit 0. Document it in `--help`.

- [ ] **Step 4: Verify by hand**

Start a card under the daemon backend, then from any terminal:

```bash
ift sessions
ift attach <id>          # the card's shell, in your terminal
# type something, Ctrl-\ to detach, and see it in the card
```

- [ ] **Step 5: Commit**

```bash
git add infiniterm-cli
git commit -m "Reach a card's shell from outside the app

This is the escape hatch, and it was tmux's one real advantage over a
daemon of our own: when the GUI is the thing that broke, the work is still
running and `ift attach` gets you to it with no app in the way."
```

---

### Task 7: Ship it

**Files:**
- Modify: `tools/bundle.sh`, `infiniterm-core/src/config.rs`, `infiniterm-core/src/settings_doc.rs`, `README.md`, `CLAUDE.md`, `docs/tmux-handover.md`
- Create: `tools/drive/daemon.sh`

- [ ] **Step 1: Bundle `iftd`**

`tools/bundle.sh` builds and copies three sidecars now:

```bash
cargo build $( [ "$profile" = release ] && echo --release ) -p infiniterm-cli -p infiniterm-hook -p infiniterm-session
cp "target/$profile/ift" "target/$profile/infiniterm-hook" "target/$profile/iftd" "$app/Contents/MacOS/"
```

- [ ] **Step 2: Write the falsification scenario**

`tools/drive/daemon.sh`, on the pattern of `tmux.sh`, against a scratch data dir:

1. Open a card, run a program that redraws inline. Screenshot.
2. Quit the app.
3. Relaunch. Screenshot the adopted card.
4. Compare: the card holds its history and the live prompt works.

Then the real one, which the spec calls the falsification test for the whole design: run Claude Code in a card, let it draw, quit, relaunch, screenshot. If Claude comes back with two lines in one row here, the ring replay is not the cure and the fault is in our widths or our parser. **Run this before Step 3. If it fails, stop and report rather than making this the default.**

- [ ] **Step 3: Flip the default**

`config.rs`: `backend: TerminalBackend::Daemon`. `settings_doc.rs`: rewrite the entry so `daemon` is described as the default and `pty` as the one that dies with the window.

- [ ] **Step 4: Update the prose**

- `README.md`: persistent sessions are a feature now, in one plain sentence.
- `CLAUDE.md`: replace the tmux bullet under "Rules that shape the code" with the daemon's rule (one daemon per card, no VT parsing in it, the socket is the backpressure, `we_are_the_terminal`), keeping a line that tmux remains selectable and why it is not the default.
- `docs/tmux-handover.md`: a note at the top that the daemon superseded it, with the date and a pointer to the spec. Do not delete the bug list: it is why the daemon is shaped this way.

- [ ] **Step 5: `make check`, then install and live on it**

```bash
make check && make release
```

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "Persistent shells by default, without a second emulator

Our own daemon replaces tmux as the default. It parses nothing, so a
reattach replays the original bytes into our own parser and the two cannot
disagree; tmux stays selectable for anyone who wants its session sharing."
```

---

## What could still be wrong after all seven

1. **Claude could corrupt anyway.** Task 7 Step 2 finds this out, and it is gated before the default flips.
2. **4 MiB may be too shallow for a Claude card**, because Ink repaints are large and repetitive. The setting exists; the deeper fix (grid snapshot compaction) is named in the spec and deliberately not built here.
3. **A daemon per card is a process per card.** ~150 MB worst case at 25 cards with every ring full.
4. **A reboot still ends everything.** Unchanged, known, out of scope.
