//! Pure app logic and backend services for the native port.
//!
//! The terminal and UI crates will consume these modules. Keep this crate
//! independent of rendering and card bodies; see HANDOVER.md for the port plan.

pub mod viewport;

pub mod grid;

pub mod layout;

pub mod resize;

pub mod cards;

pub mod navigate;

pub mod slots;

#[cfg(test)]
mod test_support;

pub mod split;

pub mod swap;

pub mod multi_select;

pub mod agent_state;

pub mod groups;

pub mod workspaces;

pub mod momentum;

pub mod pan_mode;

pub mod chrome;

pub mod format_zoom;

pub mod config;
pub mod fuzzy;
pub mod itermcolors;
pub mod jsonc;
pub mod palette;
pub mod palette_usage;
pub mod saved_layout;
pub mod settings_doc;

pub mod sidebar;

pub mod blame;

pub mod label_colors;
