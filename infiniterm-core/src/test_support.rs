//! Shared fixture constructors for the ported geometry tests.
//! Test modules use these to preserve the reference coordinates without UI state.
//! No production code depends on this module.
use crate::{cards::PlacedCard, grid::Rect};
pub fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { x, y, w, h }
}
pub fn card(id: &str, x: f64, y: f64) -> PlacedCard {
    PlacedCard {
        id: id.into(),
        rect: r(x, y, 100., 100.),
        group_id: None,
    }
}

/// A private endpoint for a test that binds its own listener, in whatever
/// shape this platform's `transport` speaks: a file under the temp dir on
/// unix, a named pipe on Windows (see `paths::socket_path`). Tagged by pid
/// so two test binaries running at once cannot collide.
pub fn endpoint(tag: &str) -> std::path::PathBuf {
    let name = format!("infiniterm-test-{tag}-{}", std::process::id());
    #[cfg(windows)]
    {
        std::path::PathBuf::from(format!(r"\\.\pipe\{name}"))
    }
    #[cfg(unix)]
    {
        std::env::temp_dir().join(format!("{name}.sock"))
    }
}
