//! Integration tests for `iftd`: run the real binary against a temp
//! directory and talk to it exactly as `infiniterm-core::backend::daemon`
//! will (Task 3), one frame at a time, over the real socket.
//!
//! Never binds `/tmp/infiniterm.sock` and never touches the real data dir:
//! every socket here lives under a throwaway temp directory this file
//! creates and removes itself, because a real, installed instance of the
//! app may be running on this machine at the same time.
//!
//! Related: `infiniterm_core::backend::session_protocol` (the wire, Task 1),
//! `src/main.rs` (what is under test),
//! docs/superpowers/specs/2026-09-17-session-daemon-design.md ("Testing").

use infiniterm_core::backend::session_protocol::{Frame, FrameReader};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Budget for any single "wait for a frame" loop below. Generous because
/// these tests run a real shell under a real pty, not a mock.
const DEADLINE: Duration = Duration::from_secs(5);

/// Socket read granularity while polling for frames. Short enough that the
/// deadline above is actually honoured to within a fraction of a second.
const POLL: Duration = Duration::from_millis(200);

/// A directory under the OS temp dir that removes itself on drop. Not
/// `tempfile`: this crate has no dependency on it and the few lines here
/// are the whole job (see the reuse ladder in CLAUDE.md).
///
/// Kept SHORT on purpose: a unix domain socket path is limited to
/// `sizeof(sockaddr_un.sun_path)`, 104 bytes on macOS including the NUL.
/// macOS's own `$TMPDIR` is already ~50 bytes of that budget, and an
/// earlier version of this helper (pid + counter + a nanosecond timestamp)
/// filled the rest exactly — `bind()` failed with "path too long" before
/// `iftd` ever forked, which looked identical to the daemon refusing to
/// start. pid + a small counter is unique enough for one test binary run.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        // A per-process counter: cargo runs this file's tests on several
        // threads at once, so the pid alone is not unique enough.
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("iftd-t-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a fresh temp dir");
        TempDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tempdir() -> TempDir {
    TempDir::new()
}

/// Runs iftd and returns a connected stream. The socket is under a temp
/// dir, so this can never reach the real one at `<data>/sessions`.
///
/// `iftd` binds the listener before it forks and the first process exits 0
/// only once that has happened (see `src/main.rs`), so `status()` returning
/// success is itself the readiness signal: there is nothing to poll for.
fn start(dir: &Path, name: &str) -> UnixStream {
    let sock = dir.join(format!("{name}.sock"));
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_iftd"))
        .args(["--socket", sock.to_str().unwrap(), "--cwd", "/tmp"])
        .status()
        .expect("iftd runs");
    assert!(status.success(), "iftd bound the socket before it forked");
    UnixStream::connect(&sock).expect("connectable the instant iftd exits")
}

/// A connected socket plus the buffering needed to pull frames out of it
/// one at a time. A bare `UnixStream` is not enough for these tests: two
/// helper calls in a row (say, "wait for Hello" then "collect the replay")
/// must share one buffer, or bytes that arrived in the same `read()` as the
/// frame the first call was waiting for would be silently dropped when that
/// call's reader went out of scope.
struct Conn {
    stream: UnixStream,
    reader: FrameReader,
}

impl Conn {
    fn new(stream: UnixStream) -> Self {
        stream
            .set_read_timeout(Some(POLL))
            .expect("a socket takes a read timeout");
        Conn {
            stream,
            reader: FrameReader::default(),
        }
    }

    fn send(&mut self, f: &Frame) {
        self.stream
            .write_all(&f.encode())
            .expect("iftd is still reading");
    }

