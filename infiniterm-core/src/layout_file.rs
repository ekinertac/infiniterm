//! The saved canvas on disk: `workspace.json` under Application Support.
//! From the Tauri app's layout.rs. The schema, its version and every
//! validation rule are `saved_layout.rs`; this file moves bytes.
//!
//! Named layout_file because `layout.rs` is where a new card goes; the
//! reference once lost a file to that confusion.
use crate::paths::layout_path;

/// The saved layout, or `None` on a first run. An unreadable file reads as
/// `None` rather than an error: a layout that cannot be parsed must start
/// the app empty, never block it from starting.
pub fn read_layout() -> Option<String> {
    std::fs::read_to_string(layout_path()).ok()
}

pub fn write_layout(contents: &str) -> Result<(), String> {
    // Through a link, not over it (#219): the canvas file may live in a dotfiles repo.
    let path = crate::files::resolve_link(&layout_path());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Write-then-rename, so a quit mid-write cannot leave a truncated file
    // that fails to parse and loses the whole canvas. Written far more often
    // than the config, so the window for that is much wider.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}
