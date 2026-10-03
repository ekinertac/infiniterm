//! PTY ownership. The trait exists so v2's tmux control-mode backend follows the
//! same shape: it owns ALL panes and emits ONE stream tagged by pane id, because
//! tmux control mode delivers every pane's output interleaved on a single socket.
//! Per-pane readers would have to be unwound to add it.
//!
//! `async fn` in this trait makes it non-dyn-compatible (no `&dyn SessionBackend`,
//! E0038), so the v1/v2 swap is not virtual dispatch: v1 wires up the concrete
//! `LocalPtyBackend` directly, and v2 will choose between backends with an enum
//! wrapper instead of a trait object.
//!
//! Moved from the Tauri app's `src-tauri/src/backend/` unchanged; the ui
//! crate drains the receiver `LocalPtyBackend::new` returns once per frame
//! and feeds `infiniterm-term`'s scheduler. See the reference's
//! docs/superpowers/specs/2026-09-10-infiniterm-design.md, "SessionBackend".

use crate::config::TerminalBackend;
use crate::paths;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

pub mod daemon;
pub mod local_pty;
pub mod remote;
pub mod session_protocol;
pub mod tmux;
pub mod tmux_protocol;

pub type PaneId = u32;

#[derive(Debug, Clone)]
pub enum PaneEvent {
    Output(Vec<u8>),
    /// Bulk history replayed into a fresh emulator. Never emitted in v1; the path
    /// exists so v2 (tmux reattach) and workspace rehydration need no new code path.
    Replay(Vec<u8>),
    Exited {
        code: i32,
    },
    /// The daemon closed our socket without the shell exiting: another
    /// client attached (`ift attach`) and displaced us. The shell is fine
    /// and the session is still ours to take back once that client lets go
    /// (`Panes::session_attached`). Only the daemon backend emits it.
    Detached,
    TitleChanged(String),
    CwdChanged(PathBuf),
}

// The trait is used only inside this workspace, where the concrete type is
// known; the Send bound on the future is not needed (see the header).
#[allow(async_fn_in_trait)]
pub trait SessionBackend: Send + Sync {
    /// Async even though LocalPty answers instantly — tmux assigns ids
    /// asynchronously via %begin/%end, so the signature must not change in v2.
    async fn spawn(
        &self,
        cwd: &Path,
        cmd: Option<&str>,
        env: Vec<(String, String)>,
    ) -> anyhow::Result<PaneId>;

    fn write(&self, pane: PaneId, bytes: &[u8]);
    /// Fire-and-forget. tmux may refuse or adjust; reconcile from events.
    fn resize(&self, pane: PaneId, cols: u16, rows: u16);
    fn kill(&self, pane: PaneId);
    /// The consumer has finished with `bytes` of this pane's output. This is
    /// the backpressure: a backend stops reading a pane once too much of its
    /// output is unacknowledged, which stalls the child at the kernel's pty
    /// buffer — the only place a `yes` can be made to wait.
    fn ack(&self, pane: PaneId, bytes: usize);
}

/// Which backend a card's shell lives in. An enum rather than a trait
/// object because `SessionBackend` is not dyn-compatible (async fn, E0038),
/// which the header above has said since v1 and is now the reason this
/// exists. Every method is the `_now` name the ui already calls, so the
/// call sites do not know or care which one they have.
pub enum Panes {
    Local(local_pty::LocalPtyBackend),
    Tmux(tmux::TmuxBackend),
    Daemon(daemon::DaemonBackend),
}

