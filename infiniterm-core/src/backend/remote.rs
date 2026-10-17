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
//! - Hooks and `ift` on the server reach this instance through ONE more ssh
//!   (`forward`): a reverse unix-socket forward from `REMOTE_SOCKET` on the
//!   server to this instance's own socket. `infiniterm-hook` and `ift` already
//!   look in `<tmp>/infiniterm.sock`, so the server needs no change.

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

/// The host a viewer window joins (`ift join`, #423): `INFINITERM_JOIN` is the
/// ssh target, with the same optional `_ARGS`, `_IFT` and `_SSH` as a remote
/// instance. Its cards are the host app's, never started here.
pub fn join_from_env() -> Option<RemoteHost> {
    let var = |k: &str| std::env::var(k).ok();
    from_vars(
        var("INFINITERM_JOIN").as_deref(),
        var("INFINITERM_JOIN_ARGS").as_deref(),
        var("INFINITERM_JOIN_IFT").as_deref(),
        var("INFINITERM_JOIN_SSH").as_deref(),
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

/// Where `ift` is looked for beyond the PATH. A non-interactive ssh has a
/// short PATH, and the installer puts `ift` in `~/.local/bin` for anyone but
/// root (`install.rs`), so a plain `ift` was "not installed" on a server it was
/// installed on (#397).
pub const INSTALL_DIRS: [&str; 2] = ["$HOME/.local/bin", "/usr/local/bin"];

/// The script `check` runs on the server: `ift` is there (on the PATH or in
/// `INSTALL_DIRS`, for a bare name), `iftd` is beside it or on the PATH, and
/// `ift sessions` works (so its data directory does). It prints the full path
/// of the `ift` it found. Each failure says what is missing on stderr and uses
/// its own exit status.
pub fn check_command(ift: &str) -> String {
    let dirs = INSTALL_DIRS.join(" ");
    let script = format!(
        concat!(
            "ift={ift}; ",
            "p=$(command -v \"$ift\"); ",
            "if [ -z \"$p\" ]; then case \"$ift\" in */*) ;; *) ",
            "for d in {dirs}; do [ -x \"$d/$ift\" ] && p=\"$d/$ift\" && break; done;; esac; fi; ",
            "[ -n \"$p\" ] || {{ echo 'ift is not installed on the server' >&2; exit 11; }}; ",
            "d=$(dirname \"$p\"); ",
            "[ -x \"$d/iftd\" ] || command -v iftd >/dev/null || ",
            "{{ echo 'iftd is not installed on the server' >&2; exit 12; }}; ",
            "\"$p\" sessions >/dev/null && printf '%s\\n' \"$p\""
        ),
        ift = shell_quote(ift),
        dirs = dirs
    );
    format!("sh -c {}", shell_quote(&script))
}

/// Can this host run infiniterm's server half: ssh works without a prompt,
/// `ift` and `iftd` are installed, and the sessions folder answers. Returns the
/// full path of the `ift` there, which the caller uses from then on; the error
/// is a sentence for a person.
pub fn check(host: &RemoteHost) -> Result<String, String> {
    let out = run_on(host, &check_command(&host.ift))
        .map_err(|e| format!("could not run {}: {e}", host.ssh))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
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
    ssh_args_for(host, remote_command(&host.ift, id, spawn))
}

/// `ift proxy join session <id>`: the host app's join socket for one card
/// (#423), quoted like `remote_command`.
pub fn join_command(ift: &str, id: &str) -> String {
    format!(
        "{} proxy join session {}",
        shell_quote(ift),
        shell_quote(id)
    )
}

/// `ift proxy join canvas`: the host's layout snapshots.
pub fn join_canvas_command(ift: &str) -> String {
    format!("{} proxy join canvas", shell_quote(ift))
}

/// What the host's join server says first, as one line: `ift-join <major> ok`
/// or `ift-join <major> error <sentence>`. The sentence is the host's own,
/// shown as it is: it already says what to do.
pub fn check_join_answer(line: &str) -> Result<(), String> {
    let line = line.trim_end();
    let mut words = line.splitn(4, ' ');
    match (words.next(), words.next(), words.next(), words.next()) {
        (Some("ift-join"), Some(_), Some("ok"), None) => Ok(()),
        (Some("ift-join"), Some(_), Some("error"), Some(why)) => Err(why.to_string()),
        _ => Err(format!(
            "the host did not answer as ift-join (is infiniterm running there, and is it a build with join?): {line:?}"
        )),
    }
}

/// The argument list for an ssh that runs `command` on the host.
pub fn ssh_args_for(host: &RemoteHost, command: String) -> Vec<String> {
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
    args.push(command);
    args
}

/// Where the server's hooks and `ift` look for the app: their default, which
/// is `/tmp/infiniterm.sock` on Linux. Two Macs connected to one server share
/// it and the later connection wins; a per-Mac path would need the cards'
/// environment to carry it.
pub const REMOTE_SOCKET: &str = "/tmp/infiniterm.sock";

/// How long the forward waits before it is started again after it ends.
const FORWARD_RETRY: std::time::Duration = std::time::Duration::from_secs(5);

/// Arguments for the ssh that carries the reverse forward. The remote command
/// is `cat`, not `-N`: ssh ends when `cat` sees its stdin close, and that
/// stdin is a pipe from THIS process, so a forward never outlives the app.
/// The server's own sshd decides whether an old socket file may be replaced
/// (`StreamLocalBindUnlink`, off by default), so `forward` removes the file
/// itself first: a forward that ended leaves it behind and blocks the next.
pub fn forward_args(host: &RemoteHost, local_socket: &str) -> Vec<String> {
    let mut args: Vec<String> = [
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=3",
        "-o",
        "ExitOnForwardFailure=yes",
        "-R",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.push(format!("{REMOTE_SOCKET}:{local_socket}"));
    args.extend(host.ssh_args.iter().cloned());
    args.push(host.target.clone());
    args.push("cat >/dev/null".into());
    args
}

/// Keeps the reverse forward up for as long as the app runs: a thread that
/// starts the ssh, waits for it to end and starts it again. A host that is
/// down costs one failed ssh every few seconds, nothing else.
pub fn forward(host: RemoteHost, local_socket: String) {
    std::thread::spawn(move || loop {
        let _ = run_on(&host, &format!("rm -f {}", shell_quote(REMOTE_SOCKET)));
        let child = Command::new(&host.ssh)
            .args(forward_args(&host, &local_socket))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            // `wait` closes the child's stdin first, which would end `cat`
            // and the forward with it: hold the pipe until ssh is gone.
            let _stdin = child.stdin.take();
            let _ = child.wait();
        }
        std::thread::sleep(FORWARD_RETRY);
    });
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
    connect_command(host, remote_command(&host.ift, id, spawn))
}

/// Joins one card of the host's running canvas (#423): the same splice, to
/// `ift proxy join session <id>`, with the host's answer line read and checked
/// before anything else, so the caller's next read is the host's `Hello`.
pub fn connect_join(host: &RemoteHost, id: &str) -> Result<(UnixStream, SshStderr), JoinFailure> {
    connect_join_with(host, join_command(&host.ift, id))
}

/// Joins the host's canvas: its layout and agent states (#423).
pub fn connect_join_canvas(host: &RemoteHost) -> Result<(UnixStream, SshStderr), JoinFailure> {
    connect_join_with(host, join_canvas_command(&host.ift))
}

/// Whether the host can be joined right now: one canvas handshake, closed at
/// once. What `ift join` asks before it opens a window (#423).
pub fn probe_join(host: &RemoteHost) -> Result<(), JoinFailure> {
    connect_join_canvas(host).map(|_| ())
}

/// Why joining a host failed, and whether trying again can help (#423).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinFailure {
    /// The connection itself failed (ssh down, a timeout, a reset): it can
    /// work later, so the viewer tries again.
    Retry(String),
    /// The host answered, and waiting will not change what it said: an `ift`
    /// too old to join, a version it refuses, no app running there, a login
    /// that was refused. A person has to do something, so the viewer says so
    /// once and does not retry.
    Fatal(String),
}

impl JoinFailure {
    pub fn message(&self) -> &str {
        match self {
            JoinFailure::Retry(m) | JoinFailure::Fatal(m) => m,
        }
    }
}

/// What came back where the host's answer line should be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// A line.
    Line(String),
    /// The connection ended first (ssh exited, the remote command failed).
    Closed,
    /// Nothing within the wait.
    TimedOut,
    /// More than a line's worth of bytes with no line end.
    Unreadable,
}

