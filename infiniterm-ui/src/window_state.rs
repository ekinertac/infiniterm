//! Where the window was: its frame and whether it was maximized or full
//! screen, kept in `window.json` beside `workspace.json` and put back at
//! the next launch. The reference gets this from `tauri-plugin-window-state`;
//! the mapping asks the port to own it. Only the ui crate knows gpui's
//! `WindowBounds`, so the file lives here rather than in core.
//!
//! The frame is read from the window each frame and written half a second
//! after it last changed (`AppView::schedule_window_save`), so a drag is one
//! write. A missing or unreadable file means the default centred window.
use gpui::{point, px, size, Bounds, Pixels, WindowBounds};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// "windowed", "maximized" or "fullscreen"; the frame is the restore
    /// size for the last two.
    pub mode: String,
}

fn path() -> PathBuf {
    infiniterm_core::paths::app_support_dir().join("window.json")
}

impl WindowState {
    pub fn of(bounds: WindowBounds) -> WindowState {
        let (b, mode) = match bounds {
            WindowBounds::Windowed(b) => (b, "windowed"),
            WindowBounds::Maximized(b) => (b, "maximized"),
            WindowBounds::Fullscreen(b) => (b, "fullscreen"),
        };
        WindowState {
            x: f32::from(b.origin.x),
            y: f32::from(b.origin.y),
            w: f32::from(b.size.width),
            h: f32::from(b.size.height),
            mode: mode.to_string(),
        }
    }

    pub fn bounds(&self) -> WindowBounds {
        let b: Bounds<Pixels> =
            Bounds::new(point(px(self.x), px(self.y)), size(px(self.w), px(self.h)));
        match self.mode.as_str() {
            "maximized" => WindowBounds::Maximized(b),
            "fullscreen" => WindowBounds::Fullscreen(b),
            _ => WindowBounds::Windowed(b),
        }
    }

    /// The saved state, if there is one that makes sense: a window smaller
    /// than a card's chrome is a corrupt file, not a preference.
    pub fn load() -> Option<WindowState> {
        let text = std::fs::read_to_string(path()).ok()?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Option<WindowState> {
        let s: WindowState = serde_json::from_str(text).ok()?;
        (s.w >= 200. && s.h >= 200. && s.x.is_finite() && s.y.is_finite()).then_some(s)
    }

    pub fn save(&self) {
        if let Ok(text) = serde_json::to_string(self) {
            if let Some(dir) = path().parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path(), text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_round_trip_through_the_file_text() {
        let b = Bounds::new(point(px(10.), px(20.)), size(px(1600.), px(1000.)));
        for wb in [
            WindowBounds::Windowed(b),
            WindowBounds::Maximized(b),
            WindowBounds::Fullscreen(b),
        ] {
            let s = WindowState::of(wb);
            let text = serde_json::to_string(&s).unwrap();
            let back = WindowState::parse(&text).unwrap();
            assert_eq!(back, s);
            assert_eq!(back.bounds(), wb);
        }
    }

    #[test]
    fn a_tiny_or_broken_frame_is_ignored() {
        assert!(WindowState::parse(r#"{"x":0,"y":0,"w":10,"h":10,"mode":"windowed"}"#).is_none());
        assert!(WindowState::parse("nonsense").is_none());
    }
}
