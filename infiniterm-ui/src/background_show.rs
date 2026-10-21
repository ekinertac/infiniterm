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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
}

fn source(path: &Path) -> Resource {
    Resource::Path(path.into())
}

impl Show {
    /// A frame is due: a fade is running, or the clock reached the next
    /// look at the rotation. A still picture never asks.
    pub fn needs_frame(&self, now: f64) -> bool {
        self.animating || self.wake.is_some_and(|w| now >= w)
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
        bundled: Option<&Path>,
        animate: bool,
        now: f64,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        if self.entries != ui.background_image {
            self.entries = ui.background_image.clone();
            let home = infiniterm_core::paths::home_dir();
            self.paths = self
                .entries
                .iter()
                .filter_map(|e| resolve(e, bundled, &home))
                .collect();
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