/// What ssh says when the CONNECTION is what failed, in the words OpenSSH
/// uses. Matched in lower case anywhere in its stderr.
const CONNECTION_WORDS: [&str; 11] = [
    "connection refused",
    "connection timed out",
    "operation timed out",
    "connection reset",
    "connection closed",
    "network is unreachable",
    "no route to host",
    "could not resolve hostname",
    "broken pipe",
    "host is down",
    "kex_exchange_identification",
];

/// Decides whether a failed handshake is worth another try. Pure, so the rule
/// is tested without ssh: only a real connection failure retries; anything the
/// host itself said (or its ssh refused) is fatal, with a sentence.
pub fn classify_join_failure(target: &str, answer: &Answer, stderr: &str) -> JoinFailure {
    match answer {
        Answer::Line(line) => match check_join_answer(line) {
            // The host's own sentence, or the line it did say.
            Err(why) if line.trim_start().starts_with("ift-join ") => JoinFailure::Fatal(why),
            _ => JoinFailure::Fatal(format!(
                "{target} has an ift that cannot be joined (it did not answer as ift-join); update infiniterm there"
            )),
        },
        Answer::TimedOut => JoinFailure::Retry(format!("{target} did not answer in time")),
        Answer::Unreadable => JoinFailure::Fatal(format!(
            "{target} sent something that is not an answer; update infiniterm there"
        )),
        Answer::Closed => {
            let said = stderr.trim();
            let lower = said.to_lowercase();
            if said.is_empty() {
                JoinFailure::Retry(format!("{target} closed the connection before it answered"))
            } else if CONNECTION_WORDS.iter().any(|w| lower.contains(w)) {
                JoinFailure::Retry(said.lines().last().unwrap_or(said).to_string())
            } else {
                JoinFailure::Fatal(said.lines().last().unwrap_or(said).to_string())
            }
        }
    }
}

