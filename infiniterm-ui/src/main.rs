#![allow(unexpected_cfgs)]
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
mod browser_body;
mod browsers;
mod chrome;
mod composing;
mod decoys;
mod diff_body;
mod editor_body;
mod editor_tabs;
mod editors;
mod field;
mod fullscreen;
mod icon_font;
mod ime;
mod input;
mod keycast;
mod keycode;
mod middle_drag;
mod omnibox;
mod overlays;
mod paint;
mod runtime;
mod switcher_view;
mod tab_strip;
mod terminal_body;
mod terminals;
mod text;
mod transcript_body;
mod updater;
mod window_state;

use animator::Animator;
use body::CardBody;
use chrome::Chrome;
use gpui::{
    actions, point, prelude::*, px, size, App, Application, Bounds, FocusHandle, KeyBinding, Menu,
    MenuItem, TitlebarOptions, WindowBounds, WindowOptions,
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

impl AppView {
    /// The title bar's height under the interface multiplier. The traffic
    /// lights stay where macOS put them; the bar grows around them.
    pub fn titlebar_h(&self) -> f32 {
        TITLEBAR_H * self.model.ui_scale as f32
    }

    pub fn statusbar_h(&self) -> f32 {
        STATUSBAR_H * self.model.ui_scale as f32
    }
}
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
    /// A single card's drag moves a GHOST, not the card: the outline where
    /// it would land, snapped to the grid, and the card goes there on the
    /// drop if the space is free (`Model::drop_card`). The card sliding
    /// under the pointer from the first pixel felt like a mistake being
    /// made rather than a choice being offered.
    pub ghost: Option<Rect>,
}

/// A card mid-glide after a swap or a split, from its old rect.
pub struct Glide {
    pub from: Rect,
    pub started: f64,
}

/// The omnibox's answers on their way back to the UI thread: the query id
/// they were asked for, and the completions.
pub type SuggestChannel = (
    std::sync::mpsc::Sender<(u64, Vec<String>)>,
    std::sync::mpsc::Receiver<(u64, Vec<String>)>,
);

