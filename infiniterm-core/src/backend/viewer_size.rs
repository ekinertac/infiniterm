//! A viewer that declares its own size (#805): the phone joins a session with
//! `ift proxy join session <id> --size <cols>x<rows>`, and the host sizes the
//! session's pty to whichever side gave input last, the tmux
//! `window-size latest` rule. The card's rect on the canvas is never changed:
//! while a viewer holds the size, the card draws the viewer's grid inside its
//! own frame, with a thin line where the grid ends. When the viewer leaves,
//! for any reason, the card has its own size back, so no restore depends on
//! the viewer.
//!
//! Ekin's rules (#805, 2026-11-24):
//! - Being connected is not a claim, only input is. On the host, only the
//!   person's own key, click or paste in the card takes the size back; focus,
//!   `ift send` and hooks are programs and do not (`terminals.rs` counts a
//!   body's `outgoing` and `clicked`, never the emulator's replies).
//! - The host takes it back at once: the person is there. Only a viewer's
//!   claim waits `HOLD_MS` after the last switch, so a phone typing while
//!   the person at the Mac types does not resize the pty on every key.
//! - Latest input wins between viewers; when the holder leaves the card goes
//!   back to its own size, with no history of earlier claimants.
//! - A viewer's input in its first `JOIN_GRACE_MS` claims nothing: right
//!   after a join the phone's emulator answers the queries in the replayed
//!   scrollback, and those replies arrive as keys (`counts_as_typing`).
//! - `ift serve` (no local user) follows the same rule: the latest viewer
//!   keeps the size until it leaves (`headless.rs`).
//!
//! Pure, and on every platform: the join server reports what a viewer did
//! (`SizeEvent`, from `join.rs`), the app keeps one `SizeClaims` per session
//! and asks it for the pty size (`terminals.rs`).

/// The sizes a viewer may declare. Outside them a size is clamped, not
/// refused: a phone in landscape with a small font is still a phone.
pub const MIN_COLS: u16 = 20;
pub const MAX_COLS: u16 = 500;
pub const MIN_ROWS: u16 = 5;
pub const MAX_ROWS: u16 = 200;

/// After the size changes hands, a viewer's input does not claim it for this
/// long, so a phone typing while someone types on the Mac does not resize the
/// pty on every key. The host's own input is never held (the person is
/// there), and a viewer leaving is not held: the card gets its size back at
/// once.
pub const HOLD_MS: u64 = 2_000;

/// A viewer's input this soon after it joined is not a claim: the emulator
/// answering queries from the replay, not a person (#805). About a second
/// covers the replay of a long scrollback on a phone.
pub const JOIN_GRACE_MS: u64 = 1_000;

/// A sized viewer's keys are reported at most this often: the app needs to
/// know who typed last, not every key.
pub const TYPED_EVERY_MS: u64 = 250;

/// Whether a viewer's input frame counts as typing, to be reported: not in
/// its first `JOIN_GRACE_MS`, and not again within `TYPED_EVERY_MS` of the
/// last report. `since_join_ms` and `since_report_ms` are measured by the
/// join server, which sees the frames as they come.
pub fn counts_as_typing(since_join_ms: u64, since_report_ms: Option<u64>) -> bool {
    since_join_ms >= JOIN_GRACE_MS && since_report_ms.is_none_or(|ms| ms >= TYPED_EVERY_MS)
}

/// `<cols>x<rows>`, as `--size` and the hello carry it, clamped. `None` for
/// anything else; the caller refuses it with a sentence.
pub fn parse_size(text: &str) -> Option<(u16, u16)> {
    let (cols, rows) = text.split_once('x')?;
    let number = |s: &str| -> Option<u16> {
        if s.is_empty() || s.len() > 4 || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    };
    Some(clamp_size(number(cols)?, number(rows)?))
}

/// A declared size inside the limits.
pub fn clamp_size(cols: u16, rows: u16) -> (u16, u16) {
    (
        cols.clamp(MIN_COLS, MAX_COLS),
        rows.clamp(MIN_ROWS, MAX_ROWS),
    )
}

/// What one viewer of a session did, from the join server to the app.
/// `viewer` is a number the server gives each sized connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeEvent {
    pub session: String,
    pub viewer: u64,
    pub change: SizeChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeChange {
    /// Its screen is this size: at the hello and at every later `Resize`.
    Declared { cols: u16, rows: u16 },
    /// It sent keys.
    Typed,
    /// Its connection closed.
    Left,
}

/// Who holds one session's size, and the sizes its viewers declared.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SizeClaims {
    /// Connected sized viewers, in the order they joined.
    viewers: Vec<(u64, (u16, u16))>,
    /// The viewer that typed last; `None` is the card on the host.
    holder: Option<u64>,
    /// When the size last changed hands, for `HOLD_MS`.
    switched_at: Option<u64>,
}

impl SizeClaims {
    pub fn apply(&mut self, viewer: u64, change: SizeChange, now_ms: u64) {
        match change {
            SizeChange::Declared { cols, rows } => {
                let size = clamp_size(cols, rows);
                match self.viewers.iter_mut().find(|(v, _)| *v == viewer) {
                    Some(entry) => entry.1 = size,
                    None => self.viewers.push((viewer, size)),
                }
            }
            SizeChange::Typed => {
                let known = self.viewers.iter().any(|(v, _)| *v == viewer);
                if known && self.holder != Some(viewer) && self.may_switch(now_ms) {
                    self.holder = Some(viewer);
                    self.switched_at = Some(now_ms);
                }
            }
            SizeChange::Left => {
                self.viewers.retain(|(v, _)| *v != viewer);
                if self.holder == Some(viewer) {
                    self.holder = None;
                    self.switched_at = None;
                }
            }
        }
    }

