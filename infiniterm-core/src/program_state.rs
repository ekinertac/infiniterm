//! A card's state from any program, not only from an agent's hooks: the
//! escape sequences a card's output carries, read off the LIVE byte stream
//! (`Scanner`), and what each does to the card's `AgentState` (`Track`).
//!
//! Why: the colours came only from Claude's and Pi's hooks, so `cargo
//! build` or `python run.py` finishing said nothing. Two de facto standards
//! cover every other program, both read by iTerm2, WezTerm, Ghostty, kitty
//! and VS Code:
//! - OSC 133, shell integration: 133;C a command started, 133;D;<status> it
//!   finished. zsh sends it through our integration (shell_integration.rs).
//! - OSC 9;4, progress a program reports itself: 1 or 3 running, 2 error,
//!   4 paused, 0 cleared. And OSC 9 / 777 / 99 notifications, a program
//!   asking for you.
//!
//! The rules, and why:
//! - A command must run `LONG_MS` before it shows as working, and only a
//!   command that ran that long turns the card done (status 0) or waiting
//!   (anything else) when it ends, so every `ls` does not flash the card.
//!   A new command clears the last one's done or waiting.
//! - A full-screen program (vim, htop, less: the alternate screen) is
//!   something you are using, not something running for you, so it never
//!   colours the card.
//! - Hooks stay authoritative: once a hook arrives during a command, or on
//!   a card with an agent session, shell marks and notifications change
//!   nothing and the hooks' states stand.
//!
//! Called from model/hooks_in.rs (`apply_pane_event` feeds each card's
//! Output through its Scanner; a Replay is never fed, or old output would
//! relight cards) and model/mod.rs `tick` (the working promotion).
use crate::agent_state::AgentState;

/// How long a command must run before it colours its card, working while
/// it runs and done or waiting when it ends. Below it, a command is a
/// glance (`ls`, `git status`) and a flash of colour would be noise, the
/// tab dots' rule: a lamp always lit says nothing.
pub const LONG_MS: f64 = 5_000.;

/// An OSC longer than this is not one of ours (a clipboard write, an image)
/// and is not buffered.
const OSC_MAX: usize = 256;
/// A CSI longer than this is not a mode switch.
const CSI_MAX: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signal {
    /// OSC 133;C.
    CommandStart,
    /// OSC 133;D with its status when the shell sent one.
    CommandEnd(Option<i32>),
    /// OSC 9;4;<state>.
    Progress(u8),
    /// OSC 9;<text>, 777;notify;..., 99;...: a program asking for you.
    Notify,
    /// The alternate screen entered (true) or left.
    AltScreen(bool),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Scan {
    #[default]
    Ground,
    Esc,
    Osc,
    OscEsc,
    Csi,
}

/// Finds our sequences in a byte stream that arrives in arbitrary chunks,
/// so a sequence split across two reads is still one sequence.
#[derive(Clone, Debug, Default)]
pub struct Scanner {
    state: Scan,
    buf: Vec<u8>,
}

impl Scanner {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Signal> {
        let mut out = vec![];
        for &b in bytes {
            self.step(b, &mut out);
        }
        out
    }

    fn step(&mut self, b: u8, out: &mut Vec<Signal>) {
        const ESC: u8 = 0x1b;
        const BEL: u8 = 0x07;
        match self.state {
            Scan::Ground => {
                if b == ESC {
                    self.state = Scan::Esc;
                }
            }
            Scan::Esc => {
                self.buf.clear();
                self.state = match b {
                    b']' => Scan::Osc,
                    b'[' => Scan::Csi,
                    ESC => Scan::Esc,
                    _ => Scan::Ground,
                };
            }
            Scan::Osc => match b {
                BEL => {
                    out.extend(parse_osc(&self.buf));
                    self.state = Scan::Ground;
                }
                ESC => self.state = Scan::OscEsc,
                _ => {
                    if self.buf.len() < OSC_MAX {
                        self.buf.push(b);
                    }
                }
            },
            Scan::OscEsc => {
                if b == b'\\' {
                    out.extend(parse_osc(&self.buf));
                    self.state = Scan::Ground;
                } else {
                    // An ESC that was not a terminator starts something new.
                    self.state = Scan::Esc;
                    self.step(b, out);
                }
            }
            Scan::Csi => {
                if (0x40..=0x7e).contains(&b) {
                    let params = &self.buf[..];
                    if matches!(params, b"?1049" | b"?1047" | b"?47") && (b == b'h' || b == b'l') {
                        out.push(Signal::AltScreen(b == b'h'));
                    }
                    self.state = Scan::Ground;
                } else if self.buf.len() < CSI_MAX {
                    self.buf.push(b);
                } else {
                    self.state = Scan::Ground;
                }
            }
        }
    }
}

/// The OSC's payload (between `ESC ]` and its terminator) as a signal.
fn parse_osc(p: &[u8]) -> Option<Signal> {
    let s = std::str::from_utf8(p).ok()?;
    if let Some(rest) = s.strip_prefix("133;") {
        let mut parts = rest.split(';');
        return match parts.next()? {
            "C" => Some(Signal::CommandStart),
            "D" => Some(Signal::CommandEnd(
                parts.next().and_then(|c| c.parse().ok()),
            )),
            _ => None,
        };
    }
    if let Some(rest) = s.strip_prefix("9;4;") {
        return rest.split(';').next()?.parse().ok().map(Signal::Progress);
    }
    if s.starts_with("9;") || s.starts_with("777;notify") || s.starts_with("99;") {
        return Some(Signal::Notify);
    }
    None
}

/// One card's command in flight, session-only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    /// When the running command started; `None` at the prompt.
    pub since: Option<f64>,
    /// A hook arrived during this command: the agent's hooks own the state.
    pub hooked: bool,
    /// The command took over the screen (vim, htop).
    pub interactive: bool,
    /// The state came from the program's own progress report.
    pub progress: bool,
}

