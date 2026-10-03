//! A session on another machine, reached over ssh (#118, the remote instances).
//!
//! `connect` starts `ssh -T host ift proxy session <id> [--spawn ...]` and
//! returns a unix socket whose far end is spliced to that ssh process's stdin
//! and stdout. `DaemonBackend` registers the near end exactly as it registers a
//! local `iftd` socket, so Hello, Replay, Data, Resize, Kill and the credit
//! ledger all work unchanged: nothing in the pane code learns about ssh.
//!
//! Called by `daemon.rs` (`spawn_remote`, `adopt_remote`). The server half is
//! `infiniterm-cli/src/proxy.rs`. Related: `session_protocol.rs`.
//!
//! Non-obvious constraints:
//! - ssh runs with `BatchMode`: a password prompt has no terminal to appear
//!   on, so key authentication is required and a failure is an error, not a
//!   hang.
//! - The remote command goes through the remote user's shell, so every value
//!   in it is quoted (`drop::shell_quote`). A card directory with a space or a
//!   quote must reach `iftd` as text.
//! - ssh's stderr is kept (its last lines) so a refused key or an unreachable
//!   host can be shown, since stdout is the protocol and carries no prose.
//! - Closing the near socket ends the relay and so ssh; the daemon on the
//!   server keeps the shell running. That is a detach.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use crate::drop::shell_quote;

/// How much of ssh's stderr to keep: enough for a reason, not a log.
const STDERR_KEEP: usize = 2048;

/// Where a remote session lives and how to reach it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteHost {
    /// `user@host` or an alias from `~/.ssh/config`.
    pub target: String,
    /// The program that speaks ssh. `ssh`; a test puts a stand-in here.
    pub ssh: String,
    /// Extra arguments before the target, for example `-p 2222`.
    pub ssh_args: Vec<String>,
    /// `ift` as the server's shell finds it: a name on its `PATH`, or a path.
    pub ift: String,
}

impl RemoteHost {
    pub fn new(target: &str) -> Self {
        RemoteHost {
            target: target.to_string(),
            ssh: "ssh".into(),
            ssh_args: vec![],
            ift: "ift".into(),
        }
    }
}

/// What `--spawn` asks the server's `iftd` for.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SpawnSpec {
    pub cwd: Option<String>,
    pub cmd: Option<String>,
    pub env: Vec<(String, String)>,
    pub buffer_mib: Option<usize>,
}

/// The one string the server's shell runs: `ift proxy session <id> ...`, every
/// value quoted.
pub fn remote_command(ift: &str, id: &str, spawn: Option<&SpawnSpec>) -> String {
    let mut parts = vec![
        shell_quote(ift),
        "proxy".into(),
        "session".into(),
        shell_quote(id),
    ];
    if let Some(s) = spawn {
        parts.push("--spawn".into());
        if let Some(cwd) = &s.cwd {
            parts.push("--cwd".into());
            parts.push(shell_quote(cwd));
        }
        if let Some(cmd) = &s.cmd {
            parts.push("--cmd".into());
            parts.push(shell_quote(cmd));
        }
        for (k, v) in &s.env {
            parts.push("--env".into());
            parts.push(shell_quote(&format!("{k}={v}")));
        }
        if let Some(b) = s.buffer_mib {
            parts.push("--buffer".into());
            parts.push(b.to_string());
        }
    }
    parts.join(" ")
}

