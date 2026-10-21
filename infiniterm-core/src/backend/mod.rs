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
// Only the daemon arm of `Panes::start` needs it, and that arm is unix-only.
#[cfg(unix)]
use crate::paths;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

// The daemon is unix-only: `iftd` is fork/setsid and a unix socket, and
// there is nothing on Windows to fork into. tmux compiles everywhere (it
// only shells out) but there is no tmux to talk to on Windows, so both are
// refused there and `Panes::start` falls back to local shells. See
// docs/windows-handoff.md, "No daemon", for why iftd is not being ported.
#[cfg(unix)]
pub mod daemon;
// Join (`ift join`), the relay behind it, remote instances and the ssh login
// step all ride on unix sockets and ssh; none of them has a Windows
// counterpart yet.
#[cfg(unix)]
pub mod join;
#[cfg(unix)]
pub mod join_client;
pub mod local_pty;
#[cfg(unix)]
pub mod relay;
#[cfg(unix)]
pub mod remote;
#[cfg(unix)]
pub mod session_protocol;
#[cfg(unix)]
pub mod ssh_login;
#[cfg(unix)]
pub mod tmux;
#[cfg(unix)]
pub mod tmux_protocol;

pub type PaneId = u32;

/// What `Panes::redraw` did (`terminal.redraw`, #447).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Redraw {
    /// The program was sent the resize jog and repaints itself.
    Asked,
    /// A viewer's pane: its connection was let go, and the caller must join
    /// the session again, whose replay and host-side jog do the repaint.
    Rejoin,
}

/// The resize jog for a backend that does not know its pane's size: one
/// column narrower, then back, so the program gets `SIGWINCH` twice and
/// repaints. A pane is never 1 wide in practice; the bump goes up there
/// rather than ask for 0 columns.
fn jog(mut resize: impl FnMut(u16, u16), cols: u16, rows: u16) {
    if cols == 0 || rows == 0 {
        return;
    }
    let bumped = if cols > 1 { cols - 1 } else { cols + 1 };
    resize(bumped, rows);
    resize(cols, rows);
}

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
    #[cfg(unix)]
    Tmux(tmux::TmuxBackend),
    #[cfg(unix)]
    Daemon(daemon::DaemonBackend),
}

