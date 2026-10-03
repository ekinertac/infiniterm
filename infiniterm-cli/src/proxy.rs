//! `ift proxy session <id> [--spawn ...]`: a session daemon's socket over this
//! process's stdin and stdout, so `ssh -T host ift proxy session <id>` carries
//! the daemon's protocol to another machine (#118, the remote instances).
//!
//! The wire is `iftd`'s own (`session_protocol`); this file parses none of it.
//! It copies bytes both ways and ends when either side closes. `--spawn`
//! starts the daemon first, with the flags `DaemonBackend::spawn_now` uses, so
//! the app can create a card's shell on a server and attach to it in one ssh
//! command. The daemon outlives this process, which is the point: a dropped
//! connection is a detach, and the next `ift proxy session <id>` reattaches.
//!
//! Called by `main.rs`; the client is `infiniterm-core/src/backend/remote.rs`.
//! Related: `attach.rs` (the same socket, for a person at a terminal).
//!
//! Non-obvious constraints:
//! - The session id arrives over ssh from another machine, so it is checked
//!   against a short alphabet before it becomes part of a path. `../x` must
//!   not reach a socket outside the sessions folder.
//! - Nothing is printed to stdout but the daemon's bytes: stdout IS the
//!   protocol. Every message to a person goes to stderr.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use infiniterm_core::paths;

/// A session id is the socket's file name stem: letters, digits, `-` and `_`.
const MAX_ID_LEN: usize = 64;

/// What `--spawn` asks `iftd` for.
#[derive(Debug, PartialEq, Eq, Default)]
pub struct SpawnArgs {
    pub cwd: Option<String>,
    pub cmd: Option<String>,
    pub env: Vec<String>,
    pub buffer: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Parsed {
    pub id: String,
    pub spawn: Option<SpawnArgs>,
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `session <id> [--spawn] [--cwd D] [--cmd C] [--env K=V]... [--buffer N]`.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    if args.first().map(String::as_str) != Some("session") {
        return Err("ift proxy takes: session <id>".into());
    }
    let id = args.get(1).ok_or("ift proxy session takes an id")?;
    if !valid_id(id) {
        return Err(format!("not a session id: {id}"));
    }
    let mut spawn = false;
    let mut s = SpawnArgs::default();
    let mut it = args[2..].iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} takes a value"))
        };
        match a.as_str() {
            "--spawn" => spawn = true,
            "--cwd" => s.cwd = Some(value("--cwd")?),
            "--cmd" => s.cmd = Some(value("--cmd")?),
            "--env" => s.env.push(value("--env")?),
            "--buffer" => s.buffer = Some(value("--buffer")?),
            other => return Err(format!("unknown option {other}")),
        }
    }
    if !spawn && s != SpawnArgs::default() {
        return Err("--cwd, --cmd, --env and --buffer need --spawn".into());
    }
    Ok(Parsed {
        id: id.clone(),
        spawn: spawn.then_some(s),
    })
}

/// Copies `from` to `to` until `from` ends, then closes `to`'s write side so
/// the other end sees the end too.
fn pump(mut from: impl Read, mut to: impl Write, then: impl FnOnce()) {
    let mut buf = [0u8; 16 * 1024];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if to.write_all(&buf[..n]).and_then(|_| to.flush()).is_err() {
                    break;
                }
            }
        }
    }
    then();
}

/// Both directions between `socket` and a reader/writer pair, returning when
/// the socket's side ends (the daemon exited or evicted us) or the input does.
pub fn splice(
    socket: UnixStream,
    input: impl Read + Send + 'static,
    mut output: impl Write,
) -> std::io::Result<()> {
    let to_socket = socket.try_clone()?;
    let closer = socket.try_clone()?;
    // Input to the socket on its own thread; when input ends the daemon is
    // told so by closing our write half, and its answer ends the loop below.
    std::thread::spawn(move || {
        pump(input, &to_socket, || {
            let _ = to_socket.shutdown(std::net::Shutdown::Write);
        })
    });
    pump(&socket, &mut output, || {});
    // The daemon's side is over: wake the input thread's write half too.
    let _ = closer.shutdown(std::net::Shutdown::Both);
    Ok(())
}

fn find_iftd() -> Option<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("iftd")));
    if let Some(p) = beside.filter(|p| p.is_file()) {
        return Some(p);
    }
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|d| d.join("iftd"))
            .find(|p| p.is_file())
    })
}