pub struct AppView {
    pub model: Model,
    pub registry: CommandRegistry<Model>,
    pub backend: Backend,
    pub animator: Animator,
    pub chrome: Chrome,
    pub bodies: HashMap<String, Box<dyn CardBody>>,
    /// The decoy drawn over each masked card, by card id (`decoys.rs`).
    pub decoys: HashMap<String, crate::terminal_body::TerminalBody>,
    pub focus: FocusHandle,
    /// Text an input method is composing and has not committed: the `\u{b4}`
    /// after Option+E, a Pinyin candidate. Held so macOS knows a composition
    /// is in progress; see `ime.rs`.
    /// The updater, for an installed app only (`runtime::startup`).
    pub updater: Option<crate::updater::Updater>,
    /// This bundle's build number (`CFBundleVersion`, the commit count),
    /// shown in the status bar so a friend can say which build they run;
    /// `None` outside a bundle.
    pub build: Option<u64>,
    pub composing: Option<String>,
    /// `app.emoji` asked; the next frame, which has the window, opens it.
    pub show_character_palette: bool,
    pub pan: Option<Pan>,
    /// The left button went down on empty canvas and has not come up: a
    /// right click now fits everything (`pan_mode::chord_fits_all`).
    pub left_on_canvas: bool,
    /// The last frame was over its glyph budget (`paint_world`): cards drew
    /// bars, and content frames are rationed (`AppView::far`).
    pub crowded: bool,
    pub gesture: Option<Gesture>,
    /// The card whose body is following a drag (a text selection).
    pub body_drag: Option<String>,
    /// A press that moved the focus to a card: where it landed, so the
    /// release can tell a click from a drag and reveal the card on a click.
    pub reveal_on_release: Option<Point>,
    /// Where each card's label chip was painted this frame, in content
    /// pixels, in paint order. The label is a FRAME target for the mouse
    /// (drag to move, double-click to fit), not a body target: a
    /// double-click that reached the terminal selected a word instead.
    pub label_hits: Vec<(String, Rect)>,
    /// The card body the pointer is currently over, so a body that cares
    /// about hover (the browser) gets told when the pointer leaves it.
    pub hover_body: Option<String>,
    /// The card frame band the pointer is over, and which edge or corner
    /// (`None` is the move band): the cursor says what a press would do
    /// and `paint.rs` lights the band, because a frame you can drag looked
    /// exactly like one you cannot.
    pub hover_edge: Option<(String, Option<infiniterm_core::resize::Edge>)>,
    /// A slim right-click menu open over a browser card. The position is
    /// fixed at open time (content-area screen pixels), not re-derived from
    /// the card each frame: simplest thing that works for a menu open for a
    /// couple of clicks, at the cost of not tracking a pan mid-menu.
    pub context_menu: Option<crate::browsers::CardContextMenu>,
    /// The prompt's field, opened with the suggestion selected; the palette's query.
    pub prompt_field: field::Field,
    pub query_field: field::Field,
    /// The shortcuts panel's filter.
    pub shortcuts_field: field::Field,
    /// The omnibox's text, opened selected so typing replaces a prefilled
    /// address the way it does in a browser.
    pub omni_field: field::Field,
    /// The find bar's text.
    pub find_field: field::Field,
    /// Something that arrived mid-frame changed what the chrome says, so one
    /// more frame is owed. The element tree is built BEFORE `frame()` runs,
    /// so anything drained there (a find count, the omnibox's suggestions)
    /// is a frame behind and would sit unseen until the next keystroke.
    pub redraw: bool,
    /// The pressed-shortcut overlay (`keycast.rs`): on or off, and what
    /// is showing.
    pub keycast_on: bool,
    pub keycasts: Vec<crate::keycast::Keycast>,
    /// When the last frame was painted (ms), for the far-zoom refresh gate.
    pub last_paint_ms: f64,
    pub prompt_was_open: bool,
    /// Text an effect asked to put on the clipboard, written on the next
    /// frame: only a frame has an App to write through.
    pub clipboard_out: Option<String>,
    /// The sessions (tmux windows, daemon sessions) that were already
    /// running when this launch started. Asked ONCE: a card adopts one only
    /// if it was there before we were, and a session this launch created
    /// must never be adopted by another card.
    pub live_sessions: Vec<String>,
    pub samples: Vec<Sample>,
    pub mouse: Point,
    pub seeded: bool,
    pub save_due: Option<f64>,
    /// When the omnibox's history is due to be written, on the layout's
    /// debounce.
    pub history_due: Option<f64>,
    pub last_sweep: f64,
    pub glides: HashMap<String, Glide>,
    /// Rects as of the last frame, to notice a MarkSwap'd card moving.
    pub marked: HashMap<String, Rect>,
    /// Each body's size as last told to it, so `resized` fires on a change.
    pub body_sizes: HashMap<String, infiniterm_core::grid::Size>,
    pub frames: u32,
    pub fps_window: Instant,
    pub fps: f32,
    /// Under `INFINITERM_KEYLOG`: feed ms, paint ms, frames, last report.
    pub timing: (f64, f64, u32, f64),
    /// The window frame as last seen, and when a changed one is due to be
    /// written.
    pub window_seen: Option<window_state::WindowState>,
    pub window_save_due: Option<f64>,
    pub themes_dir: std::path::PathBuf,
    pub scale_factor: f32,
    /// CEF initialised in this process: browser cards can open surfaces.
    pub cef_running: bool,
    /// The system's reduced-motion switch, read at launch; it outranks
    /// `ui.animations`.
    pub reduce_motion: bool,
    pub scheduler: infiniterm_term::scheduler::OutputScheduler,
    pub ledger: infiniterm_term::credit::AckLedger,
    pub palette: infiniterm_term::palette::Palette,
    /// Answers from the suggest endpoint, sent by the thread that ran curl
    /// and drained with the backend's channels once a frame.
    pub suggestions: SuggestChannel,
}

pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.)
        .unwrap_or(0.)
}

actions!(
    infiniterm,
    [
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        Zoom,
        ToggleFullScreen,
        ShowCharacterPalette,
        CheckForUpdates
    ]
);

/// `NSWorkspace.accessibilityDisplayShouldReduceMotion`: the system's
/// reduced-motion switch, which outranks `ui.animations` in the reference.
fn system_reduces_motion() -> bool {
    use objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let ws: *mut objc::runtime::Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        let reduce: objc::runtime::BOOL = msg_send![ws, accessibilityDisplayShouldReduceMotion];
        reduce == objc::runtime::YES
    }
}

fn main() {
    // CEF first: a helper invocation runs and exits here; the browser
    // process loads the framework from the bundle and goes on. Outside a
    // bundle there is no framework and the app runs without browser cards.
    let mut cef = match infiniterm_browser::process::early() {
        Ok(p) => Some(p),
        Err(infiniterm_browser::process::Unavailable::Helper(code)) => std::process::exit(code),
        Err(infiniterm_browser::process::Unavailable::NoFramework) => {
            eprintln!("[infiniterm] CEF framework not beside the executable: no browser cards");
            None
        }
    };
    if runtime::another_instance_holds_the_socket() {
        eprintln!("[infiniterm] another infiniterm holds the socket; activating it");
        let _ = std::process::Command::new("open")
            .args(["-b", "dev.ekinertac.infiniterm"])
            .status();
        std::process::exit(0);
    }
    Application::new().run(move |cx: &mut App| {
        keycode::install();
        let cef_running = match cef.as_mut() {
            Some(p) => {
                infiniterm_browser::app_protocol::install();
                let ok = p.start();
                if !ok {
                    eprintln!("[infiniterm] CEF did not initialise: no browser cards");
                }
                ok
            }
            None => false,
        };
        if cef_running {
            // CEF's message pump, as the spikes ran it: one turn every 4 ms.
            cx.spawn(async move |cx: &mut gpui::AsyncApp| loop {
                infiniterm_browser::process::pump();
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(4))
                    .await;
            })
            .detach();
        }
        // The app menu, as the reference's menu.rs builds it by hand: no
        // File menu (its Close Window sat on Cmd+W, the card-close key),
        // and no Edit menu (a webview needed one for Cmd+C/V, gpui does
        // not). AppKit matches menu keys before the window sees them, so
        // Cmd+H, Cmd+Alt+H, Cmd+M and Cmd+Q stay unbindable in the app.
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        // The window items act on the one window there is.
        cx.on_action(|_: &Minimize, cx| {
            for w in cx.windows() {
                let _ = cx.update_window(w, |_, window, _| window.minimize_window());
            }
        });
        cx.on_action(|_: &Zoom, cx| {
            for w in cx.windows() {
                let _ = cx.update_window(w, |_, window, _| window.zoom_window());
            }
        });
        // The same message the green button sends, so `ui.fullscreen`
        // decides for both (fullscreen.rs).
        cx.on_action(|_: &ToggleFullScreen, _| fullscreen::toggle());
        // The emoji panel, from the menu. The chord is `app.emoji` in the
        // keymap, not a gpui binding here: macOS also handles
        // Cmd+Ctrl+Space system-wide, and with a gpui action bound to it
        // too the three fought and it worked sometimes. What the panel
        // inserts arrives through the input handler in ime.rs.
        cx.on_action(|_: &ShowCharacterPalette, cx| {
            for w in cx.windows() {
                let _ = cx.update_window(w, |_, window, _| window.show_character_palette());
            }
        });
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("cmd-alt-h", HideOthers, None),
            KeyBinding::new("cmd-m", Minimize, None),
        ]);
        cx.set_menus(vec![
            Menu {
                name: "infiniterm".into(),
                items: vec![
                    // Where a Mac user looks for it; handled on the view
                    // (overlays.rs), which runs `app.update.check`.
                    MenuItem::action("Check for Updates…", CheckForUpdates),
                    MenuItem::separator(),
                    MenuItem::action("Hide infiniterm", Hide),
                    MenuItem::action("Hide Others", HideOthers),
                    MenuItem::action("Show All", ShowAll),
                    MenuItem::separator(),
                    MenuItem::action("Quit infiniterm", Quit),
                ],
            },
            Menu {
                name: "Edit".into(),
                items: vec![MenuItem::action("Emoji & Symbols", ShowCharacterPalette)],
            },
            Menu {
                name: "Window".into(),
                items: vec![
                    MenuItem::action("Minimize", Minimize),
                    MenuItem::action("Zoom", Zoom),
                    MenuItem::action("Toggle Full Screen", ToggleFullScreen),
                ],
            },
        ]);
        // Where it was last time, else centred: the reference's window-state
        // plugin, owned here.
        // The icon font every card falls back to (`icon_font.rs`), bundled
        // so a Mac without a Nerd Font still draws the prompt's icons. With
        // CoreText, not gpui's `add_fonts`: gpui resolves a fallback by name
        // through CoreText, which cannot see gpui's own store.
        if !icon_font::register() {
            eprintln!("[infiniterm/warn] the bundled icon font is not resolvable; prompt icons may draw as boxes");
        }
        // A window that quit full screen opens at its old frame and goes
        // full screen once it exists, through the same toggle as the green
        // button, so `ui.fullscreen` picks the kind; left to gpui it would
        // always be a native Space.
        let mut reopen_full = false;
        let window_bounds = window_state::WindowState::load()
            .map(|s| match s.bounds() {
                WindowBounds::Fullscreen(b) => {
                    reopen_full = true;
                    WindowBounds::Windowed(b)
                }
                b => b,
            })
            .unwrap_or_else(|| {
                WindowBounds::Windowed(Bounds::centered(None, size(px(1600.), px(1000.)), cx))
            });
        cx.open_window(
            WindowOptions {
                window_bounds: Some(window_bounds),
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
                    app.cef_running = cef_running;
                    app.reduce_motion = system_reduces_motion();
                    runtime::startup(&mut app);
                    fullscreen::set_mode(app.model.config.ui.fullscreen);
                    app
                });
                window.focus(&view.read(cx).focus.clone());
                // gpui's view never answers middle-button drags; now that the
                // window (and so its view class) exists, teach it to.
                middle_drag::install();
                fullscreen::install();
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
                        // The save debounce runs here, not in the frame: a
                        // layout change with nothing else moving must not
                        // hold the frame loop open for half a second.
                        this.schedule_save(now_ms());
                        this.schedule_window_save(now_ms());
                        this.idle_editors(now_ms());
                        // The green button has no model to ask; an atomic
                        // store is cheaper than noticing a settings change.
                        fullscreen::set_mode(this.model.config.ui.fullscreen);
                        if this.needs_frame() {
                            cx.notify();
                        }
                    });
                    if alive.is_err() {
                        break;
                    }
                })
                .detach();
                // A quit within the save debounce would lose the last half
                // second of moves; the window's frame goes with it.
                let quitting = view.clone();
                cx.on_app_quit(move |cx| {
                    quitting.update(cx, |this, _| {
                        this.flush_save();
                        // The shells: local ones die with us, tmux windows
                        // are left running. That is the whole point of the
                        // tmux backend, and it has to happen HERE rather
                        // than by letting the process fall over, or the
                        // control client dies without detaching.
                        this.backend.pty.leave();
                        // The surfaces close before CEF shuts down.
                        this.bodies.clear();
                    });
                    if cef_running {
                        infiniterm_browser::process::stop();
                    }
                    async {}
                })
                .detach();
                view
            },
        )
        .expect("a window");
        cx.activate(true);
        if reopen_full {
            fullscreen::toggle();
        }
    });
}
