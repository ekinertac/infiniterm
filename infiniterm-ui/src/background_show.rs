//! The canvas background as the ui draws it: the `ui.backgroundImage` list,
//! one picture or a rotation that crossfades (#97, #210).
//!
//! Called by `overlays.rs` (`render`: `update` once a frame, then `layers`
//! under the canvas element) and `runtime.rs` (`needs_frame`). The clock and
//! the decisions are core's `Slideshow` (`infiniterm-core/src/background.rs`,
//! tested); this file loads, builds the layers and frees.
//!
//! Why elements and not paint calls: gpui's `paint_image` has no alpha, and
//! only an element's opacity reaches it, which cannot be set from inside a
//! paint callback (building a div there panics: layout is a prepaint-phase
//! call). So the pictures are divs under the canvas element, each holding a
//! canvas that paints one picture, and the canvas stops filling the window
//! itself while a list is set (`active`): the fill is the first layer, the
//! picture the next, the fading one over it. A frame re-renders the view
//! (`request_animation_frame` notifies it), which is what steps the clock.
//!
//! Memory is the constraint: gpui keeps every decoded picture for good, and a
//! 4K one is about 33 MB plus the same again in the GPU's atlas. Only the
//! picture on screen, the one fading out and the one loading for the next
//! switch are kept (`evict`), so a list of twenty costs what a list of two
//! does. A picture that will not load draws nothing and never holds the
//! rotation up.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use infiniterm_core::file_source::FileSource;

use gpui::{
    canvas, div, point, px, size, AnyElement, Bounds, ContentMask, Corners, Hsla, ImageAssetLoader,
    IntoElement, ParentElement, RenderImage, Resource, Styled, Window,
};
use infiniterm_core::background::{place, resolve, Shown, Slideshow};
use infiniterm_core::config::{BackgroundFit, Ui};

#[derive(Default)]
pub struct Show {
    /// The setting the paths below were resolved from.
    entries: Vec<String>,
    paths: Vec<PathBuf>,
    slideshow: Slideshow,
    /// Pictures decoded now, so they can be freed and drawn.
    resident: HashMap<PathBuf, Arc<RenderImage>>,
    shown: Option<Shown>,
    /// A fade is running: a frame every tick.
    animating: bool,
    /// When a frame is next needed while nothing animates (ms).
    wake: Option<f64>,
    /// A joined window's pictures, which are files on the host (#560): the
    /// entries asked for, the cached copy of each that arrived, and the
    /// threads' answers. See `host_pictures.rs`.
    host_asked: HashSet<String>,
    host_ready: HashMap<String, PathBuf>,
    /// Entries whose fetch failed: drawn as nothing, never waited for.
    host_lost: HashSet<String>,
    host_answers: Option<(Sender<HostPicture>, Receiver<HostPicture>)>,
}

/// Where the list's pictures come from: the app's bundled folder, and in a
/// joined window the host's file source for the rest.
#[derive(Clone, Copy)]
pub struct Pictures<'a> {
    pub bundled: Option<&'a Path>,
    pub host: Option<&'a Arc<dyn FileSource>>,
}

/// One entry's cached copy, or why it could not be had.
type HostPicture = (String, Result<PathBuf, String>);

fn source(path: &Path) -> Resource {
    Resource::Path(path.into())
}

impl Show {
    /// A frame is due: a fade is running, or the clock reached the next
    /// look at the rotation. A still picture never asks.
    pub fn needs_frame(&self, now: f64) -> bool {
        // A host's picture still on its way: a frame takes the answer.
        let waiting = self.host_asked.len() > self.host_ready.len() + self.host_lost.len();
        self.animating || waiting || self.wake.is_some_and(|w| now >= w)
    }

    /// There is a picture to show, so the canvas leaves its fill to the
    /// layers.
    pub fn active(&self) -> bool {
        !self.paths.is_empty()
    }

    /// Steps the rotation, starts the loads it asks for and frees what the
    /// next frames will not draw.
    pub fn update(
        &mut self,
        ui: &Ui,
        from: Pictures,
        animate: bool,
        now: f64,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        let Pictures { bundled, host } = from;
        let mut rebuild = false;
        if self.entries != ui.background_image {
            self.entries = ui.background_image.clone();
            self.host_asked.clear();
            self.host_ready.clear();
            self.host_lost.clear();
            rebuild = true;
        }
        // In a joined window the list is the host's settings: a bundled name
        // is this app's own, any other entry is a file on the host, fetched
        // into the cache and drawn from there.
        if let Some(source) = host {
            self.ask_host(source, &infiniterm_core::host_pictures::cache_dir());
            rebuild |= self.take_host_answers();
        }
        if rebuild {
            self.paths = self.resolve_all(bundled, host.is_some());
            self.slideshow = Slideshow::default();
        }
        let fade = if animate {
            ui.background_image_fade
        } else {
            0.
        };
        let paths = &self.paths;
        let resident = &mut self.resident;
        let shown =
            self.slideshow
                .step(now, paths.len(), ui.background_image_interval, fade, |i| {
                    // A picture that fails to load counts as ready: it draws
                    // nothing, and the rotation moves on.
                    let loading = window.use_asset::<ImageAssetLoader>(&source(&paths[i]), cx);
                    if let Some(Ok(image)) = &loading {
                        resident.insert(paths[i].clone(), image.clone());
                    }
                    loading.is_some()
                });
        // The picture on show may still be loading the first time.
        if let Some(Ok(image)) = self.paths.get(shown.current).and_then(|p| {
            window
                .use_asset::<ImageAssetLoader>(&source(p), cx)
                .map(|r| r.map(|i| (p.clone(), i)))
        }) {
            self.resident.insert(image.0, image.1);
        }
        self.animating = shown.fading();
        self.wake = shown.wake;
        self.evict(&shown, window, cx);
        self.shown = Some(shown);
    }

