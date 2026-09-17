//! The falsification test for the session daemon, run headless.
//!
//! The plan's version drives the GUI and compares screenshots, which needs
//! an unlocked Mac and a person to look at them. This asks the same
//! question exactly instead of visually: does a pane's output, taken
//! through `iftd` and replayed from its ring, put our emulator in the same
//! state the local pty would have?
//!
//! Ekin established that Claude Code renders correctly under the local
//! backend and was corrupted under tmux, so the local backend IS the
//! ground truth here. What the daemon has to prove is that it changes
//! nothing: same bytes, same grid. tmux could not pass this, because
//! `capture-pane` returned a flattened copy of ITS grid padded with ITS
//! character widths (bug 7 in docs/tmux-handover.md) rather than the bytes
//! that drew it.
//!
//! The program under test redraws INLINE, the way Ink does and the way
//! none of the full-screen TUIs that survived tmux do: it prints rows,
//! moves the cursor back up over them, erases, and prints again. That is
//! the redraw tmux never got right and the one this design exists to
//! survive.
//!
//! Related: `infiniterm_core::backend::daemon`, `tools/drive/daemon.sh`
//! (the on-screen version, which still wants a human), and the spec at
//! docs/superpowers/specs/2026-09-17-session-daemon-design.md.

use infiniterm_core::backend::daemon::DaemonBackend;
use infiniterm_core::backend::local_pty::LocalPtyBackend;
use infiniterm_core::backend::{PaneEvent, PaneId};
use infiniterm_term::grid::Grid;
use infiniterm_term::palette::Palette;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// The size both backends run at. Fixed, because a difference here would
/// be a difference in the output and the comparison would mean nothing.
const COLS: u16 = 80;
const ROWS: u16 = 24;

/// Generous: this spawns real shells under real ptys, three times.
const DEADLINE: Duration = Duration::from_secs(10);

/// An Ink-shaped redraw: clear, draw three rows, move the cursor back up
/// over them, erase to the end of the screen, draw three DIFFERENT rows.
/// A terminal that loses a row leaves `alpha-1` on screen beside
/// `alpha-2`, which is the "two lines in one row" symptom exactly.
/// The sleep keeps the pane alive so it can be detached from.
const INLINE_REDRAW: &str = "printf '\\033[2J\\033[H'; \
     printf 'alpha-1\\nbeta-1\\ngamma-1\\n'; \
     printf '\\033[3A\\033[0J'; \
     printf 'alpha-2\\nbeta-2\\ngamma-2\\n'; \
     printf 'SENTINEL\\n'; \
     sleep 30";

fn temp_dir() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("iftd-eq-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a fresh temp dir");
    dir
}

/// `iftd` lives beside this test binary's target dir, not on `PATH`.
fn ensure_iftd_on_path() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    static FOUND: AtomicBool = AtomicBool::new(false);
    ONCE.call_once(|| {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let Some(profile_dir) = exe.parent().and_then(|d| d.parent()) else {
            return;
        };
        if !profile_dir.join("iftd").is_file() {
            return;
        }
        let existing = std::env::var_os("PATH").unwrap_or_default();
        let mut paths: Vec<PathBuf> = std::env::split_paths(&existing).collect();
        paths.insert(0, profile_dir.to_path_buf());
        if let Ok(joined) = std::env::join_paths(paths) {
            std::env::set_var("PATH", joined);
            FOUND.store(true, Ordering::SeqCst);
        }
    });
    assert!(
        FOUND.load(Ordering::SeqCst),
        "iftd not beside this test binary; run `cargo build -p infiniterm-session` first"
    );
}

