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

/// How long before a switch the next picture starts loading, so a big JPEG
/// is decoded before the fade needs it.
pub const PRELOAD_MS: f64 = 5_000.;

/// How often a late load is looked at again once a switch is due.
const LATE_POLL_MS: f64 = 100.;

/// What one frame draws: `current` over `previous` (when a fade is on) at
/// `alpha`, and which picture should be loading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shown {
    pub current: usize,
    pub previous: Option<usize>,
    /// Opacity of `current` over `previous`; 1 when nothing is fading.
    pub alpha: f32,
    /// The picture to load now, if a switch is near.
    pub preload: Option<usize>,
    /// When the next frame is needed while nothing animates (ms, the clock
    /// the caller passes in); `None` for a single picture.
    pub wake: Option<f64>,
}

impl Shown {
    pub fn fading(&self) -> bool {
        self.previous.is_some()
    }

    fn still(current: usize, preload: Option<usize>, wake: Option<f64>) -> Shown {
        Shown {
            current,
            previous: None,
            alpha: 1.,
            preload,
            wake,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Fade {
    from: usize,
    to: usize,
    start: f64,
}

/// The rotation's clock: which picture is up, since when, and the fade in
/// progress. Pure: the caller hands in `now` and whether a picture has
/// loaded, so the timing is tested without a window. A fade starts only
/// once the next picture is ready, so a slow decode delays a switch and
/// never makes the picture jump.
#[derive(Debug, Default)]
pub struct Slideshow {
    shown: usize,
    since: Option<f64>,
    fading: Option<Fade>,
}

impl Slideshow {
    /// `count` pictures that exist, `interval_s` each, `fade_s` between
    /// (0 for a cut). `ready(i)` says picture `i` can be drawn now; it is
    /// also where the caller starts loading it.
    pub fn step(
        &mut self,
        now: f64,
        count: usize,
        interval_s: f64,
        fade_s: f64,
        mut ready: impl FnMut(usize) -> bool,
    ) -> Shown {
        if self.shown >= count {
            *self = Slideshow::default();
        }
        if count < 2 {
            self.fading = None;
            return Shown::still(0, None, None);
        }
        let since = *self.since.get_or_insert(now);
        let (interval, fade) = (interval_s * 1000., fade_s * 1000.);
        if let Some(f) = self.fading {
            let t = (now - f.start) / fade;
            if t < 1. {
                let smooth = t * t * (3. - 2. * t);
                return Shown {
                    current: f.to,
                    previous: Some(f.from),
                    alpha: smooth as f32,
                    preload: None,
                    wake: None,
                };
            }
            self.shown = f.to;
            self.since = Some(f.start + fade);
            self.fading = None;
            return self.step(now, count, interval_s, fade_s, ready);
        }
        let next = (self.shown + 1) % count;
        let age = now - since;
        let preload = (age >= interval - PRELOAD_MS).then_some(next);
        // `ready` is asked from the preload window on: that call is what
        // starts the load, so it must not wait for the switch to be due.
        let loaded = preload.is_some() && ready(next);
        if age >= interval && loaded {
            if fade <= 0. {
                self.shown = next;
                self.since = Some(now);
                return Shown::still(next, None, Some(now + interval - PRELOAD_MS));
            }
            self.fading = Some(Fade {
                from: self.shown,
                to: next,
                start: now,
            });
            return Shown {
                current: next,
                previous: Some(self.shown),
                alpha: 0.,
                preload: None,
                wake: None,
            };
        }
        let wake = if age >= interval {
            now + LATE_POLL_MS
        } else if age >= interval - PRELOAD_MS {
            since + interval
        } else {
            since + interval - PRELOAD_MS
        };
        Shown::still(self.shown, preload, Some(wake))
    }
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
                // One folder per call: two tests with the same names ran in
                // parallel and one's drop removed the other's files.
                static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                let p = std::env::temp_dir().join(format!(
                    "ift-bg-{}-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
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

    const ALL_READY: fn(usize) -> bool = |_| true;

    #[test]
    fn one_picture_never_rotates() {
        let mut s = Slideshow::default();
        for t in [0., 1e6, 1e9] {
            let f = s.step(t, 1, 10., 2., ALL_READY);
            assert_eq!((f.current, f.previous, f.wake), (0, None, None));
        }
        assert_eq!(s.step(0., 0, 10., 2., ALL_READY).wake, None);
    }

    #[test]
    fn the_next_picture_fades_in_after_the_interval_and_then_stays() {
        let mut s = Slideshow::default();
        // 60 s each, 2 s fade; the clock starts at the first frame.
        assert_eq!(s.step(1000., 3, 60., 2., ALL_READY).current, 0);
        let f = s.step(1000. + 59_999., 3, 60., 2., ALL_READY);
        assert_eq!((f.current, f.previous, f.alpha), (0, None, 1.));
        let f = s.step(1000. + 60_000., 3, 60., 2., ALL_READY);
        assert_eq!((f.current, f.previous, f.alpha), (1, Some(0), 0.));
        let f = s.step(1000. + 61_000., 3, 60., 2., ALL_READY);
        assert_eq!((f.current, f.previous), (1, Some(0)));
        assert!((f.alpha - 0.5).abs() < 1e-6, "{}", f.alpha);
        let f = s.step(1000. + 62_000., 3, 60., 2., ALL_READY);
        assert_eq!((f.current, f.previous, f.alpha), (1, None, 1.));
        // the interval counts from the end of the fade
        let f = s.step(1000. + 62_000. + 60_000., 3, 60., 2., ALL_READY);
        assert_eq!((f.current, f.previous), (2, Some(1)));
    }

    #[test]
    fn the_list_wraps_to_the_first_picture() {
        let mut s = Slideshow::default();
        let mut now = 0.;
        let mut seen = vec![];
        s.step(now, 2, 60., 0., ALL_READY);
        for _ in 0..4 {
            now += 61_000.;
            seen.push(s.step(now, 2, 60., 0., ALL_READY).current);
        }
        assert_eq!(seen, [1, 0, 1, 0]);
    }

    #[test]
    fn a_zero_fade_cuts_at_once() {
        let mut s = Slideshow::default();
        s.step(0., 2, 10., 0., ALL_READY);
        let f = s.step(10_000., 2, 10., 0., ALL_READY);
        assert_eq!((f.current, f.previous, f.alpha), (1, None, 1.));
    }

    #[test]
    fn a_late_load_delays_the_fade_and_never_skips_it() {
        let mut s = Slideshow::default();
        s.step(0., 2, 10., 2., ALL_READY);
        let f = s.step(12_000., 2, 10., 2., |_| false);
        assert_eq!((f.current, f.previous), (0, None));
        assert_eq!(f.wake, Some(12_100.));
        // loaded at 13 s: the fade starts then, from zero
        let f = s.step(13_000., 2, 10., 2., ALL_READY);
        assert_eq!((f.current, f.previous, f.alpha), (1, Some(0), 0.));
    }

    #[test]
    fn the_next_picture_starts_loading_five_seconds_before_the_switch() {
        let mut s = Slideshow::default();
        let mut asked = vec![];
        for t in [0., 4_999., 5_000., 9_000.] {
            s.step(t, 3, 10., 2., |i| {
                asked.push((t, i));
                false
            });
        }
        assert_eq!(asked, [(5_000., 1), (9_000., 1)]);
        // a quiet stretch asks for a frame at the preload, then at the switch
        let mut s = Slideshow::default();
        assert_eq!(s.step(0., 3, 10., 2., ALL_READY).wake, Some(5_000.));
        assert_eq!(s.step(5_000., 3, 10., 2., |_| false).wake, Some(10_000.));
    }

    #[test]
    fn a_list_that_shrinks_starts_over() {
        let mut s = Slideshow::default();
        s.step(0., 3, 10., 0., ALL_READY);
        s.step(10_000., 3, 10., 0., ALL_READY);
        s.step(20_000., 3, 10., 0., ALL_READY);
        assert_eq!(s.step(21_000., 2, 10., 0., ALL_READY).current, 0);
    }
}
