//! The shared pan-start predicate for canvas and card input routing.
//! Port of panMode.ts and its tests. Button numbers match the reference input API.
//! Plain left-drag belongs to card content; middle or command-left can pan.
pub const MIDDLE_BUTTON: u8 = 1;
pub fn starts_pan(button: u8, cmd_held: bool) -> bool {
    button == MIDDLE_BUTTON || (button == 0 && cmd_held)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn middle_with_or_without_cmd() {
        assert!(starts_pan(MIDDLE_BUTTON, false));
        assert!(starts_pan(MIDDLE_BUTTON, true));
    }
    #[test]
    fn left_only_with_cmd() {
        assert!(starts_pan(0, true));
        assert!(!starts_pan(0, false));
    }
    #[test]
    fn plain_left_never_pans() {
        assert!(!starts_pan(0, false));
    }
    #[test]
    fn right_never_pans() {
        assert!(!starts_pan(2, true));
        assert!(!starts_pan(2, false));
    }
}
