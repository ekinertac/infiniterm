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

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

pub mod local_pty;
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
}

impl Panes {
    /// tmux when it is asked for AND there is a tmux to talk to. A missing
    /// binary must not mean no terminal, so it falls back and says so.
    pub fn start(want_tmux: bool) -> (Panes, Receiver<(PaneId, PaneEvent)>, Option<String>) {
        if want_tmux {
            if !tmux::available() {
                let (pty, rx) = local_pty::LocalPtyBackend::new();
                return (
                    Panes::Local(pty),
                    rx,
                    Some("tmux is not installed; cards use local shells".into()),
                );
            }
            match tmux::TmuxBackend::start() {
                Some((backend, rx)) => return (Panes::Tmux(backend), rx, None),
                None => {
                    let (pty, rx) = local_pty::LocalPtyBackend::new();
                    return (
                        Panes::Local(pty),
                        rx,
                        Some("tmux would not start; cards use local shells".into()),
                    );
                }
            }
        }
        let (pty, rx) = local_pty::LocalPtyBackend::new();
        (Panes::Local(pty), rx, None)
    }

    pub fn is_tmux(&self) -> bool {
        matches!(self, Panes::Tmux(_))
    }

    /// The tmux window a pane is in, for the save file. Local shells have
    /// none: there is nothing to come back to.
    pub fn window_id(&self, pane: PaneId) -> Option<String> {
        match self {
            Panes::Local(_) => None,
            Panes::Tmux(b) => b.window_id(pane),
        }
    }

    /// Takes over a window left running by an earlier launch. `None` when
    /// this is not tmux, and the caller spawns a fresh shell as it always
    /// did.
    pub fn adopt(&self, window: &str) -> Option<PaneId> {
        match self {
            Panes::Local(_) => None,
            Panes::Tmux(b) => Some(b.adopt(window)),
        }
    }

    /// Which windows are still there to adopt.
    pub fn live_windows(&self) -> Vec<String> {
        match self {
            Panes::Local(_) => vec![],
            Panes::Tmux(_) => tmux::TmuxBackend::live_windows(),
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
        }
    }

    pub fn write_now(&self, pane: PaneId, bytes: &[u8]) {
        match self {
            Panes::Local(b) => b.write_now(pane, bytes),
            Panes::Tmux(b) => b.write_now(pane, bytes),
        }
    }

    pub fn resize_now(&self, pane: PaneId, cols: u16, rows: u16) {
        match self {
            Panes::Local(b) => b.resize_now(pane, cols, rows),
            Panes::Tmux(b) => b.resize_now(pane, cols, rows),
        }
    }

    pub fn ack_now(&self, pane: PaneId, bytes: usize) {
        match self {
            Panes::Local(b) => b.ack_now(pane, bytes),
            Panes::Tmux(b) => b.ack_now(pane, bytes),
        }
    }

    pub fn kill(&self, pane: PaneId) {
        match self {
            Panes::Local(b) => b.kill(pane),
            Panes::Tmux(b) => b.kill_now(pane),
        }
    }

    pub fn write(&self, pane: PaneId, bytes: &[u8]) {
        self.write_now(pane, bytes)
    }

    /// Every pane ended. Under tmux this kills the session: it is what
    /// `app.reload` means, and a reload that left the windows would grow a
    /// second set beside them.
    pub fn kill_all(&self) {
        match self {
            Panes::Local(b) => b.kill_all(),
            Panes::Tmux(b) => b.kill_all(),
        }
    }

    /// The app is quitting. Local shells die with it; tmux windows are
    /// LEFT RUNNING, which is the entire point of the tmux backend.
    pub fn leave(&self) {
        match self {
            Panes::Local(b) => b.kill_all(),
            Panes::Tmux(b) => b.detach(),
        }
    }

    /// Live pane pids, for the remote-session poller. tmux answers nothing:
    /// a tmux pane's process lives wherever the server does, which the
    /// header of `pids_source` has always said.
    pub fn pids_source(&self) -> Box<dyn Fn() -> Vec<(PaneId, u32)> + Send + 'static> {
        match self {
            Panes::Local(b) => Box::new(b.pids_source()),
            Panes::Tmux(_) => Box::new(Vec::new),
        }
    }
}
