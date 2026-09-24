//! What a full screen toggle does, decided without AppKit. The ui's
//! `fullscreen.rs` answers `toggleFullScreen:` (the green button, the
//! `app.fullscreen` command, the Window menu) and asks `decide` which of
//! three things to do; everything it then does is Cocoa wiring.
//!
//! Why a second kind of full screen at all: macOS's own one on a MacBook with
//! a notch puts the window below the camera housing and paints the strip
//! beside it black, so the top 32 pt of the screen show nothing. `Cover`
//! makes the window as large as `NSScreen.frame`, traffic lights hidden, with the
//! menu bar and Dock auto-hidden, which is what Ghostty, kitty and WezTerm
//! call non-native full screen. The notch then covers the middle of the
//! title bar, which holds nothing: the workspace tabs start at the left.
//!
//! The setting is read at the moment of the toggle, so leaving a covered
//! window always un-covers it, whatever `ui.fullscreen` says by then, and a
//! window already in macOS's full screen always leaves it the native way.
use crate::config::FullscreenMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    /// Hand the toggle to AppKit's own `toggleFullScreen:`.
    Native,
    /// Save the frame and style, then cover the whole screen.
    EnterCover,
    /// Put the saved frame and style back.
    ExitCover,
}

/// `covering`: the window is in cover mode now. `native_full`: it is in
/// macOS's full screen now (its style mask carries the full screen bit).
pub fn decide(mode: FullscreenMode, covering: bool, native_full: bool) -> Toggle {
    if covering {
        Toggle::ExitCover
    } else if native_full || mode == FullscreenMode::Native {
        Toggle::Native
    } else {
        Toggle::EnterCover
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use FullscreenMode::{Cover, Native};

    #[test]
    fn cover_mode_covers_and_uncovers() {
        assert_eq!(decide(Cover, false, false), Toggle::EnterCover);
        assert_eq!(decide(Cover, true, false), Toggle::ExitCover);
    }

    #[test]
    fn native_mode_is_appkits_toggle() {
        assert_eq!(decide(Native, false, false), Toggle::Native);
        assert_eq!(decide(Native, false, true), Toggle::Native);
    }

    // The setting changed while the window was full screen: the way out is
    // the way it went in, or the window is stranded covering (or in a
    // Space) with no toggle that undoes it.
    #[test]
    fn leaving_goes_out_the_way_it_came_in() {
        assert_eq!(decide(Native, true, false), Toggle::ExitCover);
        assert_eq!(decide(Cover, false, true), Toggle::Native);
    }
}
