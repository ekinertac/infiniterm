//! Cards backed by tmux, so the shells outlive the window.
//!
//! One `tmux -C` client on the DEFAULT socket, attached to one session named
//! `infiniterm`. That is the whole argument for tmux over a daemon of our
//! own: `tmux attach -t infiniterm` from any terminal reaches the same
//! sessions, and the rest of somebody's tooling sees them too.
//!
//! A card is a tmux WINDOW holding one pane, never a pane inside a shared
//! window: panes tile inside a window and would have to share its size,
//! while `refresh-client -C '@0:100x30'` gives every window its own. Proved
//! in spikes/tmux/NOTES.md against tmux 3.7c.
//!
//! tmux draws nothing here. It reports bytes, our emulator renders them, and
//! scrollback, selection and the mouse stay exactly what they already are.
//!
//! ADDRESS BY ID, never by index or name: `%0` a pane, `@0` a window. An
//! index is somebody's `base-index` and a name is their `automatic-rename`.
//! An afternoon went into learning that; see the spike notes.
//!
//! Related: tmux_protocol.rs (the reading), local_pty.rs (the same surface
//! without the persistence), backend/mod.rs for `SessionBackend`.
use super::{PaneEvent, PaneId, SessionBackend};
use crate::backend::tmux_protocol::{Notice, Reader};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// The session every infiniterm window lives in. One name, so a second
/// launch attaches rather than growing a second session.
pub const SESSION: &str = "infiniterm";
/// What a scratch instance uses instead. A driver run or a `make run` on a
/// copied canvas must not attach to, resize or kill the windows the real app
/// is holding: the same reason the driver addresses the app by pid and the
/// scenarios scope `ift` to their own data dir.
pub const DEV_SESSION: &str = "infiniterm-dev";
/// A tmux user option set on every window this app makes. It is how our own
/// litter is told from somebody's window: a window made by hand with
/// `tmux neww -t infiniterm` carries no tag and is never ours to kill.
pub const TAG: &str = "@infiniterm";

/// The session this instance may touch.
pub fn session_name() -> &'static str {
    if crate::paths::data_dir_overridden() {
        DEV_SESSION
    } else {
        SESSION
    }
}

/// What tmux is asked for when a card has no size yet. The real size follows
/// within a frame, but a shell that starts at 80x24 and is corrected is
/// better than one that starts at 0.
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;

/// A card's window and pane, as tmux names them.
#[derive(Clone, Debug)]
struct Window {
    /// `@7`
    id: String,
    /// The size the ui last asked for. Remembered because the ui asks
    /// before tmux has said what the window is called, and that first ask
    /// is the one that matters: a card whose size never lands runs at
    /// 80x24 while showing a much bigger grid, and a full-screen program
    /// draws into the corner of it.
    size: Option<(u16, u16)>,
    /// `%7`. Learned from the first `%output` for the window, because tmux
    /// reports output by PANE and commands take either.
    pane: Option<String>,
}

pub struct TmuxBackend {
    /// Our pane ids to tmux's window. The model never sees a tmux id.
    windows: Arc<Mutex<HashMap<PaneId, Window>>>,
    stdin: Arc<Mutex<ChildStdin>>,
    child: Arc<Mutex<Child>>,
    next_id: AtomicU32,
    /// What each reply block is an answer to, oldest first. tmux answers
    /// commands in order and every command gets a `%begin`/`%end`, so a
    /// queue with one entry per command sent stays in step. Every command
    /// pushes, even the ones whose answer is nothing, or the queue drifts
    /// and a capture would be handed to the wrong card.
    expecting: Arc<Mutex<VecDeque<Expect>>>,
    /// The window tmux made when it created the session, which belongs to no
    /// card. The first card kills it once it has one of its own: killing it
    /// earlier would take the session with it, since a session with no
    /// windows does not exist.
    spare: Arc<Mutex<Option<String>>>,
    /// Bytes sent for each pane that the ui has not acknowledged yet, and
    /// which panes are paused because of it. The local backend stops READING
    /// a pane past the same mark, which stalls the child at the kernel's pty
    /// buffer; tmux is asked to stop sending instead, which stops it reading
    /// too once no client wants the pane.
    ///
    /// Without this a single flooding card would be a regression against the
    /// local backend: everything shares one socket here, so one screaming
    /// pane delays every other card.
    in_flight: Arc<Mutex<HashMap<PaneId, usize>>>,
    paused: Arc<Mutex<HashMap<PaneId, bool>>>,
}

