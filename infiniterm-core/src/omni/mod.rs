//! The omnibox: Cmd+L, an address or a search, ranked against what is open
//! and where you have been. Every decision here is a pure function; the
//! model holds state only, and `infiniterm-ui/src/omnibox.rs` draws it.
//!
//! Ported from the omnibox in ~/Code/glass (Electron), which settled the
//! ranking and the tab-to-search flow. See
//! docs/superpowers/specs/2026-09-16-omnibox-design.md.
//!
//! NOT the command palette: that matches a fixed list of commands by fuzzy
//! score, this takes free text and mixes sources.
pub mod address;