// On Windows only the `Local` arm survives, so the session handles every
// other arm reads (pane ids, session ids, the orphan list) go unread here.
#[cfg_attr(windows, allow(unused_variables))]
/// Whether quitting DETACHES a card's shell instead of ending it.
///
/// Not the same question as which variant the setting names, which is why
/// this exists rather than a comparison at the call site. On Windows
/// `Panes::start` falls back to local shells for both Daemon and Tmux
/// (neither exists there), so a canvas whose settings ask for the daemon
/// still has shells that die with the app.
///
/// `app.restart` is the caller. It offers a restart only when the shells
/// will survive it, and comparing the enum meant a Windows machine with
/// `"terminal.backend": "daemon"` in its settings was offered a restart
/// that killed every shell, the exact thing the guard exists to prevent.
///
/// Tmux detaches too, and is still excluded: that is the rule `app.restart`
/// already had, and widening it is a macOS decision rather than a Windows
/// one.
pub fn detaches_on_quit(backend: TerminalBackend) -> bool {
    !cfg!(windows) && matches!(backend, TerminalBackend::Daemon)
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
            // A viewer window (`ift join`, #423): the cards are the host
            // app's, reached through it, never through its `iftd`. No
            // forward: the host's hooks have nothing to say to this window.
            #[cfg(unix)]
            _ if remote::join_from_env().is_some() => {
                let host = remote::join_from_env().expect("checked by the guard");
                let (backend, rx) = daemon::DaemonBackend::new_viewer(host, buffer_mib);
                (Panes::Daemon(backend), rx, None)
            }
            // A remote instance (`ift connect`) runs its cards on the host
            // `INFINITERM_REMOTE` names, whatever the setting says, and needs no
            // `iftd` on THIS Mac: the daemons are on the server.
            #[cfg(unix)]
            _ if remote::from_env().is_some() => {
                let host = remote::from_env().expect("checked by the guard");
                // Hooks and `ift` on the server reach this instance through it.
                remote::forward(
                    host.clone(),
                    crate::paths::socket_path().to_string_lossy().into_owned(),
                );
                let (backend, rx) = daemon::DaemonBackend::new_remote(host, buffer_mib);
                (Panes::Daemon(backend), rx, None)
            }
            TerminalBackend::Pty => {
                let (pty, rx) = local_pty::LocalPtyBackend::new();
                (Panes::Local(pty), rx, None)
            }
            #[cfg(unix)]
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
            #[cfg(unix)]
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
            // Neither exists here: iftd is not built for Windows and tmux
            // does not run on it. A setting carried over from a Mac canvas
            // must still give the user a terminal, so say so and go local.
            #[cfg(windows)]
            TerminalBackend::Tmux | TerminalBackend::Daemon => {
                let (pty, rx) = local_pty::LocalPtyBackend::new();
                (
                    Panes::Local(pty),
                    rx,
                    Some("that backend is macOS only; cards use local shells".into()),
                )
            }
        }
    }

    /// Whether OUR emulator is the thing programs are talking to. False only
    /// under tmux, which is a terminal in its own right and answers colour and
    /// device queries before we can; our second answer then reaches the program
    /// as keystrokes. See `TerminalBody::replies` and tmux bug 9.
    pub fn we_are_the_terminal(&self) -> bool {
        match self {
            Panes::Local(_) => true,
            #[cfg(unix)]
            Panes::Daemon(_) => true,
            #[cfg(unix)]
            Panes::Tmux(_) => false,
        }
    }

    /// The host's columns and rows for a viewer's pane (#423); `None` for any
    /// other pane, whose size is its card's.
    pub fn viewer_pane_size(&self, pane: PaneId) -> Option<(u16, u16)> {
        match self {
            #[cfg(unix)]
            Panes::Daemon(b) => b.viewer_pane_size(pane),
            _ => {
                let _ = pane;
                None
            }
        }
    }

    /// Asks the program in `pane` to repaint (`terminal.redraw`, #447). A
    /// daemon pane gets the two-resize jog an attach sends (`cols` and `rows`
    /// are its card's size, which the other backends need to jog by); a
    /// viewer's pane cannot, because the host ignores a viewer's resizes, so
    /// its connection is let go and the caller joins the session again:
    /// that join replays the ring and the host jogs the program itself.
    pub fn redraw(&self, pane: PaneId, cols: u16, rows: u16) -> Redraw {
        match self {
            #[cfg(unix)]
            Panes::Daemon(b) => b.redraw_now(pane),
            Panes::Local(b) => {
                jog(|c, r| b.resize_now(pane, c, r), cols, rows);
                Redraw::Asked
            }
            #[cfg(unix)]
            Panes::Tmux(b) => {
                jog(|c, r| b.resize_now(pane, c, r), cols, rows);
                Redraw::Asked
            }
        }
    }

    /// A viewer card's session joined again on a thread of its own (#442);
    /// the answer comes from `take_reconnected`. Nothing for any other pane.
    #[cfg(unix)]
    pub fn begin_reconnect(&self, session: &str) {
        if let Panes::Daemon(b) = self {
            b.begin_reconnect(session);
        }
    }

    /// The reconnects that ended: a new pane, or why not (#442).
    #[cfg(unix)]
    pub fn take_reconnected(&self) -> Vec<(String, Result<PaneId, remote::JoinFailure>)> {
        match self {
            Panes::Daemon(b) => b.take_reconnected(),
            _ => Vec::new(),
        }
    }

    /// What the join server (#423) needs from this backend: the relay and a
    /// way to write a viewer's keys to a session. Only the daemon backend has
    /// them; under local and tmux a card has no session a viewer could join.
    #[cfg(unix)]
    pub fn join_parts(
        &self,
    ) -> Option<(
        std::sync::Arc<std::sync::Mutex<relay::Relay>>,
        join::WriteSession,
        join::NudgeSession,
    )> {
        match self {
            Panes::Daemon(b) => Some((b.relay(), b.session_writer(), b.session_nudger())),
            _ => None,
        }
    }

    /// This card's shell, as an opaque handle for the save file: a tmux
    /// window id (`@7`) or a daemon session id. Local shells have none:
    /// there is nothing to come back to.
    pub fn session_id(&self, pane: PaneId) -> Option<String> {
        match self {
            Panes::Local(_) => None,
            #[cfg(unix)]
            Panes::Tmux(b) => b.window_id(pane),
            #[cfg(unix)]
            Panes::Daemon(b) => b.session_id(pane),
        }
    }

    /// Takes over a session left running by an earlier launch. `None` when
    /// this is the local backend, or the session has gone; the caller
    /// spawns a fresh shell as it always did.
    pub fn adopt(&self, session: &str) -> Option<PaneId> {
        match self {
            Panes::Local(_) => None,
            #[cfg(unix)]
            Panes::Tmux(b) => Some(b.adopt(session)),
            #[cfg(unix)]
            Panes::Daemon(b) => b.adopt(session),
        }
    }

    /// Sessions this app made that no card claims: what a crash left
    /// behind. Does nothing under the local backend, which has no litter
    /// to leave.
    pub fn kill_orphans(&self, claimed: &[String]) {
        match self {
            Panes::Local(_) => {}
            #[cfg(unix)]
            Panes::Tmux(b) => b.kill_orphans(claimed),
            #[cfg(unix)]
            Panes::Daemon(b) => b.kill_orphans(claimed),
        }
    }

    /// Which sessions are still there to adopt.
    /// Whether a daemon session has a client on it right now, from its
    /// meta file; `None` when there is no such session or no daemon backend.
    pub fn session_attached(&self, session_id: &str) -> Option<bool> {
        match self {
            #[cfg(unix)]
            Panes::Daemon(b) => b.session_attached(session_id),
            Panes::Local(_) => None,
            #[cfg(unix)]
            Panes::Tmux(_) => None,
        }
    }

    /// A dead session's scrollback from disk, daemon backend only: the
    /// local pty has no daemon to have written one and tmux keeps its own.
    pub fn take_ring(&self, session_id: &str) -> Option<(Vec<u8>, String)> {
        match self {
            #[cfg(unix)]
            Panes::Daemon(b) => b.take_ring(session_id),
            Panes::Local(_) => None,
            #[cfg(unix)]
            Panes::Tmux(_) => None,
        }
    }

    pub fn live_sessions(&self) -> Vec<String> {
        match self {
            Panes::Local(_) => vec![],
            #[cfg(unix)]
            Panes::Tmux(_) => tmux::TmuxBackend::live_windows(),
            #[cfg(unix)]
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
            #[cfg(unix)]
            Panes::Tmux(b) => b.spawn_now(cwd, cmd, env),
            #[cfg(unix)]
            Panes::Daemon(b) => b.spawn_now(cwd, cmd, env),
        }
    }

    pub fn write_now(&self, pane: PaneId, bytes: &[u8]) {
        match self {
            Panes::Local(b) => b.write_now(pane, bytes),
            #[cfg(unix)]
            Panes::Tmux(b) => b.write_now(pane, bytes),
            #[cfg(unix)]
            Panes::Daemon(b) => b.write_now(pane, bytes),
        }
    }

    pub fn resize_now(&self, pane: PaneId, cols: u16, rows: u16) {
        match self {
            Panes::Local(b) => b.resize_now(pane, cols, rows),
            #[cfg(unix)]
            Panes::Tmux(b) => b.resize_now(pane, cols, rows),
            #[cfg(unix)]
            Panes::Daemon(b) => b.resize_now(pane, cols, rows),
        }
    }

    pub fn ack_now(&self, pane: PaneId, bytes: usize) {
        match self {
            Panes::Local(b) => b.ack_now(pane, bytes),
            #[cfg(unix)]
            Panes::Tmux(b) => b.ack_now(pane, bytes),
            #[cfg(unix)]
            Panes::Daemon(b) => b.ack_now(pane, bytes),
        }
    }

    pub fn kill(&self, pane: PaneId) {
        match self {
            Panes::Local(b) => b.kill(pane),
            #[cfg(unix)]
            Panes::Tmux(b) => b.kill_now(pane),
            #[cfg(unix)]
            Panes::Daemon(b) => b.kill_now(pane),
        }
    }

    /// Only the daemon can hand a closed card's running program back to a
    /// reopened one (`Model::can_park`): the local backend has no session
    /// to adopt, and tmux was left as it was.
    pub fn can_park(&self) -> bool {
        #[cfg(unix)]
        {
            matches!(self, Panes::Daemon(_))
        }
        #[cfg(windows)]
        {
            false
        }
    }

    /// Lets go of a parked card's pane so its reopened card can adopt the
    /// session again. True when the session is left running for that.
    pub fn release(&self, pane: PaneId) -> bool {
        match self {
            #[cfg(unix)]
            Panes::Daemon(b) => {
                b.detach_now(pane);
                true
            }
            _ => {
                let _ = pane;
                false
            }
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
            #[cfg(unix)]
            Panes::Tmux(b) => b.kill_all(),
            #[cfg(unix)]
            Panes::Daemon(b) => b.kill_all(),
        }
    }

    /// The app is quitting. Local shells die with it; tmux windows and
    /// daemon sessions are LEFT RUNNING, which is the entire point of
    /// either backend.
    pub fn leave(&self) {
        match self {
            Panes::Local(b) => b.kill_all(),
            #[cfg(unix)]
            Panes::Tmux(b) => b.detach(),
            #[cfg(unix)]
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
            #[cfg(unix)]
            Panes::Tmux(_) => Box::new(Vec::new),
            #[cfg(unix)]
            Panes::Daemon(b) => Box::new(b.pids_source()),
        }
    }
}