impl Track {
    /// A hook arrived: while this command runs, the hooks decide.
    pub fn hook(&mut self) {
        if self.since.is_some() {
            self.hooked = true;
        }
    }

    /// A command is running that this module speaks for, so the stale
    /// sweep (made for crashed agents) must not clear its working: a quiet
    /// ten-minute build is still running.
    pub fn owns_working(&self) -> bool {
        self.since.is_some() && !self.hooked
    }

    /// The card's state after `sig`. `agent_card`: the card has an agent
    /// session, so notifications and progress are the agent's business.
    pub fn apply(
        &mut self,
        agent: AgentState,
        agent_card: bool,
        sig: &Signal,
        now: f64,
    ) -> AgentState {
        use AgentState::*;
        let hooks_own = self.hooked || agent_card;
        match sig {
            Signal::CommandStart => {
                *self = Track {
                    since: Some(now),
                    ..Track::default()
                };
                // You are back in this card: the last command's result has
                // been seen.
                match agent {
                    Done | Waiting => None,
                    s => s,
                }
            }
            Signal::CommandEnd(status) => {
                let Some(since) = self.since.take() else {
                    return agent;
                };
                let long = now - since >= LONG_MS;
                let decided = !self.hooked && !self.interactive && long;
                let was = std::mem::take(self);
                if !decided {
                    // A short command leaves the card as it was, except a
                    // working this module set itself.
                    return if agent == Working && !was.hooked {
                        None
                    } else {
                        agent
                    };
                }
                match status {
                    Some(0) | Option::None => Done,
                    Some(_) => Waiting,
                }
            }
            Signal::AltScreen(on) => {
                if *on && self.since.is_some() {
                    self.interactive = true;
                    if agent == Working && !self.hooked {
                        return None;
                    }
                }
                agent
            }
            Signal::Progress(p) if !hooks_own => match p {
                1 | 3 => {
                    self.progress = true;
                    Working
                }
                2 | 4 => Waiting,
                0 if self.progress && agent == Working => {
                    self.progress = false;
                    Done
                }
                _ => agent,
            },
            Signal::Notify if !hooks_own => Waiting,
            _ => agent,
        }
    }