/// Starts `iftd` for `socket`. It binds before it forks and exits 0 when the
/// listener is up, so the exit status is the readiness signal.
fn spawn_daemon(socket: &Path, s: &SpawnArgs) -> Result<(), String> {
    let iftd = find_iftd().ok_or("iftd not found beside ift or on PATH")?;
    if let Some(dir) = socket.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut c = std::process::Command::new(&iftd);
    c.arg("--socket").arg(socket);
    c.arg("--cwd")
        .arg(s.cwd.clone().unwrap_or_else(|| paths::home_dir().to_string_lossy().into()));
    if let Some(b) = &s.buffer {
        c.arg("--buffer").arg(b);
    }
    if let Some(cmd) = &s.cmd {
        c.arg("--cmd").arg(cmd);
    }
    for kv in &s.env {
        c.arg("--env").arg(kv);
    }
    // iftd's own stdout is closed on purpose: ours is the protocol.
    c.stdout(std::process::Stdio::null());
    let status = c
        .status()
        .map_err(|e| format!("could not run {}: {e}", iftd.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} exited with {status}", iftd.display()))
    }
}

pub fn run(args: &[String]) -> ExitCode {
    let parsed = match parse(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ift: {e}");
            return ExitCode::from(2);
        }
    };
    let socket = paths::sessions_dir().join(format!("{}.sock", parsed.id));
    if let Some(s) = &parsed.spawn {
        if let Err(e) = spawn_daemon(&socket, s) {
            eprintln!("ift: {e}");
            return ExitCode::from(1);
        }
    }
    let stream = match UnixStream::connect(&socket) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ift: no session {}: {e}", parsed.id);
            return ExitCode::from(1);
        }
    };
    match splice(stream, std::io::stdin(), std::io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ift: {e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn an_id_is_a_short_plain_word() {
        assert!(valid_id("ccc081c9a2c44d6a"));
        assert!(valid_id("a-b_C9"));
        assert!(!valid_id(""));
        assert!(!valid_id("../x"));
        assert!(!valid_id("a/b"));
        assert!(!valid_id("a b"));
        assert!(!valid_id(&"a".repeat(MAX_ID_LEN + 1)));
    }

    #[test]
    fn session_alone_attaches_and_spawn_carries_its_options() {
        assert_eq!(
            parse(&a(&["session", "abc"])).unwrap(),
            Parsed {
                id: "abc".into(),
                spawn: None
            }
        );
        let p = parse(&a(&[
            "session", "abc", "--spawn", "--cwd", "/srv", "--cmd", "htop", "--env", "A=1",
            "--env", "B=2", "--buffer", "8",
        ]))
        .unwrap();
        assert_eq!(
            p.spawn,
            Some(SpawnArgs {
                cwd: Some("/srv".into()),
                cmd: Some("htop".into()),
                env: vec!["A=1".into(), "B=2".into()],
                buffer: Some("8".into()),
            })
        );
    }

    #[test]
    fn bad_use_is_refused_with_a_reason() {
        assert!(parse(&a(&[])).is_err());
        assert!(parse(&a(&["session"])).is_err());
        assert!(parse(&a(&["session", "../etc"])).is_err());
        assert!(parse(&a(&["session", "abc", "--cwd", "/x"])).is_err());
        assert!(parse(&a(&["session", "abc", "--spawn", "--cwd"])).is_err());
        assert!(parse(&a(&["session", "abc", "--nope"])).is_err());
    }

    // The relay with two socketpairs as the daemon and as the ssh side.
    #[test]
    fn bytes_go_both_ways_and_the_end_of_input_ends_the_relay() {
        let (daemon, ours) = UnixStream::pair().unwrap();
        let (input_far, input_near) = UnixStream::pair().unwrap();
        let handle = std::thread::spawn(move || {
            let mut out = Vec::new();
            splice(ours, input_near, &mut out).unwrap();
            out
        });
        let mut daemon_w = daemon.try_clone().unwrap();
        let mut daemon_r = daemon;
        // Daemon speaks first, as iftd does with Hello.
        daemon_w.write_all(b"hello").unwrap();
        // The far end of "ssh" types, then goes away.
        let mut far = input_far;
        far.write_all(b"typed").unwrap();
        far.shutdown(std::net::Shutdown::Write).unwrap();
        let mut got = [0u8; 5];
        daemon_r.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"typed");
        // Its end of input closed our write half; the daemon sees EOF and quits.
        let mut rest = Vec::new();
        daemon_r.read_to_end(&mut rest).unwrap();
        assert!(rest.is_empty());
        drop(daemon_w);
        drop(daemon_r);
        assert_eq!(handle.join().unwrap(), b"hello");
    }
}
