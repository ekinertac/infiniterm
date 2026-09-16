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
