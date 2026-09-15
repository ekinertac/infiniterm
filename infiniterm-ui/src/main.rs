//! infiniterm, native. The window, the one view that owns the model, and
//! the frame loop. Port of `App.svelte`'s wiring: startup, the backend's
//! streams drained each frame, the key handler, the template.
//!
//! One gpui entity, `AppView`, holds `Model` (every store and command of the
//! reference, in core), the command registry, the backend, the animator and
//! the card bodies. Each frame, in `paint.rs`: drain the backend, tick the
//! model, step the animator, run the effects, paint the world, schedule the
//! save. Input (`input.rs`) turns gpui events into model calls; the overlays
//! (`overlays.rs`) are plain div trees over the canvas. Nothing here decides
//! anything about cards; it draws what the model says and hands back what
//! the person did.
mod animator;
mod body;
mod chrome;
mod field;
mod input;
mod overlays;
mod paint;
mod runtime;
mod terminal_body;
mod terminals;
mod text;

use animator::Animator;
use body::CardBody;
use chrome::Chrome;
use gpui::{
    point, prelude::*, px, size, App, Application, Bounds, FocusHandle, TitlebarOptions,
    WindowBounds, WindowOptions,
};
use infiniterm_core::app::Backend;
use infiniterm_core::commands::CommandRegistry;
use infiniterm_core::grid::{Point, Rect};
use infiniterm_core::model::Model;
use infiniterm_core::momentum::Sample;
use infiniterm_core::resize::Edge;
use std::collections::HashMap;
use std::time::Instant;

pub const TITLEBAR_H: f32 = 44.;
pub const STATUSBAR_H: f32 = 22.;
/// Movement below this is a click with a shaky hand, not a drag.
pub const DRAG_SLOP: f64 = 4.;
/// The edge band of a card that moves or resizes it, in screen pixels.
pub const EDGE_HIT: f64 = 8.;
/// How long a swapped or split card takes to glide to its new rect.
pub const SWAP_MS: f64 = 200.;
pub const SAVE_DEBOUNCE_MS: f64 = 500.;

/// A Cmd+left press waiting to become a pan, or a pan in progress.
pub enum Pan {
    Pending(Point),
    Dragging(Point),
}

pub enum GestureKind {
    Move,
    Resize(Edge),
    /// Dragging a group by its name tab: every member moves together.
    MoveGroup(String),
}

pub struct Gesture {
    pub card: String,
    pub kind: GestureKind,
    pub start_px: Point,
    pub start_rect: Rect,
    pub start_rects: Vec<(String, Rect)>,
}

/// A card mid-glide after a swap or a split, from its old rect.
pub struct Glide {
    pub from: Rect,
    pub started: f64,
}

pub struct AppView {
    pub model: Model,
    pub registry: CommandRegistry<Model>,
    pub backend: Backend,
    pub animator: Animator,
    pub chrome: Chrome,
    pub bodies: HashMap<String, Box<dyn CardBody>>,
    pub focus: FocusHandle,
    pub pan: Option<Pan>,
    pub gesture: Option<Gesture>,
    /// The prompt's field, opened with the suggestion selected; the palette's query.
    pub prompt_field: field::Field,
    pub query_field: field::Field,
    /// The shortcuts panel's filter.
    pub shortcuts_field: field::Field,
    pub prompt_was_open: bool,
    pub samples: Vec<Sample>,
    pub mouse: Point,
    pub seeded: bool,
    pub save_due: Option<f64>,
    pub last_sweep: f64,
    pub glides: HashMap<String, Glide>,
    /// Rects as of the last frame, to notice a MarkSwap'd card moving.
    pub marked: HashMap<String, Rect>,
    pub frames: u32,
    pub fps_window: Instant,
    pub fps: f32,
    pub themes_dir: std::path::PathBuf,
    pub scale_factor: f32,
    pub scheduler: infiniterm_term::scheduler::OutputScheduler,
    pub ledger: infiniterm_term::credit::AckLedger,
    pub palette: infiniterm_term::palette::Palette,
}

pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.)
        .unwrap_or(0.)
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1600.), px(1000.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("infiniterm".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(12.), px(14.))),
                }),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| {
                    let mut app = AppView::new(cx.focus_handle(), window.scale_factor());
                    runtime::startup(&mut app);
                    app
                });
                window.focus(&view.read(cx).focus.clone());
                // The idle wake-up: every 16 ms, drain what the backend's
                // threads sent and draw a frame if anything needs one. A
                // receiver cannot be awaited on gpui's executor, so a short
                // timer is what stands in for it; it costs nothing when
                // nothing arrived.
                let poll = view.clone();
                cx.spawn(async move |cx: &mut gpui::AsyncApp| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(16))
                        .await;
                    let alive = poll.update(cx, |this, cx| {
                        this.drain_backend();
                        if this.needs_frame() {
                            cx.notify();
                        }
                    });
                    if alive.is_err() {
                        break;
                    }
                })
                .detach();
                view
            },
        )
        .expect("a window");
        cx.activate(true);
    });
}
