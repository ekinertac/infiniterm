//! The pressed-shortcut overlay (`app.keycast`, for screencasts and for
//! showing somebody what you just did): every chord the app handles is
//! shown as its keycaps and the command's name, bottom left, newest at
//! the bottom with the earlier ones stacked above it, each fading out on
//! its own clock. Off by default; a runtime toggle, not a setting, since
//! it is turned on for a recording and off after.
//!
//! `input.rs::key_down` calls `note_chord` after a chord ran; `overlays.rs`
//! draws `render_keycast`; `needs_frame` asks for frames while any entry
//! is alive, so the fade is smooth and the app idles the moment the last
//! one is gone.
use crate::AppView;
use gpui::{div, px, IntoElement, ParentElement, Styled};
use infiniterm_core::shortcuts::{chord_keys, CHORD_SEPARATOR};

/// How long a pressed chord stays: long enough to read it and the name.
pub const KEYCAST_MS: f64 = 2200.;
/// The last part of that is the fade.
pub const KEYCAST_FADE_MS: f64 = 600.;
/// How many stay stacked: more than a hand can press in the time.
pub const KEYCAST_MAX: usize = 6;
/// Big enough to read in a screen recording played back small; was 14,
/// which Ekin asked to be bigger.
const KEYCAST_FONT_PX: f32 = 22.;
const KEYCAST_INSET_PX: f32 = 24.;
const KEYCAST_GAP_PX: f32 = 8.;

#[derive(Clone, Debug, PartialEq)]
pub struct Keycast {
    pub chord: String,
    pub label: String,
    pub at: f64,
}

/// The alpha an entry pressed at `at` has at `now`: 1 until the fade,
/// then down to 0 at `KEYCAST_MS`.
pub fn keycast_alpha(at: f64, now: f64) -> f32 {
    let age = now - at;
    if age >= KEYCAST_MS {
        0.
    } else if age <= KEYCAST_MS - KEYCAST_FADE_MS {
        1.
    } else {
        ((KEYCAST_MS - age) / KEYCAST_FADE_MS) as f32
    }
}

impl AppView {
    /// A chord the app handled, with the command it ran, when the overlay
    /// is on. The same chord pressed again is a new entry: a repeated key
    /// is a thing worth seeing repeated.
    pub fn note_chord(&mut self, chord: &str, label: &str, now: f64) {
        if !self.keycast_on {
            return;
        }
        self.keycasts.push(Keycast {
            chord: chord_keys(chord).join(CHORD_SEPARATOR),
            label: label.to_string(),
            at: now,
        });
        if self.keycasts.len() > KEYCAST_MAX {
            self.keycasts.remove(0);
        }
        self.redraw = true;
    }

    /// Drops what has faded; true while anything is still showing.
    pub fn keycasts_alive(&mut self, now: f64) -> bool {
        self.keycasts.retain(|k| keycast_alpha(k.at, now) > 0.);
        !self.keycasts.is_empty()
    }

    pub fn render_keycast(&self) -> impl IntoElement {
        let ui = self.model.ui_scale as f32;
        let chrome = &self.chrome;
        let now = crate::now_ms();
        let mut column = div()
            .absolute()
            .bottom(px(KEYCAST_INSET_PX * ui))
            .left(px(KEYCAST_INSET_PX * ui))
            .flex()
            .flex_col()
            .items_start()
            .gap(px(KEYCAST_GAP_PX * ui))
            .font_family(chrome.typography.family.clone())
            .font_weight(chrome.typography.regular.weight);
        for k in &self.keycasts {
            let alpha = keycast_alpha(k.at, now);
            column = column.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(KEYCAST_GAP_PX * ui))
                    .px(px(14. * ui))
                    .py(px(9. * ui))
                    .rounded_md()
                    .bg(crate::chrome::with_alpha(chrome.bar_bg, 0.92 * alpha))
                    .text_size(px(chrome.typography.size(KEYCAST_FONT_PX) * ui))
                    .text_color(crate::chrome::with_alpha(chrome.text_muted, alpha))
                    .child(
                        div()
                            .px(px(11. * ui))
                            .py(px(4. * ui))
                            .rounded_sm()
                            .bg(crate::chrome::with_alpha(chrome.text_bright, alpha))
                            .text_color(crate::chrome::with_alpha(chrome.bar_bg, alpha))
                            .font_weight(chrome.typography.bold.weight)
                            .child(k.chord.clone()),
                    )
                    .child(k.label.clone()),
            );
        }
        column
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_holds_then_fades_then_is_gone() {
        assert_eq!(keycast_alpha(0., 0.), 1.);
        assert_eq!(keycast_alpha(0., KEYCAST_MS - KEYCAST_FADE_MS), 1.);
        let mid = keycast_alpha(0., KEYCAST_MS - KEYCAST_FADE_MS / 2.);
        assert!(mid > 0.4 && mid < 0.6, "{mid}");
        assert_eq!(keycast_alpha(0., KEYCAST_MS), 0.);
        assert_eq!(keycast_alpha(0., KEYCAST_MS + 1.), 0.);
    }
}