/// The argument list for the ssh child, after the program name.
pub fn ssh_args(host: &RemoteHost, id: &str, spawn: Option<&SpawnSpec>) -> Vec<String> {
    let mut args: Vec<String> = [
        "-T",
        "-o",
        "BatchMode=yes",
        // A dead network should end the connection in about 45 s, not never.
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=3",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.extend(host.ssh_args.iter().cloned());
    args.push(host.target.clone());
    args.push(remote_command(&host.ift, id, spawn));
    args
}

/// The last lines ssh said, shared with the thread that reads them.
#[derive(Clone, Default)]
pub struct SshStderr(Arc<Mutex<String>>);

impl SshStderr {
    pub fn text(&self) -> String {
        self.0.lock().unwrap().trim().to_string()
    }
}

/// Starts ssh and returns the near end of a socketpair spliced to it, plus
/// what ssh writes to stderr. The caller reads `Hello` from the socket.
pub fn connect(
    host: &RemoteHost,
    id: &str,
    spawn: Option<&SpawnSpec>,
) -> std::io::Result<(UnixStream, SshStderr)> {
    let mut child = Command::new(&host.ssh)
        .args(ssh_args(host, id, spawn))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut ssh_in = child.stdin.take().expect("piped stdin");
    let mut ssh_out = child.stdout.take().expect("piped stdout");
    let mut ssh_err = child.stderr.take().expect("piped stderr");
    let (near, far) = UnixStream::pair()?;
    let mut far_write = far.try_clone()?;
    let mut far_read = far;

    let stderr = SshStderr::default();
    let keep = stderr.0.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 512];
        while let Ok(n) = ssh_err.read(&mut buf) {
            if n == 0 {
                break;
            }
            let mut s = keep.lock().unwrap();
            s.push_str(&String::from_utf8_lossy(&buf[..n]));
            if s.len() > STDERR_KEEP {
                let cut = s.len() - STDERR_KEEP;
                let at = (cut..s.len())
                    .find(|i| s.is_char_boundary(*i))
                    .unwrap_or(cut);
                s.drain(..at);
            }
        }
    });
    // ssh to the app: the protocol, until ssh ends.
    let down = std::thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        while let Ok(n) = ssh_out.read(&mut buf) {
            if n == 0 || far_write.write_all(&buf[..n]).is_err() {
                break;
            }
        }
        let _ = far_write.shutdown(std::net::Shutdown::Both);
    });
    // The app to ssh, until the app closes its end; then ssh's stdin closes,
    // which ends the proxy, which leaves the daemon's shell running.
    std::thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        while let Ok(n) = far_read.read(&mut buf) {
            if n == 0
                || ssh_in
                    .write_all(&buf[..n])
                    .and_then(|_| ssh_in.flush())
                    .is_err()
            {
                break;
            }
        }
        drop(ssh_in);
        let _ = down.join();
        let _ = child.wait();
    });
    Ok((near, stderr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_remote_command_quotes_every_value() {
        let spawn = SpawnSpec {
            cwd: Some("/srv/my app".into()),
            cmd: Some("echo 'hi'; ls".into()),
            env: vec![("A".into(), "x y".into())],
            buffer_mib: Some(8),
        };
        let cmd = remote_command("ift", "abc123", Some(&spawn));
        assert_eq!(
            cmd,
            "ift proxy session abc123 --spawn --cwd '/srv/my app' --cmd 'echo '\\''hi'\\''; ls' --env 'A=x y' --buffer 8"
        );
        assert_eq!(
            remote_command("/opt/ift", "abc", None),
            "/opt/ift proxy session abc"
        );
    }

    #[test]
    fn a_hostile_value_stays_one_word() {
        let spawn = SpawnSpec {
            cwd: Some("; rm -rf ~".into()),
            ..Default::default()
        };
        let cmd = remote_command("ift", "a", Some(&spawn));
        assert!(cmd.ends_with("--cwd '; rm -rf ~'"), "{cmd}");
    }

    #[test]
    fn ssh_gets_batch_mode_keepalive_the_extras_the_target_and_the_command() {
        let mut host = RemoteHost::new("root@100.1.1.1");
        host.ssh_args = vec!["-p".into(), "2222".into()];
        let args = ssh_args(&host, "id1", None);
        assert_eq!(&args[..2], ["-T", "-o"]);
        assert!(args.contains(&"BatchMode=yes".to_string()));
        assert!(args.contains(&"ServerAliveInterval=15".to_string()));
        let n = args.len();
        assert_eq!(args[n - 4], "-p");
        assert_eq!(args[n - 3], "2222");
        assert_eq!(args[n - 2], "root@100.1.1.1");
        assert_eq!(args[n - 1], "ift proxy session id1");
    }
}
