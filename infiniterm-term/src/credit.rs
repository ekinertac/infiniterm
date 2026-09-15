//! Parsed-byte acknowledgement ledger for reader backpressure.
//! Port of flowControl.ts and its tests; the parser calls note after consumption.
//! The UI drains remaining acknowledgements each tick so small outputs cannot stall.
pub const ACK_BATCH: usize = 64 * 1024;
#[derive(Default)]
pub struct AckLedger {
    // Preserve JS Map insertion order. Shortcut: linear lookup, fine below ~1k panes.
    pending: Vec<(u32, usize)>,
}
impl AckLedger {
    pub fn note(&mut self, pane: u32, bytes: usize) -> usize {
        let index = self.pending.iter().position(|(p, _)| *p == pane);
        let owed = index.map_or(0, |i| self.pending[i].1) + bytes;
        if owed >= ACK_BATCH {
            if let Some(i) = index {
                self.pending.remove(i);
            }
            return owed;
        }
        if let Some(i) = index {
            self.pending[i].1 = owed;
        } else {
            self.pending.push((pane, owed));
        }
        0
    }
    pub fn drain(&mut self) -> Vec<(u32, usize)> {
        std::mem::take(&mut self.pending)
    }
    pub fn forget(&mut self, pane: u32) {
        self.pending.retain(|(p, _)| *p != pane);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_acks_drain_together() {
        let mut l = AckLedger::default();
        assert_eq!(l.note(1, 100), 0);
        assert_eq!(l.note(1, 200), 0);
        assert_eq!(l.note(2, 50), 0);
        assert_eq!(l.drain(), vec![(1, 300), (2, 50)]);
        assert!(l.drain().is_empty());
    }
    #[test]
    fn full_batch_flushes_only_that_pane() {
        let mut l = AckLedger::default();
        l.note(1, ACK_BATCH - 1);
        l.note(2, 10);
        assert_eq!(l.note(1, 1), ACK_BATCH);
        assert_eq!(l.drain(), vec![(2, 10)]);
    }
    #[test]
    fn oversized_chunk_returns_all_owed() {
        let mut l = AckLedger::default();
        l.note(1, 10);
        assert_eq!(l.note(1, ACK_BATCH * 3), ACK_BATCH * 3 + 10);
    }
    #[test]
    fn forget_closed_pane() {
        let mut l = AckLedger::default();
        l.note(1, 10);
        l.forget(1);
        assert!(l.drain().is_empty());
    }
}
