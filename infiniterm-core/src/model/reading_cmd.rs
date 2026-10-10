//! Zoom on a card's bottom, once called reading mode (#313, #340): zoom to the bottom of a terminal card at `ui.cardZoom`
//! and pan along it with Cmd+Up and Cmd+Down, for reading a regular tall card
//! at a size the fit leaves too small. Cmd+1 on a card that is already fitted
//! enters it; Cmd+1 or Cmd+2 (any viewport change: `Model::stop_framing`)
//! leaves it. Cmd+Up and Cmd+Down mean this only while it is on
//! (`register::resolve_chord`); at every other time they are the shell's.
//!
//! The geometry is `reading.rs`, tested; this file is the state and the
//! commands. Related: `canvas_cmd` (`canvas.zoom.fitCard` decides when Cmd+1
//! enters), `Model::note_input` (typing snaps the view back to the bottom).
use super::{Effect, Model};
use crate::commands::CommandRegistry;
use crate::reading::{frame, pan, Frame};
use crate::saved_layout::CardKind;
use crate::viewport::Viewport;

/// Rows one Cmd+Up or Cmd+Down moves.
const PAN_ROWS: f64 = 3.;

/// The card being read and where the view is on it.
#[derive(Clone, Debug)]
pub struct Reading {
    pub card: String,
    pub frame: Frame,
    pub y: f64,
}

impl Model {
    /// Ends framing and reading: the view is no longer a statement about one
    /// card. Every place that moves the view by hand calls this.
    pub fn stop_framing(&mut self) {
        self.framing = false;
        self.framed_card = None;
        self.reading = None;
    }

    /// Zooms to `ui.cardZoom` on the focused terminal card, its bottom at the
    /// window's bottom.
    pub fn read_card(&mut self) {
        let Some(card) = self.focused().cloned() else {
            return;
        };
        if card.kind != CardKind::Terminal {
            self.notify("reading mode is for terminal cards");
            return;
        }
        let scale = self.config.ui.card_zoom;
        if scale <= 0. {
            self.notify("zooming on a card is off: set ui.cardZoom above 0");
            return;
        }
        let f = frame(card.rect, self.view_size, scale);
        self.effects.push(Effect::AnimateFit(Viewport {
            x: f.x,
            y: f.hi,
            scale,
        }));
        self.selection.maximized = false;
        // Framing stays on, so moving focus re-fits the new card (which leaves
        // reading), as it does after Cmd+1.
        self.framing = true;
        self.framed_card = Some(card.id.clone());
        self.reading = Some(Reading {
            card: card.id,
            frame: f,
            y: f.hi,
        });
    }

    /// Moves the view `rows` rows of the card's text; negative is up.
    pub fn read_pan(&mut self, rows: f64) {
        let Some(r) = self.reading.clone() else {
            self.notify("not zoomed on a card: Cmd+1 on a fitted terminal card does that");
            return;
        };
        let row_h = self.config.terminal.font_size * self.config.terminal.line_height;
        self.read_to(pan(r.y, rows * row_h, &r.frame));
    }

    /// Back to the bottom of the card, where the prompt is: typing does this,
    /// so a keystroke is never typed into a part of the card you cannot see.
    pub fn read_to_bottom(&mut self) {
        if let Some(r) = &self.reading {
            let bottom = r.frame.hi;
            if r.y != bottom {
                self.read_to(bottom);
            }
        }
    }

    fn read_to(&mut self, y: f64) {
        let Some(r) = &mut self.reading else {
            return;
        };
        r.y = y;
        let x = r.frame.x;
        self.effects.push(Effect::AnimatePan { x, y });
    }
}

pub fn register(r: &mut CommandRegistry<Model>) {
    r.register(
        "canvas.zoom.cardBottom",
        "Canvas: zoom in on the focused card's bottom",
        |m| m.read_card(),
    );
    r.register(
        "canvas.zoom.cardBottom.up",
        "Canvas: pan up while zoomed on a card (Cmd+Up)",
        |m| m.read_pan(-PAN_ROWS),
    );
    r.register(
        "canvas.zoom.cardBottom.down",
        "Canvas: pan down while zoomed on a card (Cmd+Down)",
        |m| m.read_pan(PAN_ROWS),
    );
}