    /// The person at the host typed, clicked or pasted in the card: the size
    /// is the card's again at once, whatever the hold (Ekin, #805). The
    /// switch starts the hold against the viewers.
    pub fn local_input(&mut self, now_ms: u64) {
        if self.holder.is_some() {
            self.holder = None;
            self.switched_at = Some(now_ms);
        }
    }

    /// The size the pty should have: the holding viewer's, or `None` for the
    /// card's own.
    pub fn size(&self) -> Option<(u16, u16)> {
        let holder = self.holder?;
        self.viewers
            .iter()
            .find(|(v, _)| *v == holder)
            .map(|(_, size)| *size)
    }

    /// No sized viewer is connected: the app can forget this session.
    pub fn is_empty(&self) -> bool {
        self.viewers.is_empty()
    }

    fn may_switch(&self, now_ms: u64) -> bool {
        self.switched_at
            .is_none_or(|at| now_ms >= at.saturating_add(HOLD_MS))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: u64 = 1;
    const TABLET: u64 = 2;

    fn declared(cols: u16, rows: u16) -> SizeChange {
        SizeChange::Declared { cols, rows }
    }

    #[test]
    fn a_size_is_columns_x_rows_clamped() {
        assert_eq!(parse_size("80x24"), Some((80, 24)));
        assert_eq!(parse_size("1x1"), Some((MIN_COLS, MIN_ROWS)));
        assert_eq!(parse_size("9999x9999"), Some((MAX_COLS, MAX_ROWS)));
        for bad in [
            "", "80", "80x", "x24", "80X24", "-1x24", "80x24x1", "+8x24", "80 x24", "12345x2",
        ] {
            assert_eq!(parse_size(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn being_connected_is_not_a_claim() {
        let mut c = SizeClaims::default();
        c.apply(PHONE, declared(40, 30), 0);
        assert_eq!(
            c.size(),
            None,
            "the card keeps its size until the phone types"
        );
        c.apply(PHONE, SizeChange::Typed, 10);
        assert_eq!(c.size(), Some((40, 30)));
    }

    // Ekin (#805): the host takes the size back at once; only a viewer's
    // claim waits the hold after a switch.
    #[test]
    fn the_host_takes_it_back_at_once_and_only_a_viewer_waits_the_hold() {
        let mut c = SizeClaims::default();
        c.apply(PHONE, declared(40, 30), 0);
        c.apply(PHONE, SizeChange::Typed, 1_000);
        assert_eq!(c.size(), Some((40, 30)));
        c.local_input(1_001);
        assert_eq!(c.size(), None, "the person at the Mac is there: no hold");
        c.apply(PHONE, SizeChange::Typed, 1_001 + HOLD_MS - 1);
        assert_eq!(c.size(), None, "the phone waits the hold");
        c.apply(PHONE, SizeChange::Typed, 1_001 + HOLD_MS);
        assert_eq!(c.size(), Some((40, 30)));
        // And back again at once, however soon after.
        c.local_input(1_002 + HOLD_MS);
        assert_eq!(c.size(), None);
    }

    #[test]
    fn host_input_with_no_viewer_holding_starts_no_hold() {
        let mut c = SizeClaims::default();
        c.apply(PHONE, declared(40, 30), 0);
        c.local_input(5);
        c.apply(PHONE, SizeChange::Typed, 6);
        assert_eq!(c.size(), Some((40, 30)), "nothing changed hands");
    }

    #[test]
    fn a_viewers_first_second_and_a_burst_of_keys_are_not_reported_as_typing() {
        assert!(!counts_as_typing(0, None), "the replay's query replies");
        assert!(!counts_as_typing(JOIN_GRACE_MS - 1, None));
        assert!(counts_as_typing(JOIN_GRACE_MS, None));
        assert!(!counts_as_typing(5_000, Some(TYPED_EVERY_MS - 1)));
        assert!(counts_as_typing(5_000, Some(TYPED_EVERY_MS)));
    }

    #[test]
    fn leaving_gives_the_card_its_size_back_at_once() {
        let mut c = SizeClaims::default();
        c.apply(PHONE, declared(40, 30), 0);
        c.apply(PHONE, SizeChange::Typed, 0);
        c.apply(PHONE, SizeChange::Left, 1);
        assert_eq!(c.size(), None);
        assert!(c.is_empty());
    }

    #[test]
    fn a_new_declared_size_applies_to_the_holder_at_once() {
        let mut c = SizeClaims::default();
        c.apply(PHONE, declared(40, 30), 0);
        c.apply(PHONE, SizeChange::Typed, 0);
        c.apply(PHONE, declared(90, 20), 5);
        assert_eq!(c.size(), Some((90, 20)), "the phone was turned");
    }

    #[test]
    fn two_viewers_and_the_holder_leaving_goes_back_to_the_card() {
        let mut c = SizeClaims::default();
        c.apply(PHONE, declared(40, 30), 0);
        c.apply(TABLET, declared(120, 40), 0);
        c.apply(PHONE, SizeChange::Typed, 0);
        c.apply(TABLET, SizeChange::Typed, HOLD_MS);
        assert_eq!(c.size(), Some((120, 40)));
        c.apply(TABLET, SizeChange::Left, HOLD_MS + 1);
        assert_eq!(c.size(), None, "not to the phone until it types");
        assert!(!c.is_empty());
        c.apply(PHONE, SizeChange::Typed, HOLD_MS + 2);
        assert_eq!(c.size(), Some((40, 30)), "a leave is not held");
    }

    #[test]
    fn keys_from_an_unsized_viewer_claim_nothing() {
        let mut c = SizeClaims::default();
        c.apply(PHONE, SizeChange::Typed, 0);
        assert_eq!(c.size(), None);
    }
}