#[cfg(test)]
mod panes_tests {

    #[test]
    fn only_a_backend_that_really_detaches_allows_a_restart() {
        // The daemon is the one that detaches, and only where it exists.
        assert_eq!(
            detaches_on_quit(TerminalBackend::Daemon),
            cfg!(not(windows))
        );
        // Local shells never survive, anywhere.
        assert!(!detaches_on_quit(TerminalBackend::Pty));
        // Tmux detaches but is not what `app.restart` allows; see the doc.
        assert!(!detaches_on_quit(TerminalBackend::Tmux));
    }
    use super::*;

    // The jog for a backend that does not know its pane's size: one column
    // narrower, then the real size, so the program gets two `SIGWINCH`.
    #[test]
    fn the_redraw_jog_is_one_column_narrower_and_back() {
        let mut seen = vec![];
        jog(|c, r| seen.push((c, r)), 80, 24);
        assert_eq!(seen, vec![(79, 24), (80, 24)]);
        // A one-column pane goes up instead of asking for none, and a card
        // with no size yet (0) is left alone.
        let mut seen = vec![];
        jog(|c, r| seen.push((c, r)), 1, 5);
        assert_eq!(seen, vec![(2, 5), (1, 5)]);
        let mut seen = vec![];
        jog(|c, r| seen.push((c, r)), 0, 0);
        assert!(seen.is_empty());
    }

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
        // On Windows that call lands on `Panes::Local` through the refusal
        // arm above, which is the same answer for the same reason.
        // tmux's arm is asserted in tmux's own tests, which have a tmux to
        // talk to; this crate's tests must not depend on one being installed.
    }
}
