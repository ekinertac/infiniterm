//! The terminal engine boundary for infiniterm-ui: the grid (alacritty's
//! Term and parser as a byte sink handing out frames), the output scheduler
//! and the ack ledger (the app's backpressure, ported), the key and mouse
//! encoders, and the palette. No toolkit types anywhere in here; the ui
//! paints what `Grid::frame` returns. See HANDOVER.md, Phase 4.

pub mod credit;
pub mod grid;
pub mod keys;
pub mod mouse;
pub mod palette;
pub mod scheduler;
pub mod visual_keys;