    /// The files the entries name, in order: the host's from their cached
    /// copies (when they have arrived) in a viewer, the rest as on any Mac.
    fn resolve_all(&self, bundled: Option<&Path>, viewer: bool) -> Vec<PathBuf> {
        let home = infiniterm_core::paths::home_dir();
        self.entries
            .iter()
            .filter_map(|e| {
                if viewer && infiniterm_core::host_pictures::is_host_entry(e) {
                    self.host_ready.get(e).cloned()
                } else {
                    resolve(e, bundled, &home)
                }
            })
            .collect()
    }

    /// Starts the fetch of each host entry not asked for yet, one thread
    /// each: the host's `~` and a file's bytes both take a round trip.
    fn ask_host(&mut self, source: &Arc<dyn FileSource>, cache: &Path) {
        let (tx, _) = self.host_answers.get_or_insert_with(channel);
        for entry in &self.entries {
            if !infiniterm_core::host_pictures::is_host_entry(entry)
                || !self.host_asked.insert(entry.clone())
            {
                continue;
            }
            let (source, tx, entry, cache) = (
                source.clone(),
                tx.clone(),
                entry.clone(),
                cache.to_path_buf(),
            );
            std::thread::spawn(move || {
                let result = infiniterm_core::host_pictures::host_path(&*source, &entry)
                    .and_then(|p| infiniterm_core::host_pictures::fetch(&*source, &cache, &p))
                    .map(|(cached, _)| cached)
                    .map_err(|e| e.to_string());
                let _ = tx.send((entry, result));
            });
        }
    }

    /// Takes the fetches that finished; true when a picture is new.
    fn take_host_answers(&mut self) -> bool {
        let Some((_, rx)) = &self.host_answers else {
            return false;
        };
        let mut arrived = false;
        while let Ok((entry, result)) = rx.try_recv() {
            // An answer for an entry the list no longer has is dropped.
            if !self.entries.contains(&entry) {
                continue;
            }
            match result {
                Ok(path) => {
                    self.host_ready.insert(entry, path);
                    arrived = true;
                }
                Err(why) => {
                    eprintln!("[infiniterm/warn] background {entry} from the host: {why}");
                    self.host_lost.insert(entry);
                }
            }
        }
        arrived
    }

    /// The layers under the canvas, bottom first: the window's fill, the
    /// picture fading out, the picture fading in. Empty without a list.
    pub fn layers(&self, fit: BackgroundFit, fill: Hsla) -> Vec<AnyElement> {
        let (Some(shown), true) = (&self.shown, self.active()) else {
            return Vec::new();
        };
        let layer = || div().absolute().top_0().left_0().size_full();
        let mut out = vec![layer().bg(fill).into_any_element()];
        if let Some(prev) = shown.previous {
            out.extend(self.picture(prev, 1., fit).map(|e| e.into_any_element()));
        }
        out.extend(
            self.picture(shown.current, shown.alpha, fit)
                .map(|e| e.into_any_element()),
        );
        out
    }

    fn picture(&self, index: usize, alpha: f32, fit: BackgroundFit) -> Option<impl IntoElement> {
        let image = self.resident.get(self.paths.get(index)?)?.clone();
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .opacity(alpha)
                .child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _| paint_picture(&image, fit, bounds, window),
                    )
                    .size_full(),
                ),
        )
    }

    /// Frees every decoded picture the next frames do not need.
    fn evict(&mut self, shown: &Shown, window: &mut Window, cx: &mut gpui::App) {
        let keep: Vec<&PathBuf> = [Some(shown.current), shown.previous, shown.preload]
            .into_iter()
            .flatten()
            .filter_map(|i| self.paths.get(i))
            .collect();
        let gone: Vec<PathBuf> = self
            .resident
            .keys()
            .filter(|p| !keep.contains(p))
            .cloned()
            .collect();
        for path in gone {
            cx.remove_asset::<ImageAssetLoader>(&source(&path));
            if let Some(image) = self.resident.remove(&path) {
                let _ = window.drop_image(image);
            }
        }
    }
}