/// The grid as text: trailing blanks stripped, empty rows dropped. What a
/// person would see, with nothing about cell padding in the comparison.
fn rows_of(grid: &mut Grid) -> Vec<String> {
    grid.frame(&Palette::default_palette())
        .rows
        .iter()
        .map(|r| r.text.trim_end().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Drains a backend's events into a grid until the sentinel appears in it.
/// A read boundary says nothing about whether the program finished
/// drawing, so the program says so itself.
///
/// `ack` is not optional. A pane stalls at `HIGH_WATER` (256 KiB) of
/// unacknowledged output, which is the whole backpressure design, so a
/// reader that never acks hangs on anything bigger than that. The app acks
/// every frame; a test that did not would only ever be testing small
/// output.
fn render_until_sentinel(
    rx: &Receiver<(PaneId, PaneEvent)>,
    want: PaneId,
    ack: impl Fn(usize),
) -> Grid {
    let mut grid = Grid::new(COLS as usize, ROWS as usize, 1000);
    let start = Instant::now();
    while start.elapsed() < DEADLINE {
        if let Ok((id, event)) = rx.recv_timeout(Duration::from_millis(100)) {
            if id != want {
                continue;
            }
            match event {
                PaneEvent::Output(bytes) | PaneEvent::Replay(bytes) => {
                    grid.advance(&bytes);
                    ack(bytes.len());
                }
                _ => continue,
            }
            if rows_of(&mut grid).iter().any(|r| r.contains("SENTINEL")) {
                return grid;
            }
        }
    }
    panic!(
        "no sentinel within {DEADLINE:?}; the grid held:\n{}",
        rows_of(&mut grid).join("\n")
    );
}

#[test]
fn the_daemon_puts_the_emulator_where_the_local_pty_would_have() {
    ensure_iftd_on_path();
    let dir = temp_dir();

    // Ground truth: the backend Claude Code already renders correctly in.
    let (local, local_rx) = LocalPtyBackend::new();
    let lpane = local
        .spawn_now(std::path::Path::new("/tmp"), Some(INLINE_REDRAW), vec![])
        .expect("a local shell");
    local.resize_now(lpane, COLS, ROWS);
    let mut local_grid = render_until_sentinel(&local_rx, lpane, |n| local.ack_now(lpane, n));
    let expected = rows_of(&mut local_grid);
    local.kill_all();

    // The inline redraw really happened. Without this the comparisons
    // below could pass on two identically broken grids.
    assert!(
        expected.iter().any(|r| r.contains("alpha-2")),
        "the second pass drew: {expected:?}"
    );
    assert!(
        !expected.iter().any(|r| r.contains("alpha-1")),
        "the second pass REPLACED the first rather than landing beside it: {expected:?}"
    );

    let (daemon, daemon_rx) = DaemonBackend::new(dir.clone(), 4);
    let dpane = daemon
        .spawn_now(std::path::Path::new("/tmp"), Some(INLINE_REDRAW), vec![])
        .expect("iftd starts");
    daemon.resize_now(dpane, COLS, ROWS);
    let mut daemon_grid = render_until_sentinel(&daemon_rx, dpane, |n| daemon.ack_now(dpane, n));
    assert_eq!(
        rows_of(&mut daemon_grid),
        expected,
        "live through iftd is not what the local pty produced"
    );

    // The part tmux could not do: quit, come back, and rebuild the same
    // screen from the ring alone.
    let session = daemon.session_id(dpane).expect("a session id");
    daemon.detach();

    let (again, again_rx) = DaemonBackend::new(dir.clone(), 4);
    let apane = again.adopt(&session).expect("the session is still there");
    let mut replayed = render_until_sentinel(&again_rx, apane, |n| again.ack_now(apane, n));
    assert_eq!(
        rows_of(&mut replayed),
        expected,
        "the replayed ring did not rebuild the screen the pty drew"
    );

    again.kill_now(apane);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Enough filler to push a 1 MiB ring past its cap several times over.
/// A Claude card reaches this in ordinary use: Ink repaints are large and
/// it repaints constantly.
const OVERFLOW_THEN_REDRAW: &str = "yes 'filler line long enough to add up quickly' \
     | head -n 40000; \
     printf '\\033[2J\\033[H'; \
     printf 'alpha-2\\nbeta-2\\ngamma-2\\n'; \
     printf 'SENTINEL\\n'; \
     sleep 30";

/// The ring's stated imperfection is that output older than the cap is
/// gone and the cut may land mid-state. What must NOT happen is the
/// VISIBLE screen coming back wrong, and that is the case a long-running
/// Claude card is in every day: far more than a ring of scrollback, with
/// the part you are looking at drawn last.
///
/// What this does NOT prove, measured rather than assumed: moving the
/// ring's cut into the middle of a line leaves this test passing. The
/// trim-to-a-newline only ever affects the FIRST replayed line, which by
/// then is far above the screen, and a program that clears before drawing
/// wipes even that. `Ring`'s own unit tests are where that behaviour is
/// pinned; this is not a second copy of them.
#[test]
fn a_ring_that_wrapped_still_rebuilds_the_visible_screen() {
    ensure_iftd_on_path();
    let dir = temp_dir();

    // One MiB, the smallest the daemon allows, so the filler overflows it
    // many times over rather than needing 4 MiB of output to prove it.
    let (daemon, daemon_rx) = DaemonBackend::new(dir.clone(), 1);
    let pane = daemon
        .spawn_now(
            std::path::Path::new("/tmp"),
            Some(OVERFLOW_THEN_REDRAW),
            vec![],
        )
        .expect("iftd starts");
    daemon.resize_now(pane, COLS, ROWS);
    let mut live = render_until_sentinel(&daemon_rx, pane, |n| daemon.ack_now(pane, n));
    let expected = rows_of(&mut live);
    assert!(
        expected.iter().any(|r| r.contains("SENTINEL")),
        "the final screen drew: {expected:?}"
    );

    let session = daemon.session_id(pane).expect("a session id");
    daemon.detach();

    let (again, again_rx) = DaemonBackend::new(dir.clone(), 1);
    let apane = again.adopt(&session).expect("still there");
    let mut replayed = render_until_sentinel(&again_rx, apane, |n| again.ack_now(apane, n));
    assert_eq!(
        rows_of(&mut replayed),
        expected,
        "a wrapped ring rebuilt the wrong screen"
    );

    again.kill_now(apane);
    let _ = std::fs::remove_dir_all(&dir);
}
