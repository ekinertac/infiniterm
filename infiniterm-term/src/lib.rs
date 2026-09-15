//! Terminal grid, parsing, and output scheduling boundary for infiniterm-ui.
//!
//! The UI feeds scheduler output to the parser and returns consumed-byte credit.
//! The app owns PTY reads; parsing stays budgeted on the UI thread.
//! See HANDOVER.md and spikes/term-zoom/NOTES.md for that constraint.

pub mod credit;
pub mod scheduler;
