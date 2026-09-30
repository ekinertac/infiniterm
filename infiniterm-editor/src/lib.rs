//! The editor's logic, toolkit-free: `buffer` (rope, cursor, selection,
//! undo), `search` (find and replace), `language` (which grammar reads a
//! path) and `highlight` (tree-sitter spans). The gpui element in
//! `infiniterm-ui/src/editor_body.rs` draws what these say.
pub mod buffer;
pub mod diff;
pub mod explorer;
pub mod highlight;
pub mod jumps;
pub mod language;
pub mod search;
pub mod transforms;
pub mod wrap;
