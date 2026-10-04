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

/// The remote host this process was told to run its cards on, from the
/// environment `ift connect` sets (#118): `INFINITERM_REMOTE` is the ssh
/// target; `INFINITERM_REMOTE_ARGS` (extra ssh arguments, space separated),
/// `INFINITERM_REMOTE_IFT` (`ift` on the server) and `INFINITERM_REMOTE_SSH`
/// (the program, for a test) are optional.
pub fn from_env() -> Option<RemoteHost> {
    let var = |k: &str| std::env::var(k).ok();
    from_vars(
        var("INFINITERM_REMOTE").as_deref(),
        var("INFINITERM_REMOTE_ARGS").as_deref(),
        var("INFINITERM_REMOTE_IFT").as_deref(),
        var("INFINITERM_REMOTE_SSH").as_deref(),
    )
}

/// `from_env` over explicit values, so the rules are tested without touching
/// the process environment. An empty target is no remote at all.
pub fn from_vars(
    target: Option<&str>,
    args: Option<&str>,
    ift: Option<&str>,
    ssh: Option<&str>,
) -> Option<RemoteHost> {
    let target = target.map(str::trim).filter(|t| !t.is_empty())?;
    let mut host = RemoteHost::new(target);
    if let Some(a) = args {
        host.ssh_args = a.split_whitespace().map(str::to_string).collect();
    }
    if let Some(i) = ift.map(str::trim).filter(|i| !i.is_empty()) {
        host.ift = i.to_string();
    }
    if let Some(s) = ssh.map(str::trim).filter(|s| !s.is_empty()) {
        host.ssh = s.to_string();
    }
    Some(host)
}

/// The session ids `ift sessions` printed on the server. Into a pipe it
/// prints every column, tab separated, the id first; blank lines are skipped.
pub fn parse_sessions(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.split('\t').next())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

/// Runs one command on the host over ssh and returns what it printed. The
/// command goes through the server's login shell, so quote what you put in it.
pub fn run_on(host: &RemoteHost, command: &str) -> std::io::Result<std::process::Output> {
    let mut args = ssh_args(host, "x", None);
    // The last argument is the remote command; replace it.
    args.pop();
    args.push(command.to_string());
    Command::new(&host.ssh)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
}

/// `run_on` with a file as the command's input: the way a package reaches
/// `tar` on the host, with no copy tool and no temporary file there.
pub fn run_on_stdin(
    host: &RemoteHost,
    command: &str,
    input: std::fs::File,
) -> std::io::Result<std::process::Output> {
    let mut args = ssh_args(host, "x", None);
    args.pop();
    args.push(command.to_string());
    Command::new(&host.ssh)
        .args(args)
        .stdin(Stdio::from(input))
        .stderr(Stdio::piped())
        .output()
}

/// Which sessions the server still runs: `ssh host ift sessions`. One short
/// ssh at launch tells which saved cards can be adopted and which are gone.
pub fn list_sessions(host: &RemoteHost) -> std::io::Result<Vec<String>> {
    let out = run_on(host, &format!("{} sessions", shell_quote(&host.ift)))?;
    if !out.status.success() {
        return Err(std::io::Error::other(format!(
            "{}: {}",
            host.target,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(parse_sessions(&String::from_utf8_lossy(&out.stdout)))
}

/// The script `check` runs on the server: `ift` is there, `iftd` is beside it
/// or on the PATH, and `ift sessions` works (so its data directory does). Each
/// failure says what is missing on stderr and uses its own exit status.
pub fn check_command(ift: &str) -> String {
    let script = format!(
        concat!(
            "ift={ift}; ",
            "p=$(command -v \"$ift\") || {{ echo 'ift is not installed on the server' >&2; exit 11; }}; ",
            "d=$(dirname \"$p\"); ",
            "[ -x \"$d/iftd\" ] || command -v iftd >/dev/null || ",
            "{{ echo 'iftd is not installed on the server' >&2; exit 12; }}; ",
            "\"$ift\" sessions >/dev/null"
        ),
        ift = shell_quote(ift)
    );
    format!("sh -c {}", shell_quote(&script))
}

/// Can this host run infiniterm's server half: ssh works without a prompt,
/// `ift` and `iftd` are installed, and the sessions folder answers. The error
/// is a sentence for a person.
pub fn check(host: &RemoteHost) -> Result<(), String> {
    let out = run_on(host, &check_command(&host.ift))
        .map_err(|e| format!("could not run {}: {e}", host.ssh))?;
    if out.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(match out.status.code() {
        Some(11) | Some(12) => format!("{}: {said}", host.target),
        Some(255) => format!(
            "{}: ssh could not log in without a prompt: {said}",
            host.target
        ),
        _ => format!("{}: {said}", host.target),
    })
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
    fn the_remote_comes_from_the_environment_values() {
        assert_eq!(from_vars(None, None, None, None), None);
        assert_eq!(from_vars(Some("  "), None, None, None), None);
        let h = from_vars(Some("root@100.1.1.1"), None, None, None).unwrap();
        assert_eq!((h.ssh.as_str(), h.ift.as_str()), ("ssh", "ift"));
        assert!(h.ssh_args.is_empty());
        let h = from_vars(
            Some("srv"),
            Some("-p 2222 -i /k"),
            Some("/opt/ift"),
            Some("/tmp/fake-ssh"),
        )
        .unwrap();
        assert_eq!(h.ssh_args, ["-p", "2222", "-i", "/k"]);
        assert_eq!(
            (h.ssh.as_str(), h.ift.as_str()),
            ("/tmp/fake-ssh", "/opt/ift")
        );
    }

    #[test]
    fn the_session_list_is_the_first_column_of_each_row() {
        let text =
            "abc123\t4242\t/srv\tzsh\t2026-10-03T00:00:00Z\t#3\twork\n\nd4e5\t9\t/x\tsh\t-\t-\t-\n";
        assert_eq!(parse_sessions(text), ["abc123", "d4e5"]);
        assert!(parse_sessions("").is_empty());
    }

    #[test]
    fn the_server_check_quotes_the_ift_path_and_names_each_failure() {
        let cmd = check_command("/opt/my ift/ift");
        assert!(cmd.starts_with("sh -c '"), "{cmd}");
        assert!(cmd.contains("ift is not installed on the server"));
        assert!(cmd.contains("iftd is not installed on the server"));
        assert!(
            cmd.contains("'\\''/opt/my ift/ift'\\''"),
            "the path is quoted inside the script: {cmd}"
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