/// One picture, placed in `bounds` and clipped to it.
fn paint_picture(
    image: &Arc<RenderImage>,
    fit: BackgroundFit,
    bounds: Bounds<gpui::Pixels>,
    window: &mut Window,
) {
    let natural = image.size(0);
    let scale = window.scale_factor() as f64;
    let area = (
        f32::from(bounds.size.width) as f64,
        f32::from(bounds.size.height) as f64,
    );
    // The picture's own pixels are device pixels; the area is logical.
    let natural = (
        natural.width.0 as f64 / scale,
        natural.height.0 as f64 / scale,
    );
    let Some((x, y, w, h)) = place(natural, area, fit) else {
        return;
    };
    let rect = Bounds::new(
        point(
            bounds.origin.x + px(x as f32),
            bounds.origin.y + px(y as f32),
        ),
        size(px(w as f32), px(h as f32)),
    );
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        let _ = window.paint_image(rect, Corners::default(), image.clone(), 0, false);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use infiniterm_core::file_source::{
        EntryKind, FileData, FileError, HostPaths, ListEntry, LocalSource, Stat,
    };

    /// This disk dressed as another machine's: not local, with its own home.
    struct Host {
        home: String,
    }

    impl FileSource for Host {
        fn is_local(&self) -> bool {
            false
        }
        fn home(&self) -> Result<HostPaths, FileError> {
            Ok(HostPaths {
                home: self.home.clone(),
                config_dir: format!("{}/.config", self.home),
            })
        }
        fn stat(&self, path: &str) -> Result<Stat, FileError> {
            LocalSource.stat(path)
        }
        fn exists(&self, paths: &[String]) -> Result<Vec<Option<EntryKind>>, FileError> {
            LocalSource.exists(paths)
        }
        fn list(&self, path: &str) -> Result<Vec<ListEntry>, FileError> {
            LocalSource.list(path)
        }
        fn read(&self, path: &str) -> Result<FileData, FileError> {
            LocalSource.read(path)
        }
        fn write(&self, p: &str, b: &[u8], e: Option<u64>) -> Result<Option<u64>, FileError> {
            LocalSource.write(p, b, e)
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // In a joined window the list's files are the host's: a bundled name is
    // this app's, the rest are fetched to the cache, and the picture list
    // gains each as it arrives; one that cannot be had is not waited for.
    #[test]
    fn a_viewers_background_list_fetches_the_hosts_files_and_keeps_bundled_names() {
        let host_home = scratch("home");
        let cache = scratch("cache");
        let bundled = scratch("bundled");
        std::fs::write(bundled.join("aurora.jpg"), b"bundled").unwrap();
        std::fs::create_dir_all(host_home.join("Pictures")).unwrap();
        std::fs::write(host_home.join("Pictures/sky.jpg"), b"sky").unwrap();
        let abs = host_home.join("Pictures/sea.jpg");
        std::fs::write(&abs, b"sea").unwrap();

        let source: Arc<dyn FileSource> = Arc::new(Host {
            home: host_home.to_string_lossy().into_owned(),
        });
        let mut show = Show {
            entries: vec![
                "aurora".into(),
                "~/Pictures/sky.jpg".into(),
                abs.to_string_lossy().into_owned(),
                "~/Pictures/missing.jpg".into(),
            ],
            ..Default::default()
        };
        show.ask_host(&source, &cache);
        assert_eq!(show.host_asked.len(), 3, "the bundled name is not asked");
        assert!(show.needs_frame(0.), "pictures are on their way");

        let stop = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while show.needs_frame(0.) {
            assert!(std::time::Instant::now() < stop, "timed out");
            show.take_host_answers();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let paths = show.resolve_all(Some(&bundled), true);
        let bytes: Vec<_> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
        assert_eq!(
            bytes,
            vec![b"bundled".to_vec(), b"sky".to_vec(), b"sea".to_vec()],
            "in the list's order; the missing one is left out"
        );
        assert!(paths[1].starts_with(&cache) && paths[2].starts_with(&cache));
        assert_eq!(show.host_lost.len(), 1);
        assert!(!show.needs_frame(0.));

        // Not in a joined window: every entry is resolved on this disk.
        let local = show.resolve_all(Some(&bundled), false);
        assert_eq!(local.len(), 2, "aurora and the absolute path exist here");
        for d in [&host_home, &cache, &bundled] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    // An answer for an entry the list no longer has is not taken.
    #[test]
    fn an_answer_for_an_entry_that_left_the_list_is_dropped() {
        let (tx, rx) = channel();
        let mut show = Show {
            entries: vec!["~/a.jpg".into()],
            host_answers: Some((tx.clone(), rx)),
            ..Default::default()
        };
        tx.send(("~/gone.jpg".into(), Ok(PathBuf::from("/x"))))
            .unwrap();
        assert!(!show.take_host_answers());
        assert!(show.host_ready.is_empty());
    }
}
