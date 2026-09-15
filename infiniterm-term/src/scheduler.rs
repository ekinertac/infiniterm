//! Adaptive byte budget and round-robin output delivery for the terminal parser.
//! Port of outputScheduler.ts and its tests. The UI supplies frame timestamps.
//! Queues retain arrival order, with at most four panes parsed per frame.
use std::{collections::VecDeque, ops::Range, sync::Arc};
pub const FRAME_MS: f64 = 1000. / 60.;
pub const LONG_FRAME_FACTOR: f64 = 1.5;
pub const LONG_FRAME_MS: f64 = FRAME_MS * LONG_FRAME_FACTOR;
pub const MIN_BUDGET: usize = 32 * 1024;
pub const MAX_BUDGET: usize = 1024 * 1024;
pub const INITIAL_BUDGET: usize = 64 * 1024;
pub const PANES_PER_FRAME: usize = 4;
/// A gap this short is scheduling jitter, not a real frame boundary; only
/// gaps at least this long update the learned display period.
pub const MIN_PERIOD_SAMPLE_MS: f64 = 4.;
/// A long frame means the ui thread fell behind: shrink the budget so the
/// next frame's parse fits in less time.
pub const BUDGET_SHRINK_FACTOR: f64 = 0.7;
/// A frame that kept pace grows the budget back, slower than it shrinks so
/// a single stutter doesn't get amplified by the next several frames.
pub const BUDGET_GROWTH_FACTOR: f64 = 1.15;
/// Shares the reader's allocation when one chunk spans multiple frames.
#[derive(Clone, Debug)]
pub struct Piece {
    pub pane: u32,
    buffer: Arc<Vec<u8>>,
    range: Range<usize>,
}
impl Piece {
    pub fn data(&self) -> &[u8] {
        &self.buffer[self.range.clone()]
    }
}
pub struct OutputScheduler {
    // Insertion order matches JS Map. Shortcut: O(n), fine below ~1k panes.
    // Add an insertion-ordered index if pane counts grow beyond that.
    queues: Vec<(u32, VecDeque<Piece>)>,
    last_tick: Option<f64>,
    period: f64,
    next: usize,
    pub budget: f64,
}
impl Default for OutputScheduler {
    fn default() -> Self {
        Self {
            queues: vec![],
            last_tick: None,
            period: FRAME_MS,
            next: 0,
            budget: INITIAL_BUDGET as f64,
        }
    }
}
impl OutputScheduler {
    pub fn enqueue(&mut self, pane: u32, data: Vec<u8>) {
        if data.is_empty() {
            return;
        }
        let end = data.len();
        let piece = Piece {
            pane,
            buffer: Arc::new(data),
            range: 0..end,
        };
        if let Some((_, q)) = self.queues.iter_mut().find(|(p, _)| *p == pane) {
            q.push_back(piece);
        } else {
            self.queues.push((pane, VecDeque::from([piece])));
        }
    }
    pub fn forget(&mut self, pane: u32) {
        self.queues.retain(|(p, _)| *p != pane);
    }
    pub fn pending(&self) -> bool {
        !self.queues.is_empty()
    }
    pub fn take(&mut self, now: f64) -> Vec<Piece> {
        if let Some(last) = self.last_tick {
            let dt = now - last;
            if dt >= MIN_PERIOD_SAMPLE_MS {
                self.period = self.period.min(dt);
            }
            self.budget = if dt > self.period * LONG_FRAME_FACTOR {
                (self.budget * BUDGET_SHRINK_FACTOR).max(MIN_BUDGET as f64)
            } else {
                (self.budget * BUDGET_GROWTH_FACTOR).min(MAX_BUDGET as f64)
            };
        }
        if self.queues.is_empty() {
            self.last_tick = None;
            return vec![];
        }
        let count = PANES_PER_FRAME.min(self.queues.len());
        let start = self.next % self.queues.len();
        let active: Vec<_> = (0..count)
            .map(|i| self.queues[(start + i) % self.queues.len()].0)
            .collect();
        self.next = start + count;
        let share = (self.budget / count as f64).floor().max(1.) as usize;
        let mut out = vec![];
        for pane in active {
            let index = self
                .queues
                .iter()
                .position(|(p, _)| *p == pane)
                .expect("active pane remains queued until served");
            let q = &mut self.queues[index].1;
            let mut left = share;
            while left > 0 {
                let Some(mut head) = q.pop_front() else {
                    break;
                };
                if head.data().len() <= left {
                    left -= head.data().len();
                    out.push(head);
                } else {
                    let end = head.range.start + left;
                    out.push(Piece {
                        pane,
                        buffer: Arc::clone(&head.buffer),
                        range: head.range.start..end,
                    });
                    head.range.start = end;
                    q.push_front(head);
                    left = 0;
                }
            }
            if q.is_empty() {
                self.queues.remove(index);
            }
        }
        self.last_tick = if self.pending() { Some(now) } else { None };
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bytes(n: usize) -> Vec<u8> {
        vec![1; n]
    }
    fn total(pieces: &[Piece], pane: u32) -> usize {
        pieces
            .iter()
            .filter(|p| p.pane == pane)
            .map(|p| p.data().len())
            .sum()
    }
    #[test]
    fn dispenses_everything_that_fits() {
        let mut s = OutputScheduler::default();
        s.enqueue(1, bytes(100));
        s.enqueue(1, bytes(50));
        let out = s.take(0.);
        assert_eq!(
            out.iter().map(|p| p.data().len()).collect::<Vec<_>>(),
            vec![100, 50]
        );
        assert!(!s.pending());
    }
    #[test]
    fn even_budget_across_panes() {
        let mut s = OutputScheduler::default();
        s.enqueue(1, bytes(INITIAL_BUDGET));
        s.enqueue(2, bytes(INITIAL_BUDGET));
        let out = s.take(0.);
        assert_eq!(total(&out, 1), INITIAL_BUDGET / 2);
        assert_eq!(total(&out, 2), INITIAL_BUDGET / 2);
        assert!(s.pending());
    }
    #[test]
    fn slicing_preserves_byte_order() {
        let mut s = OutputScheduler::default();
        let data = (0..INITIAL_BUDGET * 4)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>();
        s.enqueue(1, data.clone());
        let mut pieces = s.take(0.);
        pieces.extend(s.take(1000.));
        pieces.extend(s.take(2000.));
        let seen = pieces
            .iter()
            .flat_map(|p| p.data().iter().copied())
            .collect::<Vec<_>>();
        assert!(seen.len() < data.len());
        assert_eq!(seen, data[..seen.len()]);
    }
    #[test]
    fn long_frame_cuts_short_frames_grow() {
        let mut s = OutputScheduler::default();
        s.enqueue(1, bytes(MAX_BUDGET * 4));
        s.take(0.);
        s.take(LONG_FRAME_MS + 1.);
        assert!((s.budget - INITIAL_BUDGET as f64 * 0.7).abs() < 1e-5);
        let mut t = LONG_FRAME_MS + 1.;
        for _ in 0..10 {
            t += 16.;
            s.take(t);
        }
        assert!(s.budget > INITIAL_BUDGET as f64);
    }
    #[test]
    fn floor_and_ceiling() {
        let mut s = OutputScheduler::default();
        s.enqueue(1, bytes(MAX_BUDGET * 64));
        let mut t = 0.;
        for _ in 0..20 {
            t += 1000.;
            s.take(t);
        }
        assert_eq!(s.budget, MIN_BUDGET as f64);
        for _ in 0..200 {
            t += 16.;
            s.take(t);
        }
        assert_eq!(s.budget, MAX_BUDGET as f64);
    }
    #[test]
    fn quiet_gap_is_not_slow_frame() {
        let mut s = OutputScheduler::default();
        s.enqueue(1, bytes(10));
        s.take(0.);
        s.enqueue(1, bytes(10));
        s.take(5000.);
        assert_eq!(s.budget, INITIAL_BUDGET as f64);
    }
    #[test]
    fn four_panes_per_frame_all_get_turn() {
        let mut s = OutputScheduler::default();
        let n = PANES_PER_FRAME * 3;
        for p in 1..=n {
            s.enqueue(p as u32, bytes(1));
        }
        let mut seen = std::collections::HashSet::new();
        for t in [0., 16., 32.] {
            let out = s.take(t);
            let ids = out
                .iter()
                .map(|p| p.pane)
                .collect::<std::collections::HashSet<_>>();
            assert_eq!(ids.len(), PANES_PER_FRAME);
            seen.extend(ids);
        }
        assert_eq!(seen.len(), n);
        assert!(!s.pending());
    }
    #[test]
    fn learns_fast_display_period() {
        let mut s = OutputScheduler::default();
        s.enqueue(1, bytes(MAX_BUDGET * 64));
        let mut t = 0.;
        for _ in 0..5 {
            t += 8.3;
            s.take(t);
        }
        let before = s.budget;
        t += 16.7;
        s.take(t);
        assert!((s.budget - before * 0.7).abs() < 1e-5);
    }
    #[test]
    fn forget_closed_pane() {
        let mut s = OutputScheduler::default();
        s.enqueue(1, bytes(10));
        s.forget(1);
        assert!(!s.pending());
        assert!(s.take(0.).is_empty());
    }
    // Native storage must preserve JS Map insertion order, not sort pane ids.
    #[test]
    fn queue_order_is_arrival_order() {
        let mut s = OutputScheduler::default();
        for p in [9, 1, 8, 2, 7] {
            s.enqueue(p, bytes(1));
        }
        assert_eq!(
            s.take(0.).iter().map(|p| p.pane).collect::<Vec<_>>(),
            vec![9, 1, 8, 2]
        );
        assert_eq!(s.take(16.)[0].pane, 7);
    }
    #[test]
    fn empty_chunks_do_not_queue() {
        let mut s = OutputScheduler::default();
        s.enqueue(1, vec![]);
        assert!(!s.pending());
    }
}
