//! `ift proxy session --spawn` against the real `iftd`: the daemon's protocol
//! comes out of the child's stdout and keystrokes go in through its stdin,
//! which is all `ssh -T host ift proxy ...` needs to carry (#118).
//!
//! `iftd` is found the way infiniterm-core's daemon tests find it: beside the
//! test binary's profile directory (`cargo build -p infiniterm-session` first).
//! The child gets its own data dir, so nothing here touches a real session.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use infiniterm_core::backend::session_protocol::{Frame, FrameReader};

const DEADLINE: Duration = Duration::from_secs(10);

fn profile_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().parent().unwrap().to_path_buf()
}

/// A scratch data dir. On drop it kills any `iftd` serving a socket under it
/// (found by the `--socket <path>` in its argv, the way infiniterm-core's
/// daemon tests do): a daemon outlives its client by design, and a graceful
/// Kill does not always end it (a shell that ignores SIGHUP, as under some
/// test runners), so a test must not leave one running on the machine.
struct Data(PathBuf);
impl Drop for Data {
    fn drop(&mut self) {
        let needle = format!("--socket {}", self.0.join("s").display());
        if let Ok(out) = Command::new("ps")
            .args(["-axww", "-o", "pid=,command="])
            .output()
        {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if line.contains(&needle) {
                    if let Some(pid) = line.split_whitespace().next() {
                        let _ = Command::new("kill").args(["-9", pid]).status();
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_spawned_session_speaks_the_daemon_protocol_over_stdio() {
    let iftd = profile_dir().join("iftd");
    assert!(
        iftd.is_file(),
        "run `cargo build -p infiniterm-session` first"
    );
    // A short path: a unix socket path has a small limit.
    let data = Data(std::env::temp_dir().join(format!("ift-px-{}", std::process::id())));
    std::fs::create_dir_all(&data.0).unwrap();
    let path = format!(
        "{}:{}",
        profile_dir().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_ift"))
        .args(["proxy", "session", "t1", "--spawn", "--cwd", "/tmp"])
        .args(["--cmd", "printf READY; exec sleep 30"])
        .env("INFINITERM_DATA_DIR", &data.0)
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    // Read frames until the shell's output says READY.
    let mut reader = FrameReader::default();
    let mut buf = [0u8; 4096];
    let started = Instant::now();
    let (mut hello, mut ready) = (false, false);
    let mut seen = Vec::new();
    while !ready {
        assert!(started.elapsed() < DEADLINE, "no READY; saw {:?}", String::from_utf8_lossy(&seen));
        let n = stdout.read(&mut buf).unwrap();
        assert!(n > 0, "the proxy closed before READY");
        reader.feed(&buf[..n]);
        while let Some(f) = reader.next().unwrap() {
            match f {
                Frame::Hello { .. } => hello = true,
                Frame::Data(d) | Frame::Replay(d) => {
                    seen.extend_from_slice(&d);
                    ready = String::from_utf8_lossy(&seen).contains("READY");
                }
                _ => {}
            }
        }
    }
    assert!(hello, "Hello comes first");

    // End the session through the proxy, as the app's close does, so no
    // daemon is left running on this machine.
    stdin.write_all(&Frame::Kill.encode()).unwrap();
    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success(), "{status:?}");
}

#[test]
fn a_session_that_does_not_exist_is_a_clear_failure() {
    let data = Data(std::env::temp_dir().join(format!("ift-px2-{}", std::process::id())));
    std::fs::create_dir_all(&data.0).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ift"))
        .args(["proxy", "session", "nope"])
        .env("INFINITERM_DATA_DIR", &data.0)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "stdout is the protocol, never prose");
    assert!(String::from_utf8_lossy(&out.stderr).contains("no session nope"));
}
