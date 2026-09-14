//! Terminal grid, parsing, and output scheduling boundary for infiniterm-ui.
//!
//! Phase 0 reserves the crate; later phases port the scheduler and terminal.
//! The app will own PTY reads and budget parsing on the UI thread.
//! See HANDOVER.md and spikes/term-zoom/NOTES.md for that constraint.
