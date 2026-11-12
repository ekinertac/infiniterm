//! `ift proxy join canvas | session <id>` on Windows: the host app's join
//! pipe (`paths::join_socket`, served by `backend/join.rs`) over this
//! process's stdin and stdout, so a Mac's `ssh host ift proxy join ...`
//! reaches a Windows host. The twin of the join half of `proxy.rs`.
//!
//! Only `join`. `proxy session` and `proxy files` need `iftd` and the file
//! service, which a Windows host does not have yet: they answer one line.
//!
//! The same contract as the unix proxy: nothing but the host's bytes goes to
//! stdout (stdout IS the protocol, every message to a person goes to stderr),
//! the host's answer line to the handshake is the first thing on stdout for
//! the viewer to check, and the proxy ends when either side closes.
//!
//! Two Windows facts shaped the ending. A pipe has no half-close, so when the
//! viewer's input ends (the ssh session is going away) the whole connection is
//! shut down, which the host reads as the viewer leaving; and Windows sshd
//! drops the tail of a child's output when stdin hits end of file early
//! (measured 2026-11-12, docs/windows-handoff.md), so nothing here may rely on
//! the host's last bytes surviving that moment. stdin and stdout are raw
//! bytes on a pipe; the viewer's ssh gives pipes, not a console.
//!
//! Called by `main.rs` (`unix_only_verb`, Windows arm); the pipe is
//! `infiniterm-core/src/pipe.rs`; the shared parsing is `proxy_join.rs`.

use std::io::{Read, Write};
use std::path::Path;
use std::process::ExitCode;

use infiniterm_core::paths;
use infiniterm_core::pipe::{self, PipeStream};

use crate::proxy_join::{parse_join, pump};

pub fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("join") => run_join(&args[1..]),
        other => {
            eprintln!(
                "ift: proxy {} is not available on Windows (only `proxy join`)",
                other.unwrap_or("")
            );
            ExitCode::from(2)
        }
    }
}

fn run_join(args: &[String]) -> ExitCode {
    let topic = match parse_join(args) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("ift: {e}");
            return ExitCode::from(2);
        }
    };
    let socket = paths::join_socket();
    match join_over(&socket, &topic, std::io::stdin(), std::io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ift: {e}");
            ExitCode::from(1)
        }
    }
}

/// Connects to the host's join pipe, says what is wanted, and splices.
pub fn join_over(
    socket: &Path,
    topic: &str,
    input: impl Read + Send + 'static,
    output: impl Write,
) -> Result<(), String> {
    let mut stream = pipe::connect(socket).map_err(|e| {
        format!(
            "no running infiniterm to join on this host ({}): {e}",
            socket.display()
        )
    })?;
    writeln!(
        stream,
        "ift-join {} {topic}",
        infiniterm_core::backend::JOIN_MAJOR
    )
    .map_err(|e| e.to_string())?;
    splice(stream, input, output).map_err(|e| e.to_string())
}

/// Both directions between the pipe and a reader/writer pair, returning when
/// the host's side ends or the input does.
pub fn splice(
    stream: PipeStream,
    input: impl Read + Send + 'static,
    mut output: impl Write,
) -> std::io::Result<()> {
    let to_host = stream.try_clone()?;
    let closer = stream.try_clone()?;
    // Input to the host on its own thread. When it ends the viewer is gone:
    // a pipe cannot close one direction, so the whole connection is shut down
    // and the host's reader sees the end.
    std::thread::spawn(move || {
        pump(input, &to_host, || {
            let _ = to_host.shutdown(std::net::Shutdown::Both);
        })
    });
    pump(&stream, &mut output, || {});
    // The host's side is over: wake the input thread's write too.
    let _ = closer.shutdown(std::net::Shutdown::Both);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use infiniterm_core::pipe::PipeListener;
    use std::io::BufRead;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Input that delivers its data and then stays open until told to end:
    /// the way the viewer's ssh holds stdin, so the host's output is not cut.
    struct HeldOpen {
        data: std::io::Cursor<Vec<u8>>,
        end: mpsc::Receiver<()>,
    }

    impl Read for HeldOpen {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.data.read(buf)?;
            if n > 0 {
                return Ok(n);
            }
            let _ = self.end.recv();
            Ok(0)
        }
    }

    fn payload(len: usize) -> Vec<u8> {
        (0..=255u8).cycle().take(len).collect()
    }

    #[test]
    fn a_proxy_says_hello_then_carries_every_byte_both_ways_and_ends_with_the_host() {
        let path = std::path::PathBuf::from(format!(
            r"\\.\pipe\infiniterm-test-proxywin-{}",
            std::process::id()
        ));
        let mut listener = PipeListener::bind(&path).unwrap();
        let from_host = payload(300_000);
        let from_viewer = payload(120_000);
        let (viewer_got_tx, viewer_got_rx) = mpsc::channel();
        let sent = from_host.clone();
        let server = std::thread::spawn(move || {
            let stream = listener.accept().unwrap();
            let mut out = stream.try_clone().unwrap();
            let mut reader = std::io::BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            writeln!(out, "ift-join 2 ok").unwrap();
            out.write_all(&sent).unwrap();
            let mut got = vec![0u8; 120_000];
            reader.read_exact(&mut got).unwrap();
            viewer_got_tx.send((line, got)).unwrap();
            // Dropping the stream ends the connection: the proxy must return.
        });
        let (end_tx, end_rx) = mpsc::channel();
        let input = HeldOpen {
            data: std::io::Cursor::new(from_viewer.clone()),
            end: end_rx,
        };
        let mut output = Vec::new();
        let result = std::thread::scope(|s| {
            let h = s.spawn(|| join_over(&path, "canvas", input, &mut output));
            let (line, got) = viewer_got_rx.recv_timeout(Duration::from_secs(20)).unwrap();
            assert_eq!(line, "ift-join 2 canvas\n", "the handshake line");
            assert!(got == from_viewer, "the viewer's bytes reached the host intact");
            server.join().unwrap();
            let r = h.join().unwrap();
            let _ = end_tx.send(());
            r
        });
        result.unwrap();
        let mut want = b"ift-join 2 ok\n".to_vec();
        want.extend_from_slice(&from_host);
        assert!(output == want, "the answer line and the host's bytes, byte for byte");
    }

    #[test]
    fn no_app_running_says_so_and_other_verbs_are_refused() {
        let missing = std::path::PathBuf::from(r"\\.\pipe\infiniterm-test-proxywin-nobody");
        let err = join_over(&missing, "canvas", std::io::empty(), Vec::new()).unwrap_err();
        assert!(err.contains("no running infiniterm"), "{err}");
        assert_eq!(
            format!("{:?}", run(&["session".to_string(), "abc".to_string()])),
            format!("{:?}", ExitCode::from(2))
        );
    }
}