fn connect_join_with(
    host: &RemoteHost,
    command: String,
) -> Result<(UnixStream, SshStderr), JoinFailure> {
    let (mut stream, stderr) = connect_command(host, command)
        .map_err(|e| JoinFailure::Fatal(format!("could not run {}: {e}", host.ssh)))?;
    match read_line(&mut stream, JOIN_ANSWER_WAIT) {
        Ok(line) => match check_join_answer(&line) {
            Ok(()) => Ok((stream, stderr)),
            Err(_) => Err(classify_join_failure(
                &host.target,
                &Answer::Line(line),
                &stderr.text(),
            )),
        },
        Err(answer) => {
            // ssh may say why a moment after the stream ends.
            std::thread::sleep(std::time::Duration::from_millis(150));
            Err(classify_join_failure(&host.target, &answer, &stderr.text()))
        }
    }
}

/// How long a host gets to answer the handshake line.
const JOIN_ANSWER_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// One line, a byte at a time, so nothing after it is taken from the frames
/// that follow. Bounded: a line over 256 bytes is not an answer.
fn read_line(stream: &mut UnixStream, wait: std::time::Duration) -> Result<String, Answer> {
    stream
        .set_read_timeout(Some(wait))
        .map_err(|_| Answer::Closed)?;
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    loop {
        match stream.read(&mut b) {
            Ok(0) => return Err(Answer::Closed),
            Ok(_) if b[0] == b'\n' => break,
            Ok(_) if line.len() < 256 => line.push(b[0]),
            Ok(_) => return Err(Answer::Unreadable),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Err(Answer::TimedOut)
            }
            Err(_) => return Err(Answer::Closed),
        }
    }
    let _ = stream.set_read_timeout(None);
    Ok(String::from_utf8_lossy(&line).into_owned())
}