impl Panes {
    /// tmux or the daemon when asked for AND there is one to talk to (a
    /// `tmux` binary on PATH, an `iftd` sidecar beside the app or on PATH).
    /// A missing binary must not mean no terminal, so both fall back to
    /// local shells and say so; `pty` has nothing to fall back from.
    pub fn start(
        backend: TerminalBackend,
        buffer_mib: usize,
    ) -> (Panes, Receiver<(PaneId, PaneEvent)>, Option<String>) {
        match backend {
            // A remote instance (`ift connect`) runs its cards on the host
            // `INFINITERM_REMOTE` names, whatever the setting says, and needs no
            // `iftd` on THIS Mac: the daemons are on the server.
            _ if remote::from_env().is_some() => {
                let host = remote::from_env().expect("checked by the guard");
                let (backend, rx) = daemon::DaemonBackend::new_remote(host, buffer_mib);
                (Panes::Daemon(backend), rx, None)
            }
            TerminalBackend::Pty => {
                let (pty, rx) = local_pty::LocalPtyBackend::new();
                (Panes::Local(pty), rx, None)
            }
            TerminalBackend::Tmux => {
                if !tmux::available() {
                    let (pty, rx) = local_pty::LocalPtyBackend::new();
                    return (
                        Panes::Local(pty),
                        rx,
                        Some("tmux is not installed; cards use local shells".into()),
                    );
                }
                match tmux::TmuxBackend::start() {
                    Some((backend, rx)) => (Panes::Tmux(backend), rx, None),
                    None => {
                        let (pty, rx) = local_pty::LocalPtyBackend::new();
                        (
                            Panes::Local(pty),
                            rx,
                            Some("tmux would not start; cards use local shells".into()),
                        )
                    }
                }
            }
            TerminalBackend::Daemon => {
                if !daemon::available() {
                    let (pty, rx) = local_pty::LocalPtyBackend::new();
                    return (
                        Panes::Local(pty),
                        rx,
                        Some("iftd is not installed; cards use local shells".into()),
                    );
                }
                let (backend, rx) = daemon::DaemonBackend::new(paths::sessions_dir(), buffer_mib);
                (Panes::Daemon(backend), rx, None)
            }
        }
    }

    /// Whether OUR emulator is the thing programs are talking to. False only
    /// under tmux, which is a terminal in its own right and answers colour and
    /// device queries before we can; our second answer then reaches the program
    /// as keystrokes. See `TerminalBody::replies` and tmux bug 9.
    pub fn we_are_the_terminal(&self) -> bool {
        !matches!(self, Panes::Tmux(_))
    }

    /// This card's shell, as an opaque handle for the save file: a tmux
    /// window id (`@7`) or a daemon session id. Local shells have none:
    /// there is nothing to come back to.
    pub fn session_id(&self, pane: PaneId) -> Option<String> {
        match self {
            Panes::Local(_) => None,
            Panes::Tmux(b) => b.window_id(pane),
            Panes::Daemon(b) => b.session_id(pane),
        }
    }

    /// Takes over a session left running by an earlier launch. `None` when
    /// this is the local backend, or the session has gone; the caller
    /// spawns a fresh shell as it always did.
    pub fn adopt(&self, session: &str) -> Option<PaneId> {
        match self {
            Panes::Local(_) => None,
            Panes::Tmux(b) => Some(b.adopt(session)),
            Panes::Daemon(b) => b.adopt(session),
        }
    }

    /// Sessions this app made that no card claims: what a crash left
    /// behind. Does nothing under the local backend, which has no litter
    /// to leave.
    pub fn kill_orphans(&self, claimed: &[String]) {
        match self {
            Panes::Local(_) => {}
            Panes::Tmux(b) => b.kill_orphans(claimed),
            Panes::Daemon(b) => b.kill_orphans(claimed),
        }
    }

    /// Which sessions are still there to adopt.
    /// Whether a daemon session has a client on it right now, from its
    /// meta file; `None` when there is no such session or no daemon backend.
    pub fn session_attached(&self, session_id: &str) -> Option<bool> {
        match self {
            Panes::Daemon(b) => b.session_attached(session_id),
            Panes::Local(_) | Panes::Tmux(_) => None,
        }
    }

    /// A dead session's scrollback from disk, daemon backend only: the
    /// local pty has no daemon to have written one and tmux keeps its own.
    pub fn take_ring(&self, session_id: &str) -> Option<(Vec<u8>, String)> {
        match self {
            Panes::Daemon(b) => b.take_ring(session_id),
            Panes::Local(_) | Panes::Tmux(_) => None,
        }
    }

    pub fn live_sessions(&self) -> Vec<String> {
        match self {
            Panes::Local(_) => vec![],
            Panes::Tmux(_) => tmux::TmuxBackend::live_windows(),
            Panes::Daemon(b) => b.live_sessions_now(),
        }
    }

    pub fn spawn_now(
        &self,
        cwd: &Path,
        cmd: Option<&str>,
        env: Vec<(String, String)>,
    ) -> anyhow::Result<PaneId> {
        match self {
            Panes::Local(b) => b.spawn_now(cwd, cmd, env),
            Panes::Tmux(b) => b.spawn_now(cwd, cmd, env),
            Panes::Daemon(b) => b.spawn_now(cwd, cmd, env),
        }
    }