    /// The next frame, waiting up to `DEADLINE` for it to arrive.
    fn recv(&mut self) -> Frame {
        let deadline = Instant::now() + DEADLINE;
        let mut buf = [0u8; 8192];
        loop {
            if let Some(f) = self.reader.next().expect("a well-formed frame") {
                return f;
            }
            assert!(
                Instant::now() < deadline,
                "no frame arrived within the deadline"
            );
            match self.stream.read(&mut buf) {
                Ok(0) => panic!("iftd closed the connection"),
                Ok(n) => self.reader.feed(&buf[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => panic!("read error while waiting for a frame: {e}"),
            }
        }
    }

    /// The next frame matching `want`. None of these tests expect an
    /// unrelated frame to show up first, so anything else is a bug in the
    /// daemon's ordering, not a case to swallow silently.
    fn recv_matching(&mut self, mut want: impl FnMut(&Frame) -> bool) -> Frame {
        loop {
            let f = self.recv();
            if want(&f) {
                return f;
            }
        }
    }

    /// Concatenates `Data` frames (echo, prompt, program output — whatever
    /// the shell wrote) until `needle` shows up in them.
    fn collect_output(&mut self, needle: &str) -> String {
        let mut acc = String::new();
        loop {
            if let Frame::Data(bytes) = self.recv() {
                acc.push_str(&String::from_utf8_lossy(&bytes));
                if acc.contains(needle) {
                    return acc;
                }
            }
        }
    }

    /// Everything a `Replay` carries, concatenated, up to `ReplayEnd`.
    fn collect_replay(&mut self) -> String {
        let mut acc = Vec::new();
        loop {
            match self.recv() {
                Frame::Replay(bytes) => acc.extend(bytes),
                Frame::ReplayEnd => return String::from_utf8_lossy(&acc).into_owned(),
                other => panic!("expected Replay or ReplayEnd, got {other:?}"),
            }
        }
    }
}

/// Sends `Kill` and waits for the socket to disappear. Every test below
/// calls this before returning, even the ones not testing `Kill` itself:
/// without it, a shell (`d`'s `yes`, spinning a core at 100%) outlives the
/// test process, and repeated `cargo test` runs pile up daemons with
/// nothing left to stop them.
fn stop(dir: &Path, name: &str, c: &mut Conn) {
    c.send(&Frame::Kill);
    assert!(
        wait_until(|| !dir.join(format!("{name}.sock")).exists()),
        "{name}'s daemon did not clean up its socket after Kill"
    );
}

/// Polls `pred` until it is true or `DEADLINE` passes.
fn wait_until(mut pred: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if pred() {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `kill -0`: true if a process with this pid exists, whoever it belongs
/// to. Shelling out rather than adding `libc` as a dev-dependency just for
/// a signal-0 check, which `kill` already gives us as a subprocess.
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Resident set size of `pid`, in KiB, or 0 once the process is gone.
fn rss_kb(pid: u32) -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

/// Finds the daemonised `iftd` process for a socket, by grepping the
/// process table for the `--socket <path>` it was launched with. `fork()`
/// never re-execs, so the grandchild that survives the double fork still
/// carries the same argv as the process `start()` launched. `-ww` asks
/// macOS's `ps` for the untruncated command line; without it a long temp
/// path gets cut and never matches.
fn daemon_pid(dir: &Path, name: &str) -> u32 {
    let needle = format!("--socket {}", dir.join(format!("{name}.sock")).display());
    let out = std::process::Command::new("ps")
        .args(["-axww", "-o", "pid=,command="])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| {
            line.contains(&needle).then(|| {
                line.split_whitespace()
                    .next()
                    .expect("a pid column")
                    .parse()
                    .expect("a numeric pid")
            })
        })
        .unwrap_or_else(|| panic!("no iftd found for {name} (looked for {needle:?})"))
}

#[test]
fn a_shell_starts_and_echoes() {
    let dir = tempdir();
    let mut c = Conn::new(start(dir.path(), "a"));

    let hello = c.recv_matching(|f| matches!(f, Frame::Hello { .. }));
    assert!(matches!(hello, Frame::Hello { pid, .. } if pid > 0));

    c.send(&Frame::Resize { cols: 80, rows: 24 });
    c.send(&Frame::Data(b"echo ready-1\n".to_vec()));
    let seen = c.collect_output("ready-1");
    assert!(seen.contains("ready-1"), "got {seen:?}");

    stop(dir.path(), "a", &mut c);
}

// The whole point: the shell outlives the client.
#[test]
fn a_reattach_replays_what_was_missed() {
    let dir = tempdir();
    let mut c = Conn::new(start(dir.path(), "b"));
    c.recv_matching(|f| matches!(f, Frame::Hello { .. }));
    c.send(&Frame::Data(b"echo marker-42\n".to_vec()));
    c.collect_output("marker-42");
    drop(c); // detach: closing the socket, not killing the shell

    let mut again = Conn::new(UnixStream::connect(dir.path().join("b.sock")).expect("still bound"));
    again.recv_matching(|f| matches!(f, Frame::Hello { .. }));
    let replay = again.collect_replay();
    assert!(
        replay.contains("marker-42"),
        "the ring came back: {replay:?}"
    );

    stop(dir.path(), "b", &mut again);
}

#[test]
fn kill_ends_the_child_and_removes_the_socket() {
    let dir = tempdir();
    let mut c = Conn::new(start(dir.path(), "c"));
    let Frame::Hello { pid, .. } = c.recv_matching(|f| matches!(f, Frame::Hello { .. })) else {
        unreachable!()
    };
    c.send(&Frame::Kill);
    // The daemon unlinks its socket on its way out.
    assert!(
        wait_until(|| !dir.path().join("c.sock").exists()),
        "socket gone"
    );
    assert!(wait_until(|| !process_alive(pid)), "child gone");
}

// A client that stops reading must stall the child, not grow the daemon:
// the socket write blocking is the entire backpressure mechanism (see
// session_protocol.rs's header), and this is the test that it actually
// holds rather than the daemon buffering behind the client's back.
#[test]
fn a_client_that_never_reads_stalls_the_child() {
    let dir = tempdir();
    let mut c = Conn::new(start(dir.path(), "d"));
    let Frame::Hello { pid: shell_pid, .. } = c.recv_matching(|f| matches!(f, Frame::Hello { .. }))
    else {
        unreachable!()
    };
    // A fast, endless producer. After this, `c` reads nothing more: it
    // plays the detached-but-not-closed client whose socket buffer fills.
    c.send(&Frame::Data(b"yes\n".to_vec()));

    std::thread::sleep(Duration::from_millis(500));
    let daemon = daemon_pid(dir.path(), "d");
    let rss = rss_kb(daemon);
    std::thread::sleep(Duration::from_secs(2));
    // 8 MiB of slack: the kernel socket buffer and the ring (4 MiB default)
    // both saturate quickly and then hold steady; unbounded growth would
    // blow well past this within two seconds of `yes`.
    const SLACK_KB: u64 = 8192;
    assert!(
        rss_kb(daemon) < rss + SLACK_KB,
        "daemon grew from {rss} KiB, bounded write is not blocking the reader"
    );

    // Cleanup, not part of the assertion above, but necessary: this test's
    // whole premise is that nothing is draining the pty (the pty-reader
    // thread is deliberately stuck mid-write to `c`), so a graceful
    // `Frame::Kill` is not guaranteed to unwind promptly here — the shell,
    // trying to write anything of its own (a job-control notice, a prompt
    // refresh) to a pty nobody is reading, can itself block right behind
    // `yes`. That graceful path (Kill, unlink, reap) is already covered
    // without this stall by `kill_ends_the_child_and_removes_the_socket`.
    // Ending the daemon PROCESS is unconditional instead: the kernel tears
    // down every fd it holds — the pty master, the stalled socket — in one
    // step, which a signal to just the shell cannot promise while any of
    // that is mid-syscall.
    let _ = std::process::Command::new("kill")
        .args(["-9", &daemon.to_string()])
        .status();
    let _ = std::process::Command::new("kill")
        .args(["-9", &shell_pid.to_string()])
        .status();
    let _ = std::fs::remove_file(dir.path().join("d.sock"));
    let _ = std::fs::remove_file(dir.path().join("d.meta"));
}
