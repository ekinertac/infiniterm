//! The caret's jump history: where it was before it last leapt (go to
//! line, a search match, the top or bottom of the file, a matching
//! bracket, a click far away), so Ctrl+- walks back and Ctrl+Shift+- forward,
//! Sublime's Jump Back / Jump Forward. Pure: char positions in, char
//! positions out.
//!
//! shortcut: positions are char indices and are not shifted by edits made
//! after they were recorded; the caller clamps to the buffer's length. An
//! edit above a recorded spot leaves it a few lines off, which is what a
//! browser's back button does to a scrolled page too. Move to tracked
//! anchors if that ever bites.

/// How many jumps are remembered each way.
const CAP: usize = 100;

#[derive(Clone, Debug, Default)]
pub struct Jumps {
    back: Vec<usize>,
    forward: Vec<usize>,
}

impl Jumps {
    /// The caret is about to leap away from `from`. A new leap ends the
    /// forward trail, as a new page does in a browser.
    pub fn record(&mut self, from: usize) {
        if self.back.last() == Some(&from) {
            return;
        }
        self.back.push(from);
        if self.back.len() > CAP {
            self.back.remove(0);
        }
        self.forward.clear();
    }

    /// Ctrl+-: the spot before the last leap, with `here` kept for a way
    /// forward again. `None` at the start of the trail.
    pub fn back(&mut self, here: usize) -> Option<usize> {
        let to = self.back.pop()?;
        self.forward.push(here);
        Some(to)
    }

    /// Ctrl+Shift+-: undoes a back.
    pub fn forward(&mut self, here: usize) -> Option<usize> {
        let to = self.forward.pop()?;
        self.back.push(here);
        Some(to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_and_forward_walk_the_trail_like_a_browser() {
        let mut j = Jumps::default();
        j.record(10); // leapt away from 10 (to 50, say)
        j.record(50); // then away from 50 (to 90)
        assert_eq!(j.back(90), Some(50));
        assert_eq!(j.back(50), Some(10));
        assert_eq!(j.back(10), None);
        assert_eq!(j.forward(10), Some(50));
        assert_eq!(j.forward(50), Some(90));
        assert_eq!(j.forward(90), None);
    }

    #[test]
    fn a_new_leap_ends_the_forward_trail_and_repeats_are_not_stored() {
        let mut j = Jumps::default();
        j.record(10);
        j.record(10);
        assert_eq!(j.back(40), Some(10));
        assert_eq!(j.back(10), None);
        j.record(20);
        assert_eq!(j.forward(20), None);
    }

    #[test]
    fn the_trail_is_capped() {
        let mut j = Jumps::default();
        for i in 0..CAP + 20 {
            j.record(i);
        }
        let mut n = 0;
        while j.back(0).is_some() {
            n += 1;
        }
        assert_eq!(n, CAP);
    }
}
