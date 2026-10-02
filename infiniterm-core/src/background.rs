//! The canvas background image (`ui.backgroundImage`, #97): which file the
//! setting names and where the picture lands in the window. Pure, so both
//! are tested; the ui crate only loads and paints.
//!
//! Called by: `infiniterm-ui/src/paint.rs` (`paint_background`) and
//! `runtime.rs` (the bundled folder). Related: `config.rs` (the two settings),
//! `tools/bundle.sh` (copies `assets/backgrounds` into the app).
//!
//! A value with no slash and no extension is the NAME of a bundled picture
//! ("dusk"); anything else is a path, `~` expanded. A name that is not
//! bundled, a missing folder or a path that does not exist all give `None`:
//! no image, never an error on every frame.

use std::path::{Path, PathBuf};

use crate::config::BackgroundFit;

/// Bundled pictures are JPEGs, so a name never needs its extension.
const BUNDLED_EXT: &str = "jpg";

/// The file `setting` names, or `None` when there is none to draw.
pub fn resolve(setting: &str, bundled_dir: Option<&Path>, home: &Path) -> Option<PathBuf> {
    let s = setting.trim();
    if s.is_empty() {
        return None;
    }
    let path = if let Some(rest) = s.strip_prefix("~/") {
        home.join(rest)
    } else if !s.contains('/') && Path::new(s).extension().is_none() {
        bundled_dir?.join(format!("{s}.{BUNDLED_EXT}"))
    } else {
        PathBuf::from(s)
    };
    path.is_file().then_some(path)
}

/// Where a picture of `natural` pixels lands in an `area`, as
/// `(x, y, w, h)` relative to the area's corner, centred. Cover fills the
/// area and lets the overflow be clipped; contain shows all of it.
pub fn place(
    natural: (f64, f64),
    area: (f64, f64),
    fit: BackgroundFit,
) -> Option<(f64, f64, f64, f64)> {
    let (nw, nh) = natural;
    let (aw, ah) = area;
    if nw <= 0. || nh <= 0. || aw <= 0. || ah <= 0. {
        return None;
    }
    let (sx, sy) = (aw / nw, ah / nh);
    let scale = match fit {
        BackgroundFit::Cover => sx.max(sy),
        BackgroundFit::Contain => sx.min(sy),
    };
    let (w, h) = (nw * scale, nh * scale);
    Some(((aw - w) / 2., (ah - h) / 2., w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with(names: &[&str]) -> tempfile_dir::Dir {
        tempfile_dir::Dir::with(names)
    }

    // A throwaway folder without a dev-dependency: the std temp dir, one
    // per test name.
    mod tempfile_dir {
        use std::path::{Path, PathBuf};
        pub struct Dir(pub PathBuf);
        impl Dir {
            pub fn with(names: &[&str]) -> Dir {
                let p = std::env::temp_dir().join(format!(
                    "ift-bg-{}-{}",
                    std::process::id(),
                    names.join("-")
                ));
                std::fs::create_dir_all(&p).unwrap();
                for n in names {
                    std::fs::write(p.join(n), b"x").unwrap();
                }
                Dir(p)
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn empty_means_no_image() {
        let d = dir_with(&["dusk.jpg"]);
        assert_eq!(resolve("", Some(d.path()), d.path()), None);
        assert_eq!(resolve("  ", Some(d.path()), d.path()), None);
    }

    #[test]
    fn a_bare_name_is_a_bundled_picture() {
        let d = dir_with(&["dusk.jpg"]);
        assert_eq!(
            resolve("dusk", Some(d.path()), Path::new("/nowhere")),
            Some(d.path().join("dusk.jpg"))
        );
        assert_eq!(resolve("nope", Some(d.path()), Path::new("/nowhere")), None);
        assert_eq!(resolve("dusk", None, Path::new("/nowhere")), None);
    }

    #[test]
    fn a_path_is_used_as_written_and_tilde_is_the_home_folder() {
        let d = dir_with(&["mine.png"]);
        let abs = d.path().join("mine.png");
        assert_eq!(
            resolve(abs.to_str().unwrap(), None, Path::new("/nowhere")),
            Some(abs.clone())
        );
        assert_eq!(resolve("~/mine.png", None, d.path()), Some(abs));
        assert_eq!(resolve("/no/such/file.png", None, d.path()), None);
    }

    #[test]
    fn cover_fills_the_area_and_contain_shows_the_whole_picture() {
        // A 200x100 picture in a 100x100 area.
        let (x, y, w, h) = place((200., 100.), (100., 100.), BackgroundFit::Cover).unwrap();
        assert_eq!((w, h), (100. * 2., 100.));
        assert_eq!((x, y), (-50., 0.), "centred, the sides overflow");
        let (x, y, w, h) = place((200., 100.), (100., 100.), BackgroundFit::Contain).unwrap();
        assert_eq!((w, h), (100., 50.));
        assert_eq!((x, y), (0., 25.), "centred, bars above and below");
    }

    #[test]
    fn an_empty_picture_or_area_places_nothing() {
        assert_eq!(place((0., 10.), (10., 10.), BackgroundFit::Cover), None);
        assert_eq!(place((10., 10.), (10., 0.), BackgroundFit::Contain), None);
    }
}