    pub fn write_now(&self, pane: PaneId, bytes: &[u8]) {
        match self {
            Panes::Local(b) => b.write_now(pane, bytes),
            Panes::Tmux(b) => b.write_now(pane, bytes),
            Panes::Daemon(b) => b.write_now(pane, bytes),
        }
    }

    pub fn resize_now(&self, pane: PaneId, cols: u16, rows: u16) {
        match self {
            Panes::Local(b) => b.resize_now(pane, cols, rows),
            Panes::Tmux(b) => b.resize_now(pane, cols, rows),
            Panes::Daemon(b) => b.resize_now(pane, cols, rows),
        }
    }

    pub fn ack_now(&self, pane: PaneId, bytes: usize) {
        match self {
            Panes::Local(b) => b.ack_now(pane, bytes),
            Panes::Tmux(b) => b.ack_now(pane, bytes),
            Panes::Daemon(b) => b.ack_now(pane, bytes),
        }
    }

    pub fn kill(&self, pane: PaneId) {
        match self {
            Panes::Local(b) => b.kill(pane),
            Panes::Tmux(b) => b.kill_now(pane),
            Panes::Daemon(b) => b.kill_now(pane),
        }
    }

    /// Only the daemon can hand a closed card's running program back to a
    /// reopened one (`Model::can_park`): the local backend has no session
    /// to adopt, and tmux was left as it was.
    pub fn can_park(&self) -> bool {
        matches!(self, Panes::Daemon(_))
    }

    /// Lets go of a parked card's pane so its reopened card can adopt the
    /// session again. True when the session is left running for that.
    pub fn release(&self, pane: PaneId) -> bool {
        match self {
            Panes::Daemon(b) => {
                b.detach_now(pane);
                true
            }
            _ => false,
        }
    }

    pub fn write(&self, pane: PaneId, bytes: &[u8]) {
        self.write_now(pane, bytes)
    }

    /// Every pane ended. Under tmux or the daemon this kills the session: it
    /// is what `app.reload` means, and a reload that left the sessions would
    /// grow a second set beside them.
    pub fn kill_all(&self) {
        match self {
            Panes::Local(b) => b.kill_all(),
            Panes::Tmux(b) => b.kill_all(),
            Panes::Daemon(b) => b.kill_all(),
        }
    }

    /// The app is quitting. Local shells die with it; tmux windows and
    /// daemon sessions are LEFT RUNNING, which is the entire point of
    /// either backend.
    pub fn leave(&self) {
        match self {
            Panes::Local(b) => b.kill_all(),
            Panes::Tmux(b) => b.detach(),
            Panes::Daemon(b) => b.detach(),
        }
    }

    /// Live pane pids, for the remote-session poller. tmux answers nothing:
    /// a tmux pane's process lives wherever the server does, which the
    /// header of `pids_source` has always said. The daemon answers real
    /// pids, unlike tmux: iftd always learns its child's pid before it ever
    /// says `Hello`.
    pub fn pids_source(&self) -> Box<dyn Fn() -> Vec<(PaneId, u32)> + Send + 'static> {
        match self {
            Panes::Local(b) => Box::new(b.pids_source()),
            Panes::Tmux(_) => Box::new(Vec::new),
            Panes::Daemon(b) => Box::new(b.pids_source()),
        }
    }
}

#[cfg(test)]
mod panes_tests {
    use super::*;

    // tmux is the ONLY backend that answers terminal queries itself. Getting
    // this backwards sends our answers into a program that already got
    // tmux's and they arrive as keystrokes: that was tmux bug 9, and it is
    // what made Claude Code "a mess" for an evening.
    //
    // Local shells and a missing `iftd` both land on `Panes::Local`, so this
    // also covers "iftd not found" without needing iftd built for this test.
    #[test]
    fn only_tmux_answers_for_itself() {
        assert!(Panes::start(TerminalBackend::Pty, 4)
            .0
            .we_are_the_terminal());
        assert!(Panes::start(TerminalBackend::Daemon, 4)
            .0
            .we_are_the_terminal());
        // tmux's arm is asserted in tmux's own tests, which have a tmux to
        // talk to; this crate's tests must not depend on one being installed.
    }
}
