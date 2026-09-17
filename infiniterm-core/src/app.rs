//! The backend as one thing the UI owns: the PTY backend, the socket, the
//! process poller and the config watcher, each feeding a channel the UI
//! thread drains once per frame. This is the rewritten glue: the Tauri
//! app's lib.rs, ipc.rs and the `#[tauri::command]` functions.
//!
//! Receivers rather than callbacks because gpui's model is not `Send` and
//! every producer here is a thread: the UI polls `pane_events` before
//! painting (feeding `infiniterm-term`'s scheduler), routes `hook_reports`
//! through `agent_state.rs`, answers `cli_requests` through `CliState::reply`,
//! assigns `pane_status` wholesale, and re-merges `config_changes`. One
//! tagged stream for all panes, never one per card: the tmux backend of v2
//! delivers every pane interleaved on one socket, and this shape is why.
//!
//! `start` never fails: a socket that cannot bind disables hooks and `ift`
//! and says so on stderr, since a terminal that refuses to start over a
//! stale socket is worse than one without hooks.
use crate::backend::{PaneEvent, PaneId, Panes};
use crate::cli::{CliRequest, CliState};
use crate::config::TerminalBackend;
use crate::config_files::{config_watch, ConfigChange};
use crate::hooks::{listen, HookReport};
use crate::inspect::{pane_status_poll, PaneStatus};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

pub struct Backend {
    /// The shells: local PTYs, or tmux windows that outlive the window.
    pub pty: Panes,
    pub pane_events: Receiver<(PaneId, PaneEvent)>,
    pub hook_reports: Receiver<HookReport>,
    pub cli: Arc<CliState>,
    pub cli_requests: Receiver<CliRequest>,
    pub pane_status: Receiver<Vec<PaneStatus>>,
    pub config_changes: Receiver<ConfigChange>,
    /// Whether the socket bound; false means hooks and `ift` are off.
    pub socket_ok: bool,
    /// Why tmux was asked for and not used, if it was. The ui says it once.
    pub fell_back: Option<String>,
}

impl Backend {
    /// Starts every producer thread. `socket` is the path `ift` and the hook
    /// binary connect to (`paths::socket_path()` in the app; a temp path in
    /// tests, so a test never fights a running infiniterm for the real one).
    /// `session_buffer_mib` is ignored by every backend but `Daemon`.
    pub fn start(socket: &Path, backend: TerminalBackend, session_buffer_mib: usize) -> Backend {
        let (pty, pane_events, fell_back) = Panes::start(backend, session_buffer_mib);
        if let Some(why) = &fell_back {
            eprintln!("[infiniterm] {why}");
        }
        let (hook_tx, hook_reports) = channel();
        let cli = Arc::new(CliState::default());
        let (cli_tx, cli_requests) = channel();
        cli.subscribe(cli_tx);
        let socket_ok = match listen(socket, hook_tx, cli.clone()) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("[infiniterm] {e}; hook reports and ift disabled");
                false
            }
        };
        let (status_tx, pane_status) = channel();
        pane_status_poll(pty.pids_source(), status_tx);
        let (config_tx, config_changes) = channel();
        config_watch(config_tx);
        Backend {
            pty,
            pane_events,
            hook_reports,
            cli,
            cli_requests,
            pane_status,
            config_changes,
            socket_ok,
            fell_back,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    fn temp_socket(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("infiniterm-test-{tag}-{}.sock", std::process::id()))
    }

    // Phase 2's done-check: the socket answers ift and a hook report with
    // no window open. The "UI" is a thread draining the request channel.
    #[test]
    fn the_socket_answers_ift_and_forwards_hook_reports_with_no_window() {
        let path = temp_socket("app");
        let backend = Backend::start(&path, TerminalBackend::Pty, 4);
        assert!(backend.socket_ok);

        // A hook report, as infiniterm-hook writes it.
        let mut hook = UnixStream::connect(&path).unwrap();
        writeln!(
            hook,
            r#"{{"card_id":"c1","event":"Stop","payload":{{"transcript_path":"/s/x.jsonl"}}}}"#
        )
        .unwrap();
        drop(hook);
        let report = backend
            .hook_reports
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert_eq!(report.card_id, "c1");
        assert_eq!(report.transcript.as_deref(), Some("/s/x.jsonl"));

        // An ift verb, answered by the model side.
        let cli = backend.cli.clone();
        let requests = backend.cli_requests;
        let ui = std::thread::spawn(move || {
            let req = requests.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(req.cmd, "ls");
            assert_eq!(req.card_id.as_deref(), Some("c1"));
            cli.reply(req.id, true, "c1\t-\t~/Code\tnone\t-".into());
        });
        let mut ift = UnixStream::connect(&path).unwrap();
        writeln!(ift, r#"{{"cmd":"ls","card_id":"c1"}}"#).unwrap();
        let mut line = String::new();
        BufReader::new(ift.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        ui.join().unwrap();
        let reply: crate::cli::CliReply = serde_json::from_str(line.trim()).unwrap();
        assert!(reply.ok);
        assert!(reply.text.starts_with("c1\t"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_socket_that_cannot_bind_disables_hooks_but_the_backend_still_starts() {
        let backend = Backend::start(
            Path::new("/nonexistent-dir/infiniterm.sock"),
            TerminalBackend::Pty,
            4,
        );
        assert!(!backend.socket_ok);
        // The PTY side is untouched by that.
        assert!(backend.pane_events.try_recv().is_err());
    }
}