/// Starts ssh with `command` on the host; the splice both `connect` and
/// `connect_join` use.
pub fn connect_command(
    host: &RemoteHost,
    command: String,
) -> std::io::Result<(UnixStream, SshStderr)> {
    let mut child = Command::new(&host.ssh)
        .args(ssh_args_for(host, command))
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
    fn a_join_runs_ift_proxy_join_with_every_value_quoted() {
        assert_eq!(join_command("ift", "abc-1"), "ift proxy join session abc-1");
        assert_eq!(
            join_command("/opt/my ift", "a b"),
            "'/opt/my ift' proxy join session 'a b'"
        );
        assert_eq!(join_canvas_command("ift"), "ift proxy join canvas");
    }

    #[test]
    fn the_hosts_answer_is_ok_or_its_own_sentence() {
        assert_eq!(check_join_answer("ift-join 1 ok"), Ok(()));
        assert_eq!(check_join_answer("ift-join 0 ok\n"), Ok(()));
        assert_eq!(
            check_join_answer("ift-join 1 error no session x on this host"),
            Err("no session x on this host".to_string())
        );
        let err = check_join_answer("bash: ift: command not found").unwrap_err();
        assert!(err.contains("did not answer as ift-join"), "{err}");
        assert!(check_join_answer("").is_err());
    }

    #[test]
    fn only_a_real_connection_failure_is_worth_another_try() {
        use Answer::*;
        let t = "ekin-mini";
        let retry =
            |a: &Answer, e: &str| matches!(classify_join_failure(t, a, e), JoinFailure::Retry(_));
        // The connection failed: try again.
        assert!(retry(&TimedOut, ""));
        assert!(retry(&Closed, ""));
        assert!(retry(
            &Closed,
            "ssh: connect to host ekin-mini port 22: Connection refused"
        ));
        assert!(retry(
            &Closed,
            "ssh: Could not resolve hostname ekin-mini: nodename nor servname"
        ));
        assert!(retry(&Closed, "Connection closed by 100.1.2.3 port 22"));
        // The host (or its login) said no: a person has to act.
        assert!(!retry(&Closed, "bash: ift: command not found"));
        assert!(!retry(
            &Closed,
            "ekin@ekin-mini: Permission denied (publickey)."
        ));
        assert!(!retry(
            &Closed,
            "ift: no running infiniterm to join on this host (/x/j.sock): not found"
        ));
        assert!(!retry(&Unreadable, ""));
    }

    #[test]
    fn a_host_that_cannot_be_joined_is_told_once_with_a_sentence() {
        let t = "ekin-mini";
        // An older ift: a shell error where the answer line should be.
        let old = classify_join_failure(t, &Answer::Line("ift: unknown command join".into()), "");
        assert_eq!(
            old,
            JoinFailure::Fatal(
                "ekin-mini has an ift that cannot be joined (it did not answer as ift-join); update infiniterm there"
                    .into()
            )
        );
        // The host's own refusal is shown as it is.
        let newer = classify_join_failure(
            t,
            &Answer::Line("ift-join 1 error this host speaks ift-join 1 and you asked for 2; update infiniterm on the host".into()),
            "",
        );
        assert_eq!(
            newer,
            JoinFailure::Fatal(
                "this host speaks ift-join 1 and you asked for 2; update infiniterm on the host"
                    .into()
            )
        );
        // Of a failed connection, the last line ssh said.
        let last = classify_join_failure(
            t,
            &Answer::Closed,
            "warning: x\nbash: ift: command not found",
        );
        assert_eq!(
            last,
            JoinFailure::Fatal("bash: ift: command not found".into())
        );
    }

    #[test]
    fn the_forward_maps_the_servers_socket_to_ours_and_ends_with_its_stdin() {
        let mut host = RemoteHost::new("root@h");
        host.ssh_args = vec!["-p".into(), "2222".into()];
        let args = forward_args(&host, "/data/infiniterm.sock");
        let at = args.iter().position(|a| a == "-R").unwrap();
        assert_eq!(args[at + 1], "/tmp/infiniterm.sock:/data/infiniterm.sock");
        assert!(args.contains(&"ExitOnForwardFailure=yes".to_string()));
        assert_eq!(
            &args[args.len() - 4..],
            ["-p", "2222", "root@h", "cat >/dev/null"]
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

    // #397: run the script itself, on a short PATH like a non-interactive ssh
    // has, with `ift` only in ~/.local/bin.
    fn run_check(home: &std::path::Path, ift: &str) -> std::process::Output {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(check_command(ift))
            .env_clear()
            .env("HOME", home)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap()
    }

    // A fresh HOME for one test, named so parallel tests do not share one.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ift-397-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn stub(dir: &std::path::Path, name: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        let f = dir.join(name);
        std::fs::write(&f, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn the_check_finds_ift_in_the_install_folder_and_says_where() {
        let home = scratch("finds");
        let bin = home.join(".local/bin");
        stub(&bin, "ift");
        stub(&bin, "iftd");
        let out = run_check(&home, "ift");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            bin.join("ift").to_str().unwrap()
        );
    }

    #[test]
    fn the_check_still_says_not_installed_when_it_is_nowhere() {
        let home = scratch("nowhere");
        let out = run_check(&home, "ift");
        assert_eq!(out.status.code(), Some(11));
        assert!(String::from_utf8_lossy(&out.stderr).contains("ift is not installed"));
    }

    // A path the user gave is taken as it is: no search for another `ift`.
    #[test]
    fn a_given_path_is_not_searched_for_elsewhere() {
        let home = scratch("given");
        let bin = home.join(".local/bin");
        stub(&bin, "ift");
        stub(&bin, "iftd");
        let out = run_check(&home, "/nowhere/ift");
        assert_eq!(out.status.code(), Some(11));
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
