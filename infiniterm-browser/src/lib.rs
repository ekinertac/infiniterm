//! The browser card's host: one CEF per process, any number of windowless
//! browsers painting into shared frames. `process` is the per-process
//! part (execute_process for the helpers, initialize, the pump,
//! shutdown); `surface` is one browser (open, frames, input, close);
//! `moat` makes Google sign-in pass; `app_protocol` is the two methods
//! CEF needs on gpui's NSApplication.
//!
//! What crosses to the ui is plain data: a `Frame` of BGRA bytes and a
//! size, popups as urls, titles and addresses as strings. The ui owns the
//! texture, the card and the keys.
pub mod app_protocol;
pub mod moat;
pub mod process;
pub mod surface;

pub use surface::{Button, ContextMenuRequest, Frame, Mods, Surface};