    /// Working once a command has run `LONG_MS`, if nothing else set a
    /// state first.
    pub fn tick(&self, agent: AgentState, now: f64) -> AgentState {
        match self.since {
            Some(since)
                if agent == AgentState::None
                    && !self.hooked
                    && !self.interactive
                    && now - since >= LONG_MS =>
            {
                AgentState::Working
            }
            _ => agent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AgentState::*;

    #[test]
    fn the_scanner_finds_marks_across_chunks_and_both_terminators() {
        let mut s = Scanner::default();
        let mut got = s.feed(b"hi\x1b]133;C\x07out\x1b]13");
        got.extend(s.feed(b"3;D;2\x1b\\ \x1b]9;4;1;40\x07 \x1b[?1049h\x1b[?1049l"));
        got.extend(s.feed(b"\x1b]0;title\x07\x1b]9;build done\x07\x1b]777;notify;a;b\x07"));
        assert_eq!(
            got,
            [
                Signal::CommandStart,
                Signal::CommandEnd(Some(2)),
                Signal::Progress(1),
                Signal::AltScreen(true),
                Signal::AltScreen(false),
                Signal::Notify,
                Signal::Notify,
            ]
        );
    }

    #[test]
    fn a_long_osc_is_not_buffered_and_the_stream_recovers() {
        let mut s = Scanner::default();
        let mut big = b"\x1b]52;c;".to_vec();
        big.extend(std::iter::repeat_n(b'A', 100_000));
        big.extend(b"\x07\x1b]133;C\x07");
        assert_eq!(s.feed(&big), [Signal::CommandStart]);
        assert!(s.buf.len() <= OSC_MAX);
    }

    fn run(t: &mut Track, start: f64, end: f64, status: i32) -> AgentState {
        let a = t.apply(None, false, &Signal::CommandStart, start);
        let a = t.tick(a, end - 1.);
        t.apply(a, false, &Signal::CommandEnd(Some(status)), end)
    }

    #[test]
    fn a_short_command_changes_nothing() {
        let mut t = Track::default();
        assert_eq!(run(&mut t, 0., 800., 0), None);
        assert_eq!(
            run(&mut t, 0., 800., 1),
            None,
            "a quick typo is not a failure worth a lamp"
        );
    }

    #[test]
    fn a_long_command_works_then_is_done_or_waiting() {
        let mut t = Track::default();
        let a = t.apply(None, false, &Signal::CommandStart, 0.);
        assert_eq!(t.tick(a, 4_000.), None, "not yet");
        let a = t.tick(a, LONG_MS);
        assert_eq!(a, Working);
        assert!(t.owns_working(), "the stale sweep leaves it alone");
        assert_eq!(
            t.apply(a, false, &Signal::CommandEnd(Some(0)), 60_000.),
            Done
        );
        assert_eq!(run(&mut Track::default(), 0., 60_000., 101), Waiting);
    }

    #[test]
    fn the_next_command_clears_the_last_result() {
        let mut t = Track::default();
        assert_eq!(t.apply(Done, false, &Signal::CommandStart, 0.), None);
        assert_eq!(t.apply(Waiting, false, &Signal::CommandStart, 0.), None);
    }

    #[test]
    fn a_full_screen_program_never_colours_the_card() {
        let mut t = Track::default();
        let a = t.apply(None, false, &Signal::CommandStart, 0.);
        let a = t.tick(a, LONG_MS);
        assert_eq!(a, Working, "before it took the screen");
        let a = t.apply(a, false, &Signal::AltScreen(true), LONG_MS + 1.);
        assert_eq!(a, None);
        assert_eq!(t.tick(a, 600_000.), None);
        assert_eq!(
            t.apply(a, false, &Signal::CommandEnd(Some(0)), 600_000.),
            None
        );
    }

    #[test]
    fn hooks_win_while_an_agent_runs() {
        let mut t = Track::default();
        let a = t.apply(None, false, &Signal::CommandStart, 0.);
        t.hook(); // claude's SessionStart
        assert_eq!(t.tick(a, 60_000.), None, "no promotion under hooks");
        assert!(!t.owns_working());
        assert_eq!(t.apply(Done, false, &Signal::Notify, 1.), Done);
        assert_eq!(t.apply(Done, false, &Signal::Progress(1), 1.), Done);
        // Claude exits: the hooks' last word stands.
        assert_eq!(
            t.apply(Done, false, &Signal::CommandEnd(Some(0)), 90_000.),
            Done
        );
    }

    #[test]
    fn progress_and_notifications_speak_for_programs_without_hooks() {
        let mut t = Track::default();
        assert_eq!(t.apply(None, false, &Signal::Progress(1), 0.), Working);
        assert_eq!(t.apply(Working, false, &Signal::Progress(0), 1.), Done);
        assert_eq!(t.apply(None, false, &Signal::Progress(2), 0.), Waiting);
        assert_eq!(t.apply(None, false, &Signal::Notify, 0.), Waiting);
        assert_eq!(
            t.apply(Done, true, &Signal::Notify, 0.),
            Done,
            "an agent card's own"
        );
    }

    #[test]
    fn an_end_without_a_start_is_ignored() {
        let mut t = Track::default();
        assert_eq!(t.apply(Done, false, &Signal::CommandEnd(Some(1)), 0.), Done);
    }
}
