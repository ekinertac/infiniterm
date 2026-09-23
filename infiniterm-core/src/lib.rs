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

pub mod alignment;
pub mod app;
pub mod backend;
pub mod browser_keys;
pub mod card_label;
pub mod cli;
pub mod commands;
pub mod config;
pub mod config_files;
pub mod drop;
pub mod editor_keys;
pub mod editor_theme;
pub mod extensions;
pub mod files;
pub mod fuzzy;
pub mod git;
pub mod hooks;
pub mod ift;
pub mod inspect;
pub mod itermcolors;
pub mod jsonc;
pub mod keymap;
pub mod layout_file;
pub mod links;
pub mod links_fs;
pub mod model;
pub mod omni;
pub mod palette;
pub mod palette_usage;
pub mod paths;
pub mod prompt;
pub mod saved_layout;
pub mod settings_doc;
pub mod shell_history;
pub mod shortcuts;
pub mod slot_snap;
pub mod snippets;
pub mod switcher;
pub mod themes_files;
pub mod transcript;
pub mod transport;
pub mod update;

pub mod sidebar;

pub mod blame;

pub mod label_colors;