/// Back under this and the pane is let go again. Half the mark, so a pane
/// hovering at the limit is not paused and continued on every frame.
const LOW_WATER: usize = super::local_pty::HIGH_WATER / 2;

/// What the next reply block will contain.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Expect {
    /// `#{window_id} #{pane_id}` for one card: `@3 %3`.
    ///
    /// BOTH, in one answer. Learning the pane from whichever `%output`
    /// arrived next let a card claim the session's own initial window, and
    /// everything typed into that card went somewhere no card owned.
    Ids(PaneId),
    /// The window tmux made when it created the session, which is nobody's.
    Spare,
    /// A `capture-pane -p -e -S -`: the pane's history, as many lines.
    History(PaneId),
    /// Anything else. tmux still frames it and the frame still has to be
    /// consumed.
    Nothing,
}

/// Is there a tmux to talk to at all? The backend falls back to local PTYs
/// when there is not: a missing binary must not mean no terminal.
pub fn available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// One command out, and what its reply block will hold.
///
/// The queue push happens while the stdin lock is HELD. Both the ui thread
/// and the reader send commands, and if one could push between the other's
/// push and write, the queue would no longer be in the order tmux answers
/// in, which is the only thing tying a reply to the command that asked.
fn send(
    stdin: &Arc<Mutex<ChildStdin>>,
    expecting: &Arc<Mutex<VecDeque<Expect>>>,
    line: &str,
    expect: Expect,
) {
    let Ok(mut out) = stdin.lock() else {
        return;
    };
    expecting.lock().unwrap().push_back(expect);
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// `@3 %3` as tmux formats it, into the pair.
fn split_ids(line: &str) -> Option<(String, String)> {
    let (window, pane) = line.trim().split_once(' ')?;
    if !is_window_id(window) || !pane.starts_with('%') || pane.len() < 2 {
        return None;
    }
    Some((window.to_string(), pane.to_string()))
}

/// `@7` and nothing else: a reply line that is a window id.
fn is_window_id(line: &str) -> bool {
    line.strip_prefix('@')
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

impl TmuxBackend {
    /// Attaches to the session, creating it if it is not there (`-A`).
    /// `None` when tmux will not start, and the caller uses local PTYs.
    pub fn start() -> Option<(TmuxBackend, Receiver<(PaneId, PaneEvent)>)> {
        // Asked BEFORE attaching: `new-session -A` either attaches to a
        // session full of our windows or creates one with a window that is
        // nobody's, and afterwards the two look identical.
        let existed = Self::session_exists();
        let mut child = Command::new("tmux")
            .args([
                // A test points this at a socket of its own; the app leaves
                // it unset and uses the default, which is the whole argument
                // for tmux: `tmux attach -t infiniterm` from any terminal.
                "-L",
                &std::env::var("INFINITERM_TMUX_SOCKET").unwrap_or_else(|_| "default".into()),
                "-C",
                "new-session",
                "-A",
                "-s",
                session_name(),
                "-x",
                &INITIAL_COLS.to_string(),
                "-y",
                &INITIAL_ROWS.to_string(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        let (tx, rx) = channel();
        let backend = TmuxBackend {
            windows: Arc::new(Mutex::new(HashMap::new())),
            stdin: Arc::new(Mutex::new(stdin)),
            child: Arc::new(Mutex::new(child)),
            next_id: AtomicU32::new(1),
            expecting: Arc::new(Mutex::new(VecDeque::new())),
            spare: Arc::new(Mutex::new(None)),
            in_flight: Arc::new(Mutex::new(HashMap::new())),
            paused: Arc::new(Mutex::new(HashMap::new())),
        };
        // tmux answers the `new-session` on its own command line with a
        // reply block of its own, before anything we send. Nothing queued
        // it, so without this seat every answer after it is off by one and
        // a window id lands in the block of some earlier command.
        backend.expecting.lock().unwrap().push_back(Expect::Nothing);
        // The status line is a row of the card, not decoration: without this
        // every pane is one row shorter than the card it fills.
        backend.command("set -g status off");
        // Ours, and only ours: a window somebody made from another terminal
        // is theirs and we do not adopt it.
        backend.command("set -g allow-rename off");
        // A pane border is a row of the card. Somebody's `pane-border-status
        // top` left every shell one row shorter than the grid drawn for it,
        // so a full-screen program's last line landed in the wrong place.
        backend.command("set -g pane-border-status off");
        backend.read_thread(stdout, tx);
        if !existed {
            backend.command_expecting(
                &format!("list-windows -t {} -F '#{{window_id}}'", session_name()),
                Expect::Spare,
            );
        }
        Some((backend, rx))
    }

    /// The reader: one thread, one line at a time, turning notices into pane
    /// events on the same channel the local backend uses.
    fn read_thread(&self, stdout: std::process::ChildStdout, tx: Sender<(PaneId, PaneEvent)>) {
        let windows = self.windows.clone();
        let expecting = self.expecting.clone();
        let spare = self.spare.clone();
        let in_flight = self.in_flight.clone();
        let paused = self.paused.clone();
        let stdin = self.stdin.clone();
        std::thread::spawn(move || {
            let mut reader = Reader::new();
            let mut block = Expect::Nothing;
            let mut history: Vec<String> = vec![];
            let mut lines = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                // Bytes, not chars: a pane's output is arbitrary and a lone
                // invalid byte must not end the session.
                let mut raw = Vec::new();
                match lines.read_until(b'\n', &mut raw) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                while raw.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
                    raw.pop();
                }
                let text = String::from_utf8_lossy(&raw);
                match reader.line(&text) {
                    Notice::Output { pane, bytes } => {
                        let Some(id) = Self::id_for_pane(&windows, &pane) else {
                            continue;
                        };
                        let owed = {
                            let mut map = in_flight.lock().unwrap();
                            let owed = map.entry(id).or_insert(0);
                            *owed += bytes.len();
                            *owed
                        };
                        let _ = tx.send((id, PaneEvent::Output(bytes)));
                        // Past the mark, ask tmux to stop sending. `off`,
                        // never `pause`: a PAUSED pane keeps running and
                        // tmux DISCARDS what it produces, so the bytes are
                        // gone for good and our grid quietly stops matching
                        // the program's. Measured: output made while paused
                        // arrives neither during nor after `continue`, while
                        // tmux's own grid has it.
                        //
                        // `off` makes tmux stop READING the pane once no
                        // client wants it, so the program blocks instead of
                        // producing output nobody receives, and everything
                        // arrives when it is turned back on. That is the
                        // same mechanism the local backend uses: a reader
                        // that stops reading stalls the child at the
                        // kernel's pty buffer.
                        if owed > super::local_pty::HIGH_WATER
                            && !paused.lock().unwrap().get(&id).copied().unwrap_or(false)
                        {
                            paused.lock().unwrap().insert(id, true);
                            send(
                                &stdin,
                                &expecting,
                                &format!("refresh-client -A '{pane}:off'"),
                                Expect::Nothing,
                            );
                        }
                    }
                    Notice::WindowClose(window) => {
                        let gone: Vec<PaneId> = windows
                            .lock()
                            .unwrap()
                            .iter()
                            .filter(|(_, w)| w.id == window)
                            .map(|(id, _)| *id)
                            .collect();
                        for id in gone {
                            windows.lock().unwrap().remove(&id);
                            let _ = tx.send((id, PaneEvent::Exited { code: 0 }));
                        }
                    }
                    Notice::Begin(_) => {
                        block = expecting
                            .lock()
                            .unwrap()
                            .pop_front()
                            .unwrap_or(Expect::Nothing);
                        history.clear();
                    }
                    Notice::Reply(text) => match &block {
                        Expect::Ids(id) => {
                            if let Some((window, pane)) = split_ids(&text) {
                                if let Some(w) = windows.lock().unwrap().get_mut(id) {
                                    w.id = window.clone();
                                    w.pane = Some(pane);
                                }
                                // The size the ui asked for before tmux had
                                // named this window. Without this the card
                                // keeps the 80x24 it was created with.
                                let size = windows.lock().unwrap().get(id).and_then(|w| w.size);
                                if let Some((cols, rows)) = size {
                                    send(
                                        &stdin,
                                        &expecting,
                                        &format!("refresh-client -C '{window}:{cols}x{rows}'"),
                                        Expect::Nothing,
                                    );
                                }
                                // Tagged BY ID, here, because this is the
                                // first moment the id exists. `-t <session>`
                                // targets the session's CURRENT window, and
                                // `new-window -d` does not change that, so
                                // tagging at send time marked the wrong
                                // window every time.
                                send(
                                    &stdin,
                                    &expecting,
                                    &format!("set-option -w -t {window} {TAG} 1"),
                                    Expect::Nothing,
                                );
                            }
                        }
                        Expect::Spare if is_window_id(&text) => {
                            *spare.lock().unwrap() = Some(text);
                        }
                        Expect::History(_) => history.push(text),
                        _ => {}
                    },
                    Notice::End { .. } => {
                        // The scrollback a restored card missed, fed to its
                        // emulator as one replay rather than as output: it
                        // is not new, and a card must not look like it just
                        // printed a day's work.
                        if let Expect::History(id) = block {
                            // Trailing padding is stripped, and it is not
                            // cosmetic. `capture-pane` flattens tmux's grid
                            // and pads every line to the pane's width using
                            // TMUX's idea of how wide each character is. A
                            // line with an emoji in it that our emulator
                            // measures one cell differently then overflows
                            // the width, wraps, and every line after it
                            // lands a row out: the replayed scrollback came
                            // back shifted with characters interleaved.
                            //
                            // Nothing is lost. Trailing blanks on a captured
                            // line are padding, never content.
                            let text: Vec<&str> = history.iter().map(|l| l.trim_end()).collect();
                            if text.iter().any(|l| !l.is_empty()) {
                                let mut bytes = text.join("\r\n").into_bytes();
                                bytes.extend_from_slice(b"\r\n");
                                let _ = tx.send((id, PaneEvent::Replay(bytes)));
                            }
                        }
                        block = Expect::Nothing;
                    }
                    // tmux is going away: every card's shell went with it.
                    Notice::Exit(_) => {
                        let all: Vec<PaneId> = windows.lock().unwrap().keys().copied().collect();
                        for id in all {
                            let _ = tx.send((id, PaneEvent::Exited { code: 0 }));
                        }
                        break;
                    }
                    _ => {}
                }
            }
        });
    }

    /// Which card a pane belongs to, and ONLY if tmux told us so. Output
    /// from anything else is not ours: the session's own initial window and
    /// any window somebody made from another terminal both report output,
    /// and a card that claimed one would send its keystrokes there.
    fn id_for_pane(windows: &Arc<Mutex<HashMap<PaneId, Window>>>, pane: &str) -> Option<PaneId> {
        windows
            .lock()
            .unwrap()
            .iter()
            .find(|(_, w)| w.pane.as_deref() == Some(pane))
            .map(|(id, _)| *id)
    }

    /// One command, one line, and what its reply block will hold. tmux
    /// answers asynchronously and nothing here waits: the answers arrive as
    /// notices and are matched by the order they were asked in.
    fn command_expecting(&self, line: &str, expect: Expect) {
        send(&self.stdin, &self.expecting, line, expect);
    }

    fn command(&self, line: &str) {
        self.command_expecting(line, Expect::Nothing);
    }

    fn window_of(&self, pane: PaneId) -> Option<Window> {
        self.windows.lock().unwrap().get(&pane).cloned()
    }

    /// The tmux target for a pane: its pane id once known, else its window.
    fn target(&self, pane: PaneId) -> Option<String> {
        self.window_of(pane).map(|w| w.pane.unwrap_or(w.id))
    }

    /// Everything tmux holds for this app, ended. Not `kill-server`: the
    /// user's own sessions live on the same socket.
    pub fn kill_all(&self) {
        self.command(&format!("kill-session -t {}", session_name()));
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }

    /// Detaches without killing anything: the whole point. Called when the
    /// app quits, so the shells keep running.
    pub fn detach(&self) {
        self.command("detach-client");
        if let Ok(mut child) = self.child.lock() {
            let _ = child.wait();
        }
    }
}

impl TmuxBackend {
    /// The same as the trait's `spawn`, without the async: nothing here
    /// waits, and the ui calls it from a frame.
    pub fn spawn_now(
        &self,
        cwd: &Path,
        cmd: Option<&str>,
        env: Vec<(String, String)>,
    ) -> anyhow::Result<PaneId> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        // The environment reaches the pane through tmux's own, set for this
        // window only: a tmux window inherits the SERVER's environment, not
        // this client's, so INFINITERM_CARD_ID has to be handed over
        // explicitly or the hooks would report the wrong card.
        // ONE COMMAND PER LINE. A `;`-separated list is several commands to
        // tmux and produces several reply blocks, which would hand the
        // window id to the wrong block and desync every answer after it.
        // Measured, not assumed: three commands, three blocks.
        for (key, value) in super::local_pty::terminal_identity().into_iter().chain(env) {
            self.command(&format!(
                "set-environment -t {} {} {}",
                session_name(),
                key,
                crate::drop::shell_quote(&value)
            ));
        }
        let start = cmd
            .map(|c| format!(" {}", crate::drop::shell_quote(c)))
            .unwrap_or_default();
        self.command_expecting(
            &format!(
                "new-window -d -P -F '#{{window_id}} #{{pane_id}}' -c {}{}",
                crate::drop::shell_quote(&cwd.to_string_lossy()),
                start
            ),
            Expect::Ids(id),
        );
        // Now that the session has a window of ours, the one tmux made for
        // itself can go. Ordered after, or killing the last window would
        // end the session.
        if let Some(window) = self.spare.lock().unwrap().take() {
            self.command(&format!("kill-window -t {window}"));
        }
        // The window id arrives in the command's reply and the pane id in the
        // window's first output. Neither is waited on: a command issued
        // before they land targets nothing, which tmux ignores, and the
        // ui reissues size on the next frame anyway.
        self.windows.lock().unwrap().insert(
            id,
            Window {
                id: String::new(),
                pane: None,
                size: None,
            },
        );
        Ok(id)
    }

    pub fn write_now(&self, pane: PaneId, bytes: &[u8]) {
        let Some(target) = self.target(pane) else {
            return;
        };
        // send-keys -H takes hex, which is the only encoding that survives
        // arbitrary bytes: a paste can hold anything, including the
        // semicolon that would otherwise end the command.
        let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
        if hex.is_empty() {
            return;
        }
        self.command(&format!("send-keys -H -t {target} {}", hex.join(" ")));
    }

    pub fn resize_now(&self, pane: PaneId, cols: u16, rows: u16) {
        // Kept whether or not it can be sent yet: the ui asks the moment it
        // spawns, which is before tmux has named the window.
        let window = {
            let mut map = self.windows.lock().unwrap();
            let Some(w) = map.get_mut(&pane) else {
                return;
            };
            w.size = Some((cols, rows));
            w.clone()
        };
        if window.id.is_empty() {
            return;
        }
        // Per WINDOW, which is what lets two cards be two sizes.
        self.command(&format!(
            "refresh-client -C '{}:{}x{}'",
            window.id, cols, rows
        ));
    }

    pub fn kill_now(&self, pane: PaneId) {
        let Some(window) = self.window_of(pane) else {
            return;
        };
        self.windows.lock().unwrap().remove(&pane);
        if !window.id.is_empty() {
            self.command(&format!("kill-window -t {}", window.id));
        }
    }

    /// The ui has parsed this much: the pane owes less, and once it is well
    /// under the mark tmux is told to send again. `on`, the counterpart of
    /// the `off` that stopped it; nothing was lost in between.
    pub fn ack_now(&self, pane: PaneId, bytes: usize) {
        let owed = {
            let mut map = self.in_flight.lock().unwrap();
            let owed = map.entry(pane).or_insert(0);
            *owed = owed.saturating_sub(bytes);
            *owed
        };
        let is_paused = self
            .paused
            .lock()
            .unwrap()
            .get(&pane)
            .copied()
            .unwrap_or(false);
        if is_paused && owed < LOW_WATER {
            self.paused.lock().unwrap().insert(pane, false);
            if let Some(target) = self.target(pane) {
                self.command(&format!("refresh-client -A '{target}:on'"));
            }
        }
    }
}

impl TmuxBackend {
    /// The tmux window a pane ended up in, once tmux has said. The ui keeps
    /// it on the card so a later launch finds the same window again.
    pub fn window_id(&self, pane: PaneId) -> Option<String> {
        self.window_of(pane)
            .map(|w| w.id)
            .filter(|id| !id.is_empty())
    }

    /// Takes over a window this app left running rather than making a new
    /// one: the card comes back to the shell it had, with the scrollback it
    /// missed. This is the whole reason for the tmux backend.
    ///
    /// The window is trusted to exist because `live_windows` was asked
    /// first. One that died in between simply never reports output, which a
    /// card already handles.
    pub fn adopt(&self, window: &str) -> PaneId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.windows.lock().unwrap().insert(
            id,
            Window {
                id: window.to_string(),
                pane: None,
                size: None,
            },
        );
        // -e keeps the colours: without it a restored card comes back grey
        // and looks broken. -S - is the whole history.
        self.command_expecting(
            &format!("capture-pane -p -e -S - -t {window}"),
            Expect::History(id),
        );
        // Which pane is in it, asked rather than guessed.
        self.command_expecting(
            &format!("display-message -p -t {window} '#{{window_id}} #{{pane_id}}'"),
            Expect::Ids(id),
        );
        id
    }

    /// Whether our session is already there. One shot, before the control
    /// client exists.
    fn session_exists() -> bool {
        Command::new("tmux")
            .args([
                "-L",
                &std::env::var("INFINITERM_TMUX_SOCKET").unwrap_or_else(|_| "default".into()),
                "has-session",
                "-t",
                session_name(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// Windows this app made and no card claimed: the litter a crash leaves
    /// behind. Killed at startup so the session mirrors the canvas.
    ///
    /// ONLY tagged windows. One somebody opened by hand in the same session
    /// is theirs, and a canvas that failed to load claims nothing, which is
    /// why the caller checks that first: a corrupt save file must not take
    /// a day's work with it.
    pub fn kill_orphans(&self, claimed: &[String]) {
        for (window, tagged) in Self::windows_with_tag() {
            if tagged && !claimed.iter().any(|c| c == &window) {
                self.command(&format!("kill-window -t {window}"));
            }
        }
    }

    /// Every window in our session, and whether we made it.
    fn windows_with_tag() -> Vec<(String, bool)> {
        let out = Command::new("tmux")
            .args([
                "-L",
                &std::env::var("INFINITERM_TMUX_SOCKET").unwrap_or_else(|_| "default".into()),
                "list-windows",
                "-t",
                session_name(),
                "-F",
                &format!("#{{window_id}} #{{?{TAG},1,0}}"),
            ])
            .output();
        match out {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| {
                    let (window, tag) = line.trim().split_once(' ')?;
                    is_window_id(window).then(|| (window.to_string(), tag == "1"))
                })
                .collect(),
            _ => vec![],
        }
    }

    /// The windows this app's session still holds. Blocking and deliberately
    /// so: it runs once, before any card exists, and its answer decides
    /// whether each card is adopted or spawned.
    ///
    /// A one-shot `tmux` rather than the control client, whose answers are
    /// asynchronous; this one question has to be answered first.
    pub fn live_windows() -> Vec<String> {
        let out = Command::new("tmux")
            .args([
                "-L",
                &std::env::var("INFINITERM_TMUX_SOCKET").unwrap_or_else(|_| "default".into()),
                "list-windows",
                "-t",
                session_name(),
                "-F",
                "#{window_id}",
            ])
            .output();
        match out {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| is_window_id(l))
                .map(String::from)
                .collect(),
            // No session yet, or no tmux: nothing to adopt.
            _ => vec![],
        }
    }
}

impl SessionBackend for TmuxBackend {
    async fn spawn(
        &self,
        cwd: &Path,
        cmd: Option<&str>,
        env: Vec<(String, String)>,
    ) -> anyhow::Result<PaneId> {
        self.spawn_now(cwd, cmd, env)
    }

    fn write(&self, pane: PaneId, bytes: &[u8]) {
        self.write_now(pane, bytes)
    }

    fn resize(&self, pane: PaneId, cols: u16, rows: u16) {
        self.resize_now(pane, cols, rows)
    }

    fn kill(&self, pane: PaneId) {
        self.kill_now(pane)
    }

    fn ack(&self, pane: PaneId, bytes: usize) {
        self.ack_now(pane, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every command this backend sends addresses tmux by id. An index is
    // somebody's base-index and a name is their automatic-rename; the spike
    // lost an afternoon to `spike:0` silently matching nothing.
    // A scratch instance gets its own session, or a driver run would resize
    // and kill the windows the real app is holding.
    #[test]
    fn a_scratch_instance_never_touches_the_real_session() {
        assert_ne!(SESSION, DEV_SESSION);
        // Whichever this process is, it is one of the two and nothing else.
        assert!(matches!(session_name(), SESSION | DEV_SESSION));
    }

    #[test]
    fn the_session_is_named_and_the_targets_are_ids() {
        assert_eq!(SESSION, "infiniterm");
        let source = include_str!("tmux.rs");
        for line in source.lines() {
            let command = line.trim();
            if !command.starts_with("self.command(") && !command.contains("format!(") {
                continue;
            }
            assert!(
                !command.contains(":0\"") && !command.contains(":0 "),
                "a target by index crept in: {command}"
            );
        }
    }

    #[test]
    fn a_window_id_is_told_from_any_other_reply() {
        assert!(is_window_id("@0"));
        assert!(is_window_id("@17"));
        assert!(!is_window_id("@"));
        assert!(!is_window_id("@0 80x24"), "a list-windows row is not an id");
        assert!(!is_window_id("%0"), "that is a pane");
        assert!(!is_window_id("no such window"));
    }

    /// A tmux on a socket of its own, killed when the test ends. Nothing
    /// here can see, resize or kill a real session.
    struct Sandbox(String);

    impl Sandbox {
        fn new(name: &str) -> Option<Sandbox> {
            if !available() {
                return None;
            }
            let socket = format!("infiniterm-test-{name}");
            std::env::set_var("INFINITERM_TMUX_SOCKET", &socket);
            let _ = Command::new("tmux")
                .args(["-L", &socket, "kill-server"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            Some(Sandbox(socket))
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .args(["-L", &self.0, "kill-server"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }

    fn drain(rx: &Receiver<(PaneId, PaneEvent)>, seconds: f64) -> Vec<u8> {
        let end = std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds);
        let mut out = vec![];
        while std::time::Instant::now() < end {
            while let Ok((_, event)) = rx.try_recv() {
                if let PaneEvent::Output(bytes) = event {
                    out.extend(bytes);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        out
    }

    // The whole backend against a real tmux: a window is made, what is
    // written to it runs, and what it prints comes back as pane output on
    // the same channel a local pty would use.
    #[test]
    fn a_window_runs_what_is_written_to_it_and_reports_back() {
        let Some(_sandbox) = Sandbox::new("roundtrip") else {
            return; // no tmux here; the pure tests still cover the protocol
        };
        let Some((backend, rx)) = TmuxBackend::start() else {
            panic!("tmux is available but would not start");
        };
        let pane = futures_lite_block(backend.spawn(
            Path::new("/tmp"),
            None,
            vec![("INFINITERM_CARD_ID".into(), "test-card".into())],
        ))
        .expect("a window");
        // The shell has to be up before it can be typed at.
        drain(&rx, 1.5);
        backend.write(pane, b"echo the-roundtrip-worked\n");
        let seen = String::from_utf8_lossy(&drain(&rx, 2.5)).to_string();
        assert!(
            seen.contains("the-roundtrip-worked"),
            "the pane never echoed it back; saw {seen:?}"
        );
        backend.kill_all();
    }

    /// The trait is async only because tmux assigns ids asynchronously; this
    /// one never yields, so a two-line executor is enough for a test.
    fn futures_lite_block<T>(future: impl std::future::Future<Output = T>) -> T {
        let mut future = Box::pin(future);
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        loop {
            if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }
        }
    }

    // What `capture-pane` returns for a line is padded to the pane's width
    // with tmux's own character widths. Replaying that padding is what
    // shifted a restored card's scrollback: one emoji measured differently
    // and the line wraps where tmux did not wrap it.
    #[test]
    fn captured_padding_is_not_replayed() {
        let captured = [
            "  config overriding bundle.                    ",
            "[Sonnet 5] \u{1f4c1} ~/Code/melina                     ",
            "",
        ];
        let text: Vec<&str> = captured.iter().map(|l| l.trim_end()).collect();
        assert_eq!(text[0], "  config overriding bundle.");
        assert_eq!(text[1], "[Sonnet 5] \u{1f4c1} ~/Code/melina");
        // Leading space is content: an indent is part of the line.
        assert!(text[0].starts_with("  "));
        assert!(
            text.iter().any(|l| !l.is_empty()),
            "there is something to replay"
        );
    }

    #[test]
    fn bytes_go_out_as_hex_so_anything_survives() {
        // The encoding matters more than it looks: a semicolon in a paste
        // would otherwise end the tmux command and run the rest.
        let bytes = b"a;b\x1b\n";
        let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex.join(" "), "61 3b 62 1b 0a");
    }
}
